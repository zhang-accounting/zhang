use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use async_recursion::async_recursion;
use beancount::Beancount;
use chrono::Datelike;
use futures::TryStreamExt;
use log::{debug, info, warn};
use minijinja::{context, Environment};
use opendal::services::{Fs, Github, Webdav, S3};
use opendal::{EntryMode, ErrorKind, HttpTransporter, Operator};
use opendal_http_transport_reqwest::ReqwestTransport;
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};
use zhang_core::data_source::{DataSource, LoadResult, SourceEntry};
use zhang_core::data_type::text::parser::parse as zhang_parse;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{is_beancount_endpoint, DataType};
use zhang_core::ledger::Ledger;
use zhang_core::utils::has_path_visited;
use zhang_core::{utils, ZhangError, ZhangResult};

use crate::{FileSystem, ServerOpts};

/// how long a plugin's read or listing of a remote ledger may take before the host gives up on it. The plugin
/// waits for the host meanwhile, and its own timeout cannot interrupt the host, so a stalled server would
/// otherwise stall the load
const PLUGIN_FILE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OpendalDataSource {
    operator: Operator,
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
    is_beancount: bool,
    /// the directory the `Fs` service reads, the ledger root on the local disk; `None` for a remote service
    local_root: Option<PathBuf>,
}

async fn is_wildcard_pathbuf(pathbuf: &Path) -> bool {
    pathbuf.to_string_lossy().to_string().contains("*")
}

#[derive(Debug)]
struct WildcardPathComponent {
    path: String,
    remaining: Vec<String>,
}

#[async_trait::async_trait]
impl DataSource for OpendalDataSource {
    fn export(&self, directive: Directive) -> ZhangResult<Vec<u8>> {
        Ok(self.data_type.export(Spanned::new(directive, SpanInfo::default())).into_bytes())
    }

    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        let file = &path;
        self.blocking(None, |operator| async move { operator.read(file).await })
            .map(|data| data.to_vec())
            .map_err(|e| ZhangError::CustomError(format!("fail to get file content [{}] : {}", path, e)))
    }

    fn local_root(&self, _entry: &Path) -> Option<PathBuf> {
        self.local_root.clone()
    }

    /// checks the size before downloading, never reads more than one chunk past `max_len`, and gives up after
    /// [`PLUGIN_FILE_TIMEOUT`]
    fn get_limited(&self, path: String, max_len: u64) -> ZhangResult<Vec<u8>> {
        let file = &path;
        let content = self
            .blocking(Some(PLUGIN_FILE_TIMEOUT), |operator| async move {
                if operator.stat(file).await?.content_length() > max_len {
                    return Ok(None);
                }
                // streamed as a second guard, in case the size was wrong or the file grew since
                let mut chunks = operator.reader(file).await?.into_stream(..).await?;
                let mut content = Vec::new();
                while let Some(chunk) = chunks.try_next().await? {
                    if (content.len() + chunk.len()) as u64 > max_len {
                        return Ok(None);
                    }
                    content.extend_from_slice(&chunk.to_bytes());
                }
                Ok(Some(content))
            })
            .map_err(|e| e.into_zhang_error(&path))?;
        content.ok_or_else(|| ZhangError::TooLarge(format!("the file {path:?} holds more than {max_len} bytes")))
    }

    /// stops listing past `max_entries`, and gives up after [`PLUGIN_FILE_TIMEOUT`]
    fn list(&self, path: String, max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
        // a directory path ends with `/` in opendal, and the root is `/`
        let dir = format!("{}/", path.trim_end_matches('/'));
        let listed = &dir;
        let entries = self
            .blocking(Some(PLUGIN_FILE_TIMEOUT), |operator| async move {
                let mut lister = operator.lister(listed).await?;
                let mut entries = vec![];
                let mut seen = 0;
                while let Some(entry) = lister.try_next().await? {
                    // the listing also holds the listed directory itself
                    if entry.path() == listed {
                        continue;
                    }
                    seen += 1;
                    if seen > max_entries {
                        return Ok(None);
                    }
                    let is_dir = match entry.metadata().mode() {
                        EntryMode::DIR => true,
                        EntryMode::FILE => false,
                        EntryMode::Unknown => continue,
                    };
                    let name = entry.name().trim_end_matches('/');
                    if !name.is_empty() {
                        entries.push(SourceEntry { name: name.to_owned(), is_dir });
                    }
                }
                Ok(Some(entries))
            })
            .map_err(|e| e.into_zhang_error(&path))?;
        entries.ok_or_else(|| ZhangError::TooLarge(format!("the directory {path:?} has more than {max_entries} entries")))
    }

    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let entry = PathBuf::from(entry);
        let main_endpoint = entry.join(endpoint);

        let mut load_queue: VecDeque<PathBuf> = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited: Vec<PathBuf> = Vec::new();
        let mut directives = vec![];
        while let Some(pathbuf) = load_queue.pop_front() {
            let striped_pathbuf = &pathbuf.strip_prefix(&entry).expect("Cannot strip entry").to_path_buf();
            if is_wildcard_pathbuf(striped_pathbuf).await {
                // Split path into components and find wildcard level
                let mut path_components: Vec<String> = striped_pathbuf.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                let first_component = path_components.remove(0);
                let wildcard_component = WildcardPathComponent {
                    path: first_component,
                    remaining: path_components,
                };
                let mut queue: VecDeque<WildcardPathComponent> = VecDeque::new();
                queue.push_back(wildcard_component);

                let mut final_file_paths: Vec<PathBuf> = vec![];
                while let Some(mut current_component) = queue.pop_front() {
                    let mut current_path = PathBuf::new();
                    current_path.push(current_component.path);

                    let next_component = current_component.remaining.remove(0);

                    let next_component_path = current_path.join(&next_component);
                    if !next_component.contains('*') {
                        // if the next component is not a wildcard, we can just add it to the current path
                        queue.push_back(WildcardPathComponent {
                            path: next_component_path.to_string_lossy().to_string(),
                            remaining: current_component.remaining,
                        });
                        continue;
                    }
                    // if the next component is a wildcard, we need to add all the files in the current path to the final file paths

                    let current_path_str = format!("{}/", current_path.to_string_lossy());
                    let files = self
                        .operator
                        .list(&current_path_str)
                        .await
                        .map_err(|e| ZhangError::CustomError(format!("fail to list files in parent directory [{}] : {}", current_path.display(), e)))?;

                    let re = regex::Regex::new(&next_component.replace('*', "[^/]+")).unwrap();

                    for entry in files {
                        let entry_name = entry.path();
                        // `list` also returns the listed directory itself, which is not a child to match
                        if entry_name == current_path_str {
                            continue;
                        }

                        if entry.metadata().is_dir() {
                            let striped_entry_name = entry.path().strip_prefix(&current_path_str).unwrap().strip_suffix("/").unwrap();
                            if re.is_match(striped_entry_name) {
                                // Build full path
                                if !current_component.remaining.is_empty() {
                                    queue.push_back(WildcardPathComponent {
                                        path: current_path.join(striped_entry_name).to_string_lossy().to_string(),
                                        remaining: current_component.remaining.clone(),
                                    });
                                }
                            }
                        } else {
                            let striped_entry_name = entry_name.strip_prefix(&current_path_str).unwrap();
                            if re.is_match(striped_entry_name) {
                                // Build full path
                                let is_remaining_empty = current_component.remaining.is_empty();
                                if is_remaining_empty {
                                    final_file_paths.push(current_path.join(striped_entry_name));
                                }
                            }
                        }
                    }
                }
                for file_path in final_file_paths {
                    let fullpath = if file_path.as_path().starts_with("/") {
                        file_path
                    } else {
                        entry.join(file_path)
                    };
                    load_queue.push_back(fullpath);
                }
                continue;
            } else {
                debug!("visited entry file: {:?}", striped_pathbuf.display());
            }
            if utils::has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get_file_content(striped_pathbuf.clone()).await?;
            let entity_directives = self.parse(&file_content, striped_pathbuf.clone())?;

            entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                let fullpath = if buf.starts_with('/') {
                    PathBuf::from_str(&buf).unwrap()
                } else {
                    pathbuf.parent().map(|it| it.join(buf)).unwrap()
                };
                load_queue.push_back(fullpath);
            });
            directives.extend(entity_directives);
            visited.push(pathbuf);
        }
        let res = LoadResult {
            directives: self.transform(directives)?,
            visited_files: visited,
        };
        Ok(res)
    }

    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        let path_for_read = path.to_owned();
        let result = self.operator.read(&path_for_read).await;
        match result {
            Ok(data) => Ok(data.to_vec()),
            Err(err) => {
                if err.kind() == ErrorKind::NotFound {
                    Ok(Vec::new())
                } else {
                    Err(ZhangError::CustomError(format!("Error getting file content from {}: {}", path, err)))
                }
            }
        }
    }

    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for directive in directives {
            self.append_directive(ledger, directive, None, true).await?;
        }
        Ok(())
    }

    async fn async_save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        info!("[opendal] save content path={}", path);
        let vec = content.to_vec();

        self.operator.write(&path, vec).await.expect("cannot write");
        Ok(())
    }
}

/// why [`OpendalDataSource::blocking`] got no result
#[derive(Debug)]
enum BlockingError {
    Opendal(opendal::Error),
    TimedOut(Duration),
    Runtime(String),
}

impl Display for BlockingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockingError::Opendal(e) => write!(f, "{e}"),
            BlockingError::TimedOut(limit) => write!(f, "timed out after {limit:?}"),
            BlockingError::Runtime(e) => write!(f, "cannot start a runtime: {e}"),
        }
    }
}

impl BlockingError {
    /// a missing file is [`ZhangError::FileNotFound`]; the rest keeps opendal's details, which only the host logs
    fn into_zhang_error(self, path: &str) -> ZhangError {
        match self {
            BlockingError::Opendal(e) if e.kind() == ErrorKind::NotFound => ZhangError::FileNotFound,
            other => ZhangError::CustomError(format!("[{path}]: {other}")),
        }
    }
}

impl OpendalDataSource {
    /// run `task` on the operator and wait for it. opendal has no native blocking IO anymore: the task runs on a
    /// dedicated thread with its own runtime and its own http client, isolated from the caller's runtime (if any).
    /// This can't panic with "Cannot start a runtime from within a runtime", and never waits on a pooled http
    /// connection owned by a caller runtime that is blocked on this very call (which deadlocks a current-thread
    /// runtime).
    ///
    /// With a `timeout`, the task is abandoned once it runs longer.
    fn blocking<T, F>(&self, timeout: Option<Duration>, task: impl FnOnce(Operator) -> F + Send) -> Result<T, BlockingError>
    where
        T: Send,
        F: Future<Output = opendal::Result<T>>,
    {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let http_transport = HttpTransporter::new(ReqwestTransport::new(reqwest::Client::new()));
                    let operator = self
                        .operator
                        .clone()
                        .with_context(self.operator.base_context().with_http_transport(http_transport));
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| BlockingError::Runtime(e.to_string()))?;
                    runtime.block_on(async {
                        match timeout {
                            Some(limit) => tokio::time::timeout(limit, task(operator)).await.map_err(|_| BlockingError::TimedOut(limit))?,
                            None => task(operator).await,
                        }
                        .map_err(BlockingError::Opendal)
                    })
                })
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    // `async_recursion` adds a `#[must_use]` to the boxed future it returns
    #[allow(clippy::double_must_use)]
    #[async_recursion]
    async fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, check_file_visit: bool) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;

        let endpoint = if let Some(file) = file {
            file
        } else if let Some(datetime) = directive.datetime() {
            let date = datetime.date();
            let mut env = Environment::new();
            env.add_template("directive_output_path", &ledger.options.directive_output_path).map_err(|e| {
                warn!("{}", e);
                ZhangError::InvalidOptionValue
            })?;

            let tmpl = env.get_template("directive_output_path").map_err(|_e| {
                warn!("{}", _e);
                ZhangError::InvalidOptionValue
            })?;

            let save_path = tmpl
                .render(&context! {
                    type => directive.directive_type().to_string(),
                    year => date.year(),
                    month => date.month(),
                    month_str => date.format("%m").to_string(),
                    day => date.day(),
                    day_str => date.format("%d").to_string(),
                    ext => Path::new(main_file_endpoint).extension().and_then(|it| it.to_str()).unwrap_or("zhang"),
                })
                .map_err(|_e| ZhangError::InvalidOptionValue)?;
            let path = PathBuf::from(save_path);
            entry.join(path)
        } else {
            entry.join(main_file_endpoint)
        };
        let striped_endpoint = endpoint.strip_prefix(entry).expect("cannot strip entry prefix");

        if !has_path_visited(&ledger.visited_files, &endpoint) && check_file_visit {
            let path = match endpoint.strip_prefix(entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => endpoint.to_str().unwrap(),
            };
            self.append_directive(
                ledger,
                Directive::Include(Include {
                    file: ZhangString::QuoteString(path.to_string()),
                }),
                None,
                false,
            )
            .await?;
        }

        let content_buf = ledger.data_source.async_get(striped_endpoint.to_string_lossy().to_string()).await?;
        let content = String::from_utf8(content_buf)?;

        let appended_content = format!("{}\n{}\n", content, self.data_type.export(Spanned::new(directive, SpanInfo::default())));

        ledger
            .data_source
            .async_save(ledger, striped_endpoint.to_string_lossy().to_string(), appended_content.as_bytes())
            .await?;
        Ok(())
    }
    pub async fn from_env(source: FileSystem, server_opts: &mut ServerOpts) -> OpendalDataSource {
        let mut local_root = None;
        let operator = match source {
            FileSystem::Fs => {
                let builder = Fs::default().root(server_opts.path.to_string_lossy().to_string().as_str());
                let operator = Operator::new(builder).unwrap();
                local_root = Some(server_opts.path.canonicalize().unwrap_or_else(|_| server_opts.path.clone()));
                operator
            }
            FileSystem::WebDav => {
                let webdav_builder = Webdav::default().endpoint(&std::env::var("ZHANG_WEBDAV_ENDPOINT").expect("ZHANG_WEBDAV_ENDPOINT must be set"));
                let webdav_root = std::env::var("ZHANG_WEBDAV_ROOT").expect("ZHANG_WEBDAV_ROOT must be set");
                let webdav_builder = webdav_builder
                    .root(&webdav_root)
                    .username(std::env::var("ZHANG_WEBDAV_USERNAME").ok().as_deref().unwrap_or_default())
                    .password(std::env::var("ZHANG_WEBDAV_PASSWORD").ok().as_deref().unwrap_or_default());
                server_opts.path = PathBuf::from(&webdav_root);
                Operator::new(webdav_builder).unwrap()
            }
            FileSystem::Github => {
                let builder = Github::default()
                    .root("/")
                    .token(&std::env::var("ZHANG_GITHUB_TOKEN").expect("ZHANG_GITHUB_TOKEN must be set"))
                    .owner(&std::env::var("ZHANG_GITHUB_USER").expect("ZHANG_GITHUB_USER must be set"))
                    .repo(&std::env::var("ZHANG_GITHUB_REPO").expect("ZHANG_GITHUB_REPO must be set"));

                Operator::new(builder).unwrap()
            }
            FileSystem::S3 => {
                let mut builder = S3::default().bucket(&std::env::var("ZHANG_S3_BUCKET").expect("ZHANG_S3_BUCKET must be set"));
                let s3_root = std::env::var("ZHANG_S3_ROOT").unwrap_or_else(|_| "/".to_string());
                builder = builder.root(&s3_root);
                // optional settings, fallback to opendal defaults and the standard AWS env/profile config
                if let Ok(endpoint) = std::env::var("ZHANG_S3_ENDPOINT") {
                    builder = builder.endpoint(&endpoint);
                }
                if let Ok(region) = std::env::var("ZHANG_S3_REGION") {
                    builder = builder.region(&region);
                }
                if let Ok(access_key_id) = std::env::var("ZHANG_S3_ACCESS_KEY_ID") {
                    builder = builder.access_key_id(&access_key_id);
                }
                if let Ok(secret_access_key) = std::env::var("ZHANG_S3_SECRET_ACCESS_KEY") {
                    builder = builder.secret_access_key(&secret_access_key);
                }
                if let Ok(session_token) = std::env::var("ZHANG_S3_SESSION_TOKEN") {
                    builder = builder.session_token(&session_token);
                }
                if matches!(std::env::var("ZHANG_S3_VIRTUAL_HOST_STYLE").as_deref(), Ok("true") | Ok("1")) {
                    builder = builder.enable_virtual_host_style();
                }
                server_opts.path = PathBuf::from(&s3_root);
                Operator::new(builder).expect("cannot build s3 operator, check your s3 configuration")
            }
        };
        let is_beancount = if is_beancount_endpoint(&server_opts.endpoint) {
            info!("detected ledger type: beancount");
            true
        } else if PathBuf::from(&server_opts.endpoint).extension().is_some_and(|it| it == "zhang") {
            info!("detected ledger type: zhang");
            false
        } else {
            unreachable!("not supported data format")
        };
        let new_data_type: Box<dyn DataType<Carrier = String> + Send + Sync> = if is_beancount { Box::new(Beancount {}) } else { Box::new(ZhangDataType {}) };
        Self {
            operator,
            data_type: new_data_type,
            is_beancount,
            local_root,
        }
    }

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Spanned<Directive>>> {
        let path_string = path.to_string_lossy().to_string();
        if self.is_beancount {
            let beancount_parser = beancount::Beancount {};
            beancount_parser
                .transform(content.to_string(), Some(path_string.clone()))
                .map_err(|it| ZhangError::PestError {
                    path: path_string,
                    msg: it.to_string(),
                })
        } else {
            zhang_parse(content, path).map_err(|it| ZhangError::PestError {
                path: path_string,
                msg: it.to_string(),
            })
        }
    }
    fn go_next(&self, directive: &Spanned<Directive>) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Spanned<Directive>>) -> ZhangResult<Vec<Spanned<Directive>>> {
        Ok(directives)
    }
    async fn get_file_content(&self, path: PathBuf) -> ZhangResult<String> {
        let path = path.to_str().expect("cannot convert path to string");

        let vec = self.async_get(path.to_string()).await?;
        Ok(String::from_utf8(vec).expect("invalid utf8 content"))
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use opendal::services::Memory;
    use opendal::Operator;
    use tempfile::tempdir;
    use zhang_core::data_source::{DataSource, SourceEntry};
    use zhang_core::data_type::text::parser::parse as zhang_parse;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;
    use zhang_core::ZhangError;

    use super::{BlockingError, OpendalDataSource, PLUGIN_FILE_TIMEOUT};
    use crate::{FileSystem, ServerOpts};

    /// a remote-like source holding `files`
    async fn source(files: &[&str]) -> OpendalDataSource {
        let operator = Operator::new(Memory::default()).unwrap();
        for file in files {
            operator.write(file, file.as_bytes().to_vec()).await.unwrap();
        }
        OpendalDataSource {
            operator,
            data_type: Box::new(ZhangDataType {}),
            is_beancount: false,
            local_root: None,
        }
    }

    fn entry(name: &str, is_dir: bool) -> SourceEntry {
        SourceEntry { name: name.to_owned(), is_dir }
    }

    #[tokio::test]
    async fn should_list_a_directory_without_itself() {
        let source = source(&["main.zhang", "documents/receipt.txt", "documents/2024/a.csv"]).await;

        let mut documents = source.list("documents".to_owned(), 10).unwrap();
        documents.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(documents, vec![entry("2024", true), entry("receipt.txt", false)]);

        let mut root = source.list(String::new(), 10).unwrap();
        root.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(root, vec![entry("documents", true), entry("main.zhang", false)]);

        assert_eq!(source.get("documents/2024/a.csv".to_owned()).unwrap(), b"documents/2024/a.csv");
        assert_eq!(source.local_root(Path::new("/")), None, "a remote source has no local root");
    }

    #[tokio::test]
    async fn should_refuse_a_file_over_the_limit() {
        let source = source(&["documents/receipt.txt"]).await;
        source.operator.write("empty.txt", Vec::<u8>::new()).await.unwrap();
        let file = "documents/receipt.txt".to_owned();

        assert_eq!(source.get_limited(file.clone(), 21).unwrap(), b"documents/receipt.txt");
        assert!(matches!(source.get_limited(file.clone(), 20), Err(ZhangError::TooLarge(_))));
        assert!(matches!(source.get_limited(file, 0), Err(ZhangError::TooLarge(_))));
        assert_eq!(source.get_limited("empty.txt".to_owned(), 0).unwrap(), b"");
        assert!(matches!(source.get_limited("missing.txt".to_owned(), 10), Err(ZhangError::FileNotFound)));
    }

    #[tokio::test]
    async fn should_stop_listing_past_the_limit() {
        let source = source(&["documents/a.pdf", "documents/b.pdf", "documents/c/d.pdf"]).await;

        assert_eq!(source.list("documents".to_owned(), 3).unwrap().len(), 3);
        assert!(matches!(source.list("documents".to_owned(), 2), Err(ZhangError::TooLarge(_))));
        assert!(matches!(source.list("documents/".to_owned(), 0), Err(ZhangError::TooLarge(_))));
    }

    #[tokio::test]
    async fn should_give_up_on_a_stalled_call_at_its_timeout() {
        let source = source(&[]).await;
        let started = Instant::now();

        let result = source.blocking(Some(Duration::from_millis(50)), |_| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        });

        assert!(matches!(result, Err(BlockingError::TimedOut(_))), "{:?}", result);
        assert!(started.elapsed() < Duration::from_secs(10), "gave up after {:?}", started.elapsed());
        assert_eq!(PLUGIN_FILE_TIMEOUT, Duration::from_secs(30));
    }

    const OPENS: &str = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

    /// Loads the ledger whose main file is `main`, appends a transaction dated 2024-01-15 the way the
    /// server does, and returns the reloaded ledger.
    async fn append_coffee(dir: &Path, main: &str) -> Ledger {
        let mut opts = ServerOpts {
            path: dir.to_path_buf(),
            endpoint: main.to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);
        let ledger = Ledger::async_load(dir.to_path_buf(), main.to_string(), source.clone())
            .await
            .expect("load ledger");
        let coffee = zhang_parse("2024-01-15 * \"Shop\" \"Coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n", None)
            .expect("parse transaction")
            .remove(0)
            .data;
        ledger.data_source.async_append(&ledger, vec![coffee]).await.expect("append transaction");
        Ledger::async_load(dir.to_path_buf(), main.to_string(), source).await.expect("reload ledger")
    }

    fn assert_coffee_written_to(dir: &Path, main: &str, ledger: &Ledger, data_file: &str) {
        let written = std::fs::read_to_string(dir.join(data_file)).expect("data file is written");
        assert!(written.contains("Coffee"), "{}", written);
        let main_content = std::fs::read_to_string(dir.join(main)).unwrap();
        assert!(main_content.contains(&format!("include \"{}\"", data_file)), "{}", main_content);
        let store = ledger.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 1);
    }

    #[tokio::test]
    async fn a_beancount_ledger_writes_new_directives_to_bean_files() {
        for main in ["main.bean", "main.beancount", "main.bc"] {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join(main), OPENS).unwrap();
            let ledger = append_coffee(dir.path(), main).await;
            let ext = main.trim_start_matches("main.");
            assert_coffee_written_to(dir.path(), main, &ledger, &format!("data/2024/01.{ext}"));
            assert!(!dir.path().join("data/2024/01.zhang").exists(), "{}", main);
        }
    }

    #[tokio::test]
    async fn a_zhang_ledger_still_writes_new_directives_to_zhang_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), OPENS).unwrap();
        let ledger = append_coffee(dir.path(), "main.zhang").await;
        assert_coffee_written_to(dir.path(), "main.zhang", &ledger, "data/2024/01.zhang");
    }

    #[tokio::test]
    async fn an_explicit_directive_output_path_is_kept() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.bean"),
            format!("option \"directive_output_path\" \"books/{{{{year}}}}.beancount\"\n{OPENS}"),
        )
        .unwrap();
        let ledger = append_coffee(dir.path(), "main.bean").await;
        assert_coffee_written_to(dir.path(), "main.bean", &ledger, "books/2024.beancount");
    }
}
