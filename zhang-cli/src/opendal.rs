use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use async_recursion::async_recursion;
use beancount::Beancount;
use futures::TryStreamExt;
use log::{debug, info};
use opendal::services::{Fs, Github, Webdav, S3};
use opendal::{EntryMode, ErrorKind, HttpTransporter, Operator};
use opendal_http_transport_reqwest::ReqwestTransport;
use zhang_ast::{Directive, SpanInfo, Spanned};
use zhang_core::data_source::{directive_output_file, include_for_append, included_file, written_into, DataSource, IncludePattern, LoadResult, SourceEntry};
use zhang_core::data_type::text::parser::parse as zhang_parse;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{is_beancount_endpoint, DataType};
use zhang_core::ledger::Ledger;
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
            // the `Fs` service is jailed to the ledger's directory and a remote service holds nothing outside its root:
            // an `include` of an absolute path outside is a load error naming it, not a panic that aborts the start or
            // kills the reload task (#492)
            let striped_pathbuf = &pathbuf
                .strip_prefix(&entry)
                .map_err(|_| {
                    ZhangError::CustomError(format!(
                        "cannot include {}: it is outside the ledger's directory {}",
                        pathbuf.display(),
                        entry.display()
                    ))
                })?
                .to_path_buf();
            if let Some(pattern) = IncludePattern::parse(striped_pathbuf) {
                // listed through the blocking helper, as plugins list during a load; a directory that is not there
                // holds nothing
                let files = pattern.expand(Path::new(""), |dir| match self.list(dir.to_string_lossy().into_owned(), usize::MAX) {
                    Err(ZhangError::FileNotFound) => Ok(vec![]),
                    listed => listed,
                })?;
                load_queue.extend(files.into_iter().map(|file| entry.join(file)));
                continue;
            }
            debug!("visited entry file: {:?}", striped_pathbuf.display());
            if utils::has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get_file_content(striped_pathbuf.clone()).await?;
            let entity_directives = self.parse(&file_content, striped_pathbuf.clone())?;

            entity_directives.iter().filter_map(included_file).for_each(|buf| {
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
            directives,
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

    /// a file only, with one stat before the read: what the stat tells a directory, which a WebDAV service reads as a
    /// page listing it, or another kind of entry, is not read. A stat that fails otherwise than for a missing entry,
    /// as on a WebDAV service without a working PROPFIND, leaves the read to tell: its error decides. A refusal, as a
    /// scoped access policy or an expired token makes, is [`ZhangError::ReadRefused`], not a missing file
    async fn async_get_existing(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        match self.operator.stat(&path).await {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Ok(None),
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => debug!("[opendal] cannot stat {}, reading it: {}", path, err),
        }
        match self.operator.read(&path).await {
            Ok(data) => Ok(Some(data.to_vec())),
            Err(err) => match err.kind() {
                ErrorKind::NotFound | ErrorKind::IsADirectory | ErrorKind::NotADirectory => Ok(None),
                ErrorKind::PermissionDenied => Err(ZhangError::ReadRefused(path)),
                _ => Err(ZhangError::CustomError(format!("Error getting file content from {}: {}", path, err))),
            },
        }
    }

    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        // the files this append includes: the ledger it was given does not know them yet
        let mut included = vec![];
        for directive in directives {
            self.append_directive(ledger, directive, None, Some(&mut included)).await?;
        }
        Ok(())
    }

    async fn async_save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        info!("[opendal] save content path={}", path);
        let vec = content.to_vec();

        self.operator
            .write(&path, vec)
            .await
            .map_err(|e| ZhangError::CustomError(format!("cannot write {path}: {e}")))?;
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

    /// append `directive` to `file`, or to the file `directive_output_path` gives it ([`directive_output_file`]), which
    /// the main file then includes unless the ledger or this append (`included`) has it already. Without `included`, no
    /// include
    // `async_recursion` adds a `#[must_use]` to the boxed future it returns
    #[allow(clippy::double_must_use)]
    #[async_recursion]
    async fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, included: Option<&mut Vec<PathBuf>>) -> ZhangResult<()> {
        let (entry, _) = &ledger.entry;

        let endpoint = match file {
            Some(file) => file,
            None => directive_output_file(ledger, &directive)?,
        };
        let striped_endpoint = endpoint
            .strip_prefix(entry)
            .map_err(|_| ZhangError::CustomError(format!("{} is not in the ledger's directory", endpoint.display())))?;

        if let Some(include) = included.and_then(|included| include_for_append(ledger, &endpoint, included)) {
            self.append_directive(ledger, include, None, None).await?;
        }

        let content_buf = ledger.data_source.async_get(striped_endpoint.to_string_lossy().to_string()).await?;
        let content = String::from_utf8(content_buf)?;

        let directive = written_into(ledger, directive, striped_endpoint);
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
            // its error names the file already
            beancount::Beancount {}.transform(content.to_string(), Some(path_string))
        } else {
            zhang_parse(content, path).map_err(|it| ZhangError::PestError {
                path: path_string,
                msg: it.to_string(),
            })
        }
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

    /// the ledger loaded from the `main.zhang` at the root of a remote-like source holding `files`
    async fn load_remote(files: &[(&str, &str)]) -> Ledger {
        let operator = Operator::new(Memory::default()).unwrap();
        for (path, content) in files {
            operator.write(path, content.as_bytes().to_vec()).await.unwrap();
        }
        let source = OpendalDataSource {
            operator,
            data_type: Box::new(ZhangDataType {}),
            is_beancount: false,
            local_root: None,
        };
        Ledger::async_load(std::path::PathBuf::from("/ledger"), "main.zhang".to_owned(), Arc::new(source))
            .await
            .unwrap()
    }

    /// A pattern names the files that exist, whole names only, in any part of the path (#494): `*.zhang` at the root
    /// and `data/*/accounts.zhang`, with a literal last part, stopped the load before, and `*.zhang` took
    /// `01.zhang.bak`, whose entries were loaded twice. The main file matches its own pattern and is still read once;
    /// the hidden `.#01.zhang` is left out.
    #[tokio::test]
    async fn an_include_pattern_names_matching_files_at_any_depth() {
        let transaction = |day: &str| format!("2024-01-{day} * \"shop\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n");
        let (first, backup, hidden, second, other) = (transaction("01"), transaction("02"), transaction("03"), transaction("04"), transaction("05"));
        let ledger = load_remote(&[
            (
                "main.zhang",
                "include \"*.zhang\"\ninclude \"data/*/*.zhang\"\ninclude \"data/*/accounts.zhang\"\ninclude \"nothing/*.zhang\"\n",
            ),
            ("accounts.zhang", "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n"),
            ("data/2024/01.zhang", &first),
            ("data/2024/01.zhang.bak", &backup),
            ("data/2024/.#01.zhang", &hidden),
            ("data/2025/02.zhang", &second),
            ("data/2025/accounts.zhang", "1970-01-01 open Assets:Bank\n"),
            ("data/other.zhang", &other),
        ])
        .await;

        let mut visited: Vec<String> = ledger
            .visited_files
            .iter()
            .map(|it| it.strip_prefix("/ledger").unwrap().to_string_lossy().into_owned())
            .collect();
        visited.sort();
        assert_eq!(
            visited,
            vec![
                "accounts.zhang",
                "data/2024/01.zhang",
                "data/2025/02.zhang",
                "data/2025/accounts.zhang",
                "main.zhang"
            ]
        );
        let store = ledger.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 2);
    }

    /// the local source reading the ledger at `dir`, as `zhang serve` builds it
    async fn local_source(dir: &Path, main: &str) -> Arc<OpendalDataSource> {
        let mut opts = ServerOpts {
            path: dir.to_path_buf(),
            endpoint: main.to_owned(),
            addr: String::new(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await)
    }

    /// An `include` of an absolute path outside the ledger's directory, which the `Fs` service can never read, is a
    /// load error naming the path, not a panic that aborts `zhang serve` or kills its reload task (#492). Once the
    /// include is gone, the ledger loads again.
    #[tokio::test]
    async fn an_include_outside_the_ledger_is_a_load_error() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("x.zhang");
        std::fs::write(&outside_file, "1970-01-01 open Assets:Outside CNY\n").unwrap();
        std::fs::write(dir.path().join("main.zhang"), format!("{OPENS}include \"{}\"\n", outside_file.display())).unwrap();
        let source = local_source(dir.path(), "main.zhang").await;

        let loaded = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source.clone()).await;
        let Err(error) = loaded else { panic!("an include outside the ledger loaded") };
        let message = error.to_string();
        assert!(message.contains(&format!("cannot include {}", outside_file.display())), "{}", message);
        assert!(message.contains("outside the ledger's directory"), "{}", message);

        std::fs::write(dir.path().join("main.zhang"), OPENS).unwrap();
        let reloaded = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source).await;
        assert!(reloaded.is_ok(), "loads again once the include is gone");
    }

    /// the same on a remote source, whose root holds every file it can read
    #[tokio::test]
    async fn an_include_outside_a_remote_ledger_is_a_load_error() {
        let operator = Operator::new(Memory::default()).unwrap();
        operator.write("main.zhang", b"include \"/elsewhere/x.zhang\"\n".to_vec()).await.unwrap();
        let source = OpendalDataSource {
            operator,
            data_type: Box::new(ZhangDataType {}),
            is_beancount: false,
            local_root: None,
        };

        let loaded = Ledger::async_load(std::path::PathBuf::from("/ledger"), "main.zhang".to_owned(), Arc::new(source)).await;
        let Err(error) = loaded else { panic!("an include outside the ledger loaded") };
        assert!(error.to_string().contains("cannot include /elsewhere/x.zhang"), "{}", error);
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

    /// A ledger whose main file starts with a UTF-8 byte order mark, as some Windows editors save it, loads in both
    /// formats (#505): `zhang serve` on it exited with status 1. The spans index the text after the mark.
    #[tokio::test]
    async fn a_file_starting_with_a_byte_order_mark_loads() {
        for main in ["main.zhang", "main.bean"] {
            let dir = tempdir().unwrap();
            std::fs::write(
                dir.path().join(main),
                format!("\u{feff}{OPENS}2024-01-15 * \"Shop\" \"Coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n"),
            )
            .unwrap();
            let mut opts = ServerOpts {
                path: dir.path().to_path_buf(),
                endpoint: main.to_string(),
                addr: "".to_string(),
                port: 0,
                auth: None,
                passkey: None,
                source: None,
                no_report: true,
            };
            let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);

            let ledger = Ledger::async_load(dir.path().to_path_buf(), main.to_string(), source)
                .await
                .unwrap_or_else(|e| panic!("{}: {}", main, e));

            assert_eq!(ledger.directives[0].span.start, 0, "{main}");
            assert_eq!(ledger.directives[0].span.content.trim_end(), "1970-01-01 open Assets:Cash", "{main}");
            let store = ledger.store.read().unwrap();
            assert!(store.errors.is_empty(), "{main}: {:?}", store.errors);
            assert_eq!(store.transactions.len(), 1, "{main}");
        }
    }

    #[tokio::test]
    async fn an_append_includes_each_new_file_once() {
        // a batch of balances writes several directives to one new file: beancount refuses a file included twice
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.bean"), OPENS).unwrap();
        let mut opts = ServerOpts {
            path: dir.path().to_path_buf(),
            endpoint: "main.bean".to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);
        let ledger = Ledger::async_load(dir.path().to_path_buf(), "main.bean".to_string(), source.clone())
            .await
            .unwrap();
        let directives = zhang_parse(
            "2024-01-15 * \"Shop\" \"Coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n2024-01-16 * \"Shop\" \"Tea\"\n  Assets:Cash -3 CNY\n  Expenses:Food 3 CNY\n",
            None,
        )
        .unwrap()
        .into_iter()
        .map(|it| it.data)
        .collect();

        ledger.data_source.async_append(&ledger, directives).await.unwrap();

        let main = std::fs::read_to_string(dir.path().join("main.bean")).unwrap();
        assert_eq!(main.matches("include \"data/2024/01.bean\"").count(), 1, "{main}");
        let reloaded = Ledger::async_load(dir.path().to_path_buf(), "main.bean".to_string(), source).await.unwrap();
        let store = reloaded.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 2);
    }

    /// A `document` the server writes into a file of a beancount ledger names its file relative to that file, as
    /// beancount reads it, and zhang reads it back by its path within the ledger. A zhang ledger keeps the path
    /// within the ledger.
    #[tokio::test]
    async fn a_document_is_written_with_the_path_its_file_reads_it_by() {
        for (main, written) in [
            ("main.bean", "../../attachments/u1/a statement.pdf"),
            ("main.zhang", "attachments/u1/a statement.pdf"),
        ] {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join(main), OPENS).unwrap();
            std::fs::create_dir_all(dir.path().join("attachments/u1")).unwrap();
            std::fs::write(dir.path().join("attachments/u1/a statement.pdf"), "%PDF").unwrap();
            let mut opts = ServerOpts {
                path: dir.path().to_path_buf(),
                endpoint: main.to_string(),
                addr: "".to_string(),
                port: 0,
                auth: None,
                passkey: None,
                source: None,
                no_report: true,
            };
            let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);
            let ledger = Ledger::async_load(dir.path().to_path_buf(), main.to_string(), source.clone()).await.unwrap();
            let document = zhang_parse("2024-01-15 document Assets:Cash \"attachments/u1/a statement.pdf\"\n", None)
                .unwrap()
                .remove(0)
                .data;
            ledger.data_source.async_append(&ledger, vec![document]).await.unwrap();

            let ext = main.trim_start_matches("main.");
            let data_file = std::fs::read_to_string(dir.path().join(format!("data/2024/01.{ext}"))).unwrap();
            assert!(
                data_file.contains(&format!("2024-01-15 document Assets:Cash \"{}\"", written)),
                "{}: {}",
                main,
                data_file
            );
            let reloaded = Ledger::async_load(dir.path().to_path_buf(), main.to_string(), source).await.unwrap();
            let store = reloaded.store.read().unwrap();
            assert!(store.errors.is_empty(), "{}: {:?}", main, store.errors);
            let paths = store.documents.iter().map(|it| it.path.as_str()).collect::<Vec<_>>();
            assert_eq!(paths, vec!["attachments/u1/a statement.pdf"], "{}", main);
        }
    }

    /// the files of a beancount ledger whose data file holds four `document`s: one written by an earlier version,
    /// relative to the root; one relative to its file; one found both relative to its file and to the root; one missing
    const DOCUMENTS: &[(&str, &str)] = &[
        ("main.bean", "1970-01-01 open Assets:Cash\ninclude \"data/2024/01.bean\"\n"),
        (
            "data/2024/01.bean",
            "2024-01-01 document Assets:Cash \"attachments/legacy.pdf\"\n\
             2024-01-02 document Assets:Cash \"../../attachments/right.pdf\"\n\
             2024-01-03 document Assets:Cash \"both.pdf\"\n\
             2024-01-04 document Assets:Cash \"attachments/missing.pdf\"\n",
        ),
        ("attachments/legacy.pdf", "legacy"),
        ("attachments/right.pdf", "right"),
        ("data/2024/both.pdf", "next to its file"),
        ("both.pdf", "at the root"),
    ];

    /// The documents of [`DOCUMENTS`] on the local disk, where a stat tells whether a file exists: beancount finds each
    /// relative to its file first. The legacy one is found at the root, still downloads, and has a notice with the path
    /// beancount reads; the missing one is reported, and its download is a 404.
    async fn assert_documents(ledger: Ledger) {
        use axum::extract::State;
        use zhang_ast::error::ErrorKind;
        use zhang_server::state::SharedLedger;

        let (paths, errors) = {
            let store = ledger.store.read().unwrap();
            let paths = store.documents.iter().map(|it| it.path.clone()).collect::<Vec<_>>();
            let mut errors = store
                .errors
                .iter()
                .map(|it| {
                    let mut metas = it.metas.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>();
                    metas.sort();
                    let line = it.span.as_ref().map(|span| span.content.trim().to_owned()).unwrap_or_default();
                    (it.error_type.clone(), metas, line)
                })
                .collect::<Vec<_>>();
            errors.sort_by(|a, b| a.2.cmp(&b.2));
            (paths, errors)
        };
        assert_eq!(
            paths,
            vec![
                "attachments/legacy.pdf",
                "attachments/right.pdf",
                "data/2024/both.pdf",
                "data/2024/attachments/missing.pdf"
            ]
        );
        assert_eq!(
            errors,
            vec![
                (
                    ErrorKind::DocumentPathRelativeToRoot,
                    vec!["file=data/2024/01.bean".to_owned(), "written_as=../../attachments/legacy.pdf".to_owned()],
                    "2024-01-01 document Assets:Cash \"attachments/legacy.pdf\"".to_owned()
                ),
                (
                    ErrorKind::DocumentNotFound,
                    vec!["path=data/2024/attachments/missing.pdf".to_owned()],
                    "2024-01-04 document Assets:Cash \"attachments/missing.pdf\"".to_owned()
                ),
            ]
        );
        let state = State(SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));
        for (path, content) in [
            ("attachments/legacy.pdf", "legacy"),
            ("attachments/right.pdf", "right"),
            ("data/2024/both.pdf", "next to its file"),
        ] {
            assert_eq!(download(&state, path).await, (200, content.to_owned()), "{}", path);
        }
        let (status, _) = download(&state, "data/2024/attachments/missing.pdf").await;
        assert_eq!(status, 404);
    }

    /// the status and the body of the download of the document at `path`
    async fn download(state: &axum::extract::State<zhang_server::state::SharedLedger>, path: &str) -> (u16, String) {
        use axum::response::IntoResponse;

        let response = zhang_server::routes::document::download_document(state.clone(), zhang_server::routes::Base64Path(path.to_owned()))
            .await
            .into_response();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn on_the_local_disk_a_document_written_relative_to_the_root_is_kept_with_a_notice() {
        let dir = tempdir().unwrap();
        for (file, content) in DOCUMENTS {
            std::fs::create_dir_all(dir.path().join(file).parent().unwrap()).unwrap();
            std::fs::write(dir.path().join(file), content).unwrap();
        }
        let mut opts = ServerOpts {
            path: dir.path().to_path_buf(),
            endpoint: "main.bean".to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);
        assert_documents(Ledger::async_load(dir.path().to_path_buf(), "main.bean".to_owned(), source).await.unwrap()).await;
    }

    /// counts the calls an operator makes to its service, which a remote service answers each with a request at least,
    /// and serves some paths as a remote service may
    #[derive(Debug, Clone, Default)]
    struct Counting {
        calls: Arc<std::sync::Mutex<Vec<String>>>,
        /// paths served as WebDAV serves a directory: a stat tells a directory, a read gives a page listing it, kept at
        /// `<path>/index.html`
        directories: Arc<std::sync::Mutex<Vec<String>>>,
        /// paths whose stat and read fail with an error of this kind
        failing: Arc<std::sync::Mutex<Vec<(String, opendal::ErrorKind)>>>,
        /// paths whose stat only fails with an error of this kind, as on a WebDAV service without a working PROPFIND
        failing_stats: Arc<std::sync::Mutex<Vec<(String, opendal::ErrorKind)>>>,
        /// paths whose read only fails with an error of this kind
        failing_reads: Arc<std::sync::Mutex<Vec<(String, opendal::ErrorKind)>>>,
        /// paths whose stat tells an entry that is neither a file nor a directory
        special: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Counting {
        /// the calls made since the last time
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().drain(..).collect()
        }
    }

    impl opendal::raw::Layer for Counting {
        fn apply_service(&self, inner: opendal::raw::Servicer) -> opendal::raw::Servicer {
            Arc::new(CountingService { inner, layer: self.clone() })
        }
    }

    #[derive(Debug)]
    struct CountingService {
        inner: opendal::raw::Servicer,
        layer: Counting,
    }

    impl CountingService {
        fn count(&self, call: &str, path: &str) {
            self.layer.calls.lock().unwrap().push(format!("{} {}", call, path));
        }

        /// the error of `path` in `failing`, or in `only`
        fn failure(&self, path: &str, only: &std::sync::Mutex<Vec<(String, opendal::ErrorKind)>>) -> opendal::Result<()> {
            let failing = self.layer.failing.lock().unwrap();
            let only = only.lock().unwrap();
            match failing.iter().chain(only.iter()).find(|(failing, _)| failing == path) {
                Some((_, kind)) => Err(opendal::Error::new(*kind, "the service failed")),
                None => Ok(()),
            }
        }

        fn is_directory(&self, path: &str) -> bool {
            self.layer.directories.lock().unwrap().iter().any(|it| it == path)
        }
    }

    impl opendal::raw::Service for CountingService {
        type Reader = opendal::raw::oio::Reader;
        type Writer = opendal::raw::oio::Writer;
        type Lister = opendal::raw::oio::Lister;
        type Deleter = opendal::raw::oio::Deleter;
        type Copier = opendal::raw::oio::Copier;
        type Composer = opendal::raw::oio::Composer;

        fn info(&self) -> opendal::raw::ServiceInfo {
            self.inner.info()
        }

        fn capability(&self) -> opendal::Capability {
            self.inner.capability()
        }

        async fn create_dir(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpCreateDir) -> opendal::Result<opendal::raw::RpCreateDir> {
            self.count("create_dir", path);
            self.inner.create_dir(ctx, path, args).await
        }

        async fn stat(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpStat) -> opendal::Result<opendal::raw::RpStat> {
            self.count("stat", path);
            self.failure(path, &self.layer.failing_stats)?;
            if self.layer.special.lock().unwrap().iter().any(|it| it == path) {
                return Ok(opendal::raw::RpStat::new(opendal::MetadataBuilder::unknown().build()));
            }
            match self.is_directory(path) {
                true => self.inner.stat(ctx, &format!("{}/", path), args).await,
                false => self.inner.stat(ctx, path, args).await,
            }
        }

        fn read(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpRead) -> opendal::Result<Self::Reader> {
            self.count("read", path);
            self.failure(path, &self.layer.failing_reads)?;
            match self.is_directory(path) {
                true => self.inner.read(ctx, &format!("{}/index.html", path), args),
                false => self.inner.read(ctx, path, args),
            }
        }

        fn write(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpWrite) -> opendal::Result<Self::Writer> {
            self.count("write", path);
            self.inner.write(ctx, path, args)
        }

        fn delete(&self, ctx: &opendal::OperationContext) -> opendal::Result<Self::Deleter> {
            self.count("delete", "");
            self.inner.delete(ctx)
        }

        fn list(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpList) -> opendal::Result<Self::Lister> {
            self.count("list", path);
            self.inner.list(ctx, path, args)
        }

        fn copy(&self, ctx: &opendal::OperationContext, from: &str, to: &str, args: opendal::raw::OpCopy) -> opendal::Result<Self::Copier> {
            self.count("copy", from);
            self.inner.copy(ctx, from, to, args)
        }

        async fn rename(&self, ctx: &opendal::OperationContext, from: &str, to: &str, args: opendal::raw::OpRename) -> opendal::Result<opendal::raw::RpRename> {
            self.count("rename", from);
            self.inner.rename(ctx, from, to, args).await
        }

        async fn presign(&self, ctx: &opendal::OperationContext, path: &str, args: opendal::raw::OpPresign) -> opendal::Result<opendal::raw::RpPresign> {
            self.count("presign", path);
            self.inner.presign(ctx, path, args).await
        }
    }

    /// On a remote source, a load looks at no document: it reads the two files of the ledger, whatever its documents,
    /// and reports nothing about them, as it does not know (a document a remote source cannot be asked about cheaply is
    /// neither missing nor at a path picked for it). A document is looked for when it is downloaded: at its path
    /// relative to its file, then relative to the root, with two reads at most.
    #[tokio::test]
    async fn on_a_remote_source_documents_are_looked_for_when_downloaded() {
        let counting = Counting::default();
        let operator = Operator::new(Memory::default()).unwrap().layer(counting.clone());
        let mut documents = String::new();
        for (file, content) in DOCUMENTS.iter().filter(|(file, _)| *file != "data/2024/01.bean") {
            operator.write(file, content.as_bytes().to_vec()).await.unwrap();
        }
        for index in 0..240 {
            let dir = ["attachments", "receipts", "statements"][index % 3];
            let file = format!("{}/{:08}-0000-0000-0000-000000000000/document {}.pdf", dir, index, index);
            operator.write(&file, format!("document {}", index).into_bytes()).await.unwrap();
            // the first half written by an earlier version, relative to the root
            match index < 120 {
                true => documents.push_str(&format!("2024-01-15 document Assets:Cash \"{}\"\n", file)),
                false => documents.push_str(&format!("2024-01-15 document Assets:Cash \"../../{}\"\n", file)),
            }
        }
        let data_file = DOCUMENTS.iter().find(|(file, _)| *file == "data/2024/01.bean").unwrap().1;
        operator
            .write("data/2024/01.bean", format!("{}{}", data_file, documents).into_bytes())
            .await
            .unwrap();
        counting.calls();

        let source = OpendalDataSource {
            operator: operator.clone(),
            data_type: Box::new(beancount::Beancount {}),
            is_beancount: true,
            local_root: None,
        };
        // a ledger of its own, for the cache of downloads
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let entry = std::path::PathBuf::from(format!("/ledger-{}", nanos));
        let ledger = Ledger::async_load(entry, "main.bean".to_owned(), Arc::new(source)).await.unwrap();

        let mut calls = counting.calls();
        calls.sort();
        assert_eq!(calls, vec!["read data/2024/01.bean", "read main.bean"]);
        {
            let store = ledger.store.read().unwrap();
            assert!(store.errors.is_empty(), "{:?}", store.errors);
            assert_eq!(store.documents.len(), 244);
            let of = |path: &str| {
                let document = store.documents.iter().find(|it| it.path == path).unwrap_or_else(|| panic!("{}", path));
                document.alternate.clone()
            };
            assert_eq!(of("data/2024/attachments/legacy.pdf"), Some("attachments/legacy.pdf".to_owned()));
            assert_eq!(of("attachments/right.pdf"), None, "the root-relative reading leaves the ledger");
            assert_eq!(of("data/2024/both.pdf"), Some("both.pdf".to_owned()));
        }

        let state = axum::extract::State(zhang_server::state::SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));
        let legacy = "attachments/00000000-0000-0000-0000-000000000000/document 0.pdf";
        let new = "attachments/00000120-0000-0000-0000-000000000000/document 120.pdf";
        let stat = |path: &str| format!("stat {}", path);
        let read = |path: &str| format!("read {}", path);
        let legacy_path = format!("data/2024/{}", legacy);
        let missing = "data/2024/attachments/missing.pdf";
        // a stat before each read, so a directory is never read as a document
        for (path, expected, calls) in [
            // nothing at its path: its alternate
            (legacy_path.as_str(), (200, "document 0"), vec![stat(&legacy_path), stat(legacy), read(legacy)]),
            (new, (200, "document 120"), vec![stat(new), read(new)]),
            (
                "data/2024/both.pdf",
                (200, "next to its file"),
                vec![stat("data/2024/both.pdf"), read("data/2024/both.pdf")],
            ),
            (missing, (404, ""), vec![stat(missing), stat("attachments/missing.pdf")]),
            // read again: kept, each by the path it was read at, so the legacy one is looked at its path first
            (legacy_path.as_str(), (200, "document 0"), vec![stat(&legacy_path)]),
            (new, (200, "document 120"), vec![]),
            // a miss is not kept
            (missing, (404, ""), vec![stat(missing), stat("attachments/missing.pdf")]),
        ] {
            let (status, body) = download(&state, path).await;
            let body = if status == 200 { body } else { String::new() };
            assert_eq!((status, body.as_str()), expected, "{}", path);
            assert_eq!(counting.calls(), calls, "{}", path);
        }

        // files put in place since: the missing one is served, and so is the legacy one put at its path
        operator.write(missing, b"found since".to_vec()).await.unwrap();
        operator.write(&legacy_path, b"moved to its path".to_vec()).await.unwrap();
        counting.calls();
        assert_eq!(download(&state, missing).await, (200, "found since".to_owned()));
        assert_eq!(download(&state, &legacy_path).await, (200, "moved to its path".to_owned()));
        assert_eq!(counting.calls(), vec![stat(missing), read(missing), stat(&legacy_path), read(&legacy_path)]);

        // an empty document is a document, kept as it is
        operator.write("attachments/empty.pdf", Vec::<u8>::new()).await.unwrap();
        counting.calls();
        assert_eq!(download(&state, "attachments/empty.pdf").await, (200, String::new()));
        assert_eq!(download(&state, "attachments/empty.pdf").await, (200, String::new()));
        assert_eq!(counting.calls(), vec![stat("attachments/empty.pdf"), read("attachments/empty.pdf")]);

        // a directory, which WebDAV reads as a page listing it, is no document
        operator.write("attachments/u9/index.html", b"<html>the listing</html>".to_vec()).await.unwrap();
        operator.write("attachments/u9/a.pdf", b"a".to_vec()).await.unwrap();
        counting.directories.lock().unwrap().push("attachments/u9".to_owned());
        counting.calls();
        assert_eq!(download(&state, "attachments/u9").await.0, 404);
        assert_eq!(counting.calls(), vec![stat("attachments/u9")]);

        // an entry the service refuses to read, as a scoped access policy or a link out of a WebDAV directory makes it,
        // is refused, not missing; a service failing is an error
        for file in [
            "refused.pdf",
            "broken.pdf",
            "stat refused.pdf",
            "stat unsupported.pdf",
            "stat broken.pdf",
            "read broken.pdf",
            "special",
        ] {
            operator.write(&format!("attachments/{}", file), file.as_bytes().to_vec()).await.unwrap();
        }
        counting.failing.lock().unwrap().extend([
            ("attachments/refused.pdf".to_owned(), opendal::ErrorKind::PermissionDenied),
            ("attachments/broken.pdf".to_owned(), opendal::ErrorKind::Unexpected),
        ]);
        counting.calls();
        assert_eq!(
            download(&state, "attachments/refused.pdf").await,
            (
                403,
                "{\"message\":\"the storage refused to read attachments/refused.pdf\",\"origin\":\"with_rejection\"}".to_owned()
            )
        );
        assert_eq!(counting.calls(), vec![stat("attachments/refused.pdf"), read("attachments/refused.pdf")]);
        let (status, message) = download(&state, "attachments/broken.pdf").await;
        assert_eq!(status, 500, "{}", message);
        // the stat failed otherwise than for a missing entry: the read was tried, and its error decided
        assert_eq!(counting.calls(), vec![stat("attachments/broken.pdf"), read("attachments/broken.pdf")]);

        // a stat that fails otherwise than for a missing entry, as without a working PROPFIND: the read tells
        counting.failing_stats.lock().unwrap().extend([
            ("attachments/stat refused.pdf".to_owned(), opendal::ErrorKind::PermissionDenied),
            ("attachments/stat unsupported.pdf".to_owned(), opendal::ErrorKind::Unsupported),
            ("attachments/stat broken.pdf".to_owned(), opendal::ErrorKind::Unexpected),
            ("attachments/stat gone.pdf".to_owned(), opendal::ErrorKind::Unexpected),
        ]);
        for file in ["stat refused.pdf", "stat unsupported.pdf", "stat broken.pdf"] {
            let path = format!("attachments/{}", file);
            assert_eq!(download(&state, &path).await, (200, file.to_owned()), "{}", path);
            assert_eq!(counting.calls(), vec![stat(&path), read(&path)]);
        }
        assert_eq!(download(&state, "attachments/stat gone.pdf").await.0, 404);
        // a read failing after a good stat is an error, not a missing document
        counting
            .failing_reads
            .lock()
            .unwrap()
            .push(("attachments/read broken.pdf".to_owned(), opendal::ErrorKind::Unexpected));
        counting.calls();
        assert_eq!(download(&state, "attachments/read broken.pdf").await.0, 500);
        assert_eq!(counting.calls(), vec![stat("attachments/read broken.pdf"), read("attachments/read broken.pdf")]);

        // an entry that is neither a file nor a directory is never read
        counting.special.lock().unwrap().push("attachments/special".to_owned());
        counting.calls();
        assert_eq!(download(&state, "attachments/special").await.0, 404);
        assert_eq!(counting.calls(), vec![stat("attachments/special")]);
    }

    /// On a remote source whose stat fails, a download reads the document, and the read's error decides: missing is
    /// a 404, refused a 403, anything else a 500.
    #[tokio::test]
    async fn without_a_working_stat_the_read_decides() {
        let counting = Counting::default();
        let operator = Operator::new(Memory::default()).unwrap().layer(counting.clone());
        operator.write("main.bean", b"1970-01-01 open Assets:Cash\n".to_vec()).await.unwrap();
        for file in ["refused.pdf", "broken.pdf"] {
            operator.write(&format!("attachments/{}", file), b"%PDF".to_vec()).await.unwrap();
        }
        for (file, kind) in [
            ("refused.pdf", opendal::ErrorKind::PermissionDenied),
            ("broken.pdf", opendal::ErrorKind::Unexpected),
            ("gone.pdf", opendal::ErrorKind::Unexpected),
        ] {
            counting
                .failing_stats
                .lock()
                .unwrap()
                .push((format!("attachments/{}", file), opendal::ErrorKind::Unsupported));
            if file != "gone.pdf" {
                counting.failing_reads.lock().unwrap().push((format!("attachments/{}", file), kind));
            }
        }
        let source = OpendalDataSource {
            operator,
            data_type: Box::new(beancount::Beancount {}),
            is_beancount: true,
            local_root: None,
        };
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let entry = std::path::PathBuf::from(format!("/no-stat-{}", nanos));
        let ledger = Ledger::async_load(entry, "main.bean".to_owned(), Arc::new(source)).await.unwrap();
        let state = axum::extract::State(zhang_server::state::SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));
        for (file, status) in [("gone.pdf", 404), ("refused.pdf", 403), ("broken.pdf", 500)] {
            let path = format!("attachments/{}", file);
            assert_eq!(download(&state, &path).await.0, status, "{}", path);
            assert_eq!(
                counting.calls().iter().filter(|it| it.ends_with(&path)).count(),
                2,
                "{}: a stat and a read",
                path
            );
        }
    }

    /// The documents kept from a remote source are kept for their ledger: two ledgers with a document at one path each
    /// get their own.
    #[tokio::test]
    async fn the_documents_kept_from_a_remote_source_are_those_of_its_ledger() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let mut states = vec![];
        for name in ["first", "second"] {
            let operator = Operator::new(Memory::default()).unwrap();
            operator.write("main.bean", b"1970-01-01 open Assets:Cash\n".to_vec()).await.unwrap();
            operator
                .write("attachments/statement.pdf", format!("the {} one", name).into_bytes())
                .await
                .unwrap();
            let source = OpendalDataSource {
                operator,
                data_type: Box::new(beancount::Beancount {}),
                is_beancount: true,
                local_root: None,
            };
            let entry = std::path::PathBuf::from(format!("/{}-{}", name, nanos));
            let ledger = Ledger::async_load(entry, "main.bean".to_owned(), Arc::new(source)).await.unwrap();
            states.push(axum::extract::State(zhang_server::state::SharedLedger(Arc::new(tokio::sync::RwLock::new(
                ledger,
            )))));
        }
        for (state, content) in states.iter().zip(["the first one", "the second one"]) {
            assert_eq!(download(state, "attachments/statement.pdf").await, (200, content.to_owned()));
        }
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

    /// A write the storage refuses, as a read-only folder or a file of another user makes it, is an error naming the
    /// file, never a panic, and the server answers it with a message the web UI shows (#494).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_refused_write_is_an_error_the_server_answers() {
        use std::os::unix::fs::PermissionsExt;

        use axum::extract::State;
        use axum::response::IntoResponse;
        use zhang_server::request::FileUpdateRequest;
        use zhang_server::routes::file::update_file_content;
        use zhang_server::routes::Base64Path;
        use zhang_server::state::{SharedLedger, SharedReloadSender};
        use zhang_server::ReloadSender;

        let dir = tempdir().unwrap();
        let main = dir.path().join("main.zhang");
        std::fs::write(&main, OPENS).unwrap();
        let mut opts = ServerOpts {
            path: dir.path().to_path_buf(),
            endpoint: "main.zhang".to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await);
        let ledger = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_string(), source.clone())
            .await
            .unwrap();
        let coffee = zhang_parse("2024-01-15 * \"Shop\" \"Coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n", None)
            .unwrap()
            .remove(0)
            .data;
        let set_mode = |path: &Path, mode: u32| std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        // neither the main file nor the folder can be written: no file is saved, no `data/` folder created
        set_mode(&main, 0o444);
        set_mode(dir.path(), 0o555);

        let saved = source.async_save(&ledger, "main.zhang".to_owned(), b"1970-01-01 open Assets:Cash\n").await;
        let appended = source.async_append(&ledger, vec![coffee]).await;
        let state = State(SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender(sender))));
        let request = axum::Json(FileUpdateRequest {
            content: "1970-01-01 open Assets:Cash\n".to_owned(),
        });
        let response = update_file_content(state, reload, Base64Path("main.zhang".to_owned()), request)
            .await
            .into_response();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        // writable again, so the folder can be removed, whatever the assertions below find
        set_mode(dir.path(), 0o755);
        set_mode(&main, 0o644);

        let error = saved.expect_err("the main file is read-only");
        assert!(error.to_string().contains("main.zhang"), "{}", error);
        let error = appended.expect_err("the folder is read-only");
        assert!(error.to_string().contains(".zhang"), "{}", error);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), OPENS, "nothing was written");
        assert!(!dir.path().join("data").exists(), "no data folder was created");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, 500, "{}", body);
        let message = body["message"].as_str().unwrap_or_default();
        assert!(message.contains("main.zhang"), "{}", body);
    }
}
