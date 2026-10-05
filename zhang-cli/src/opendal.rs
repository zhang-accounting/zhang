use std::fmt::{Display, Formatter};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use beancount::Beancount;
use futures::TryStreamExt;
use log::{debug, info};
use opendal::services::{Fs, Github, Webdav, S3};
use opendal::{EntryMode, ErrorKind, HttpTransporter, Operator};
use opendal_http_transport_reqwest::ReqwestTransport;
use zhang_ast::{Directive, SpanInfo, Spanned};
use zhang_core::data_source::{DataSource, LedgerFiles, LoadResult, SourceEntry};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{DataType, Dialect};
use zhang_core::error::{storage_error, Access, StorageFailure};
use zhang_core::ledger::Ledger;
use zhang_core::{ZhangError, ZhangResult};

use crate::{FileSystem, ServerOpts};

/// how long a plugin's read or listing of a remote ledger may take before the host gives up on it. The plugin
/// waits for the host meanwhile, and its own timeout cannot interrupt the host, so a stalled server would
/// otherwise stall the load
const PLUGIN_FILE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OpendalDataSource {
    operator: Operator,
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
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
            .map_err(|e| e.into_zhang_error(&path))
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

    /// The `Fs` service is jailed to the ledger's directory and a remote service holds nothing outside its root: an
    /// `include` of a path outside names no file this source has, and is reported as missing, never a panic that
    /// aborts the start or kills the reload task (#492). A pattern lists through the blocking helper, as plugins list
    /// during a load
    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let mut files = LedgerFiles::new(entry, &endpoint);
        let list = |dir: &Path| match self.list(dir.to_string_lossy().into_owned(), usize::MAX) {
            // a directory that is not there holds nothing
            Err(ZhangError::FileNotFound) => Ok(vec![]),
            listed => listed,
        };
        while let Some(file) = files.next(list)? {
            let content = self.async_get(file.path()).await;
            files.read(file, content, &*self.data_type)?;
        }
        Ok(files.finish())
    }

    /// [`ZhangError::FileNotFound`] for a file that is not there, never an empty file: a missing `include` or plugin
    /// module read as empty was loaded as such, and the module cached empty (#487, #494)
    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.operator
            .read(&path)
            .await
            .map(|data| data.to_vec())
            .map_err(|err| storage_error(&path, Access::Read, storage_failure(&err), err))
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
            // what turned out to be no file is none here
            Err(err) if matches!(err.kind(), ErrorKind::IsADirectory | ErrorKind::NotADirectory) => Ok(None),
            Err(err) => match storage_error(&path, Access::Read, storage_failure(&err), err) {
                ZhangError::FileNotFound => Ok(None),
                error => Err(error),
            },
        }
    }

    async fn async_save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        info!("[opendal] save content path={}", path);
        let vec = content.to_vec();

        self.operator
            .write(&path, vec)
            .await
            .map_err(|e| storage_error(&path, Access::Write, storage_failure(&e), e))?;
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
    /// the error of reading `path`, by the one mapping of storage errors ([`storage_error`]): a missing file is
    /// [`ZhangError::FileNotFound`], a refusal [`ZhangError::ReadRefused`]; the rest keeps opendal's details, which only
    /// the host logs
    fn into_zhang_error(self, path: &str) -> ZhangError {
        let failure = match &self {
            BlockingError::Opendal(e) => storage_failure(e),
            BlockingError::TimedOut(_) | BlockingError::Runtime(_) => StorageFailure::Other,
        };
        storage_error(path, Access::Read, failure, self)
    }
}

/// what opendal's `error` says of the access that failed, for [`storage_error`]
fn storage_failure(error: &opendal::Error) -> StorageFailure {
    match error.kind() {
        ErrorKind::NotFound => StorageFailure::NotFound,
        ErrorKind::PermissionDenied => StorageFailure::Refused,
        _ => StorageFailure::Other,
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

    /// The source `zhang serve` reads the ledger of `server_opts` from, through `source`: its files are read in the
    /// format of the main file ([`Dialect::of`]), and a main file whose extension tells no format is an error
    pub async fn from_env(source: FileSystem, server_opts: &mut ServerOpts) -> ZhangResult<OpendalDataSource> {
        let dialect = Dialect::of(&server_opts.endpoint)?;
        info!("detected ledger type: {}", dialect.name());
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
        let data_type: Box<dyn DataType<Carrier = String> + Send + Sync> = match dialect {
            Dialect::Zhang => Box::new(ZhangDataType {}),
            Dialect::Beancount => Box::new(Beancount {}),
        };
        Ok(Self {
            operator,
            data_type,
            local_root,
        })
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
    use zhang_core::inputs::ExtraInput;
    use zhang_core::ledger::Ledger;
    use zhang_core::{ZhangError, ZhangResult};

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
            local_root: None,
        };
        Ledger::async_load(std::path::PathBuf::from("/ledger"), "main.zhang".to_owned(), Arc::new(source))
            .await
            .unwrap()
    }

    /// A pattern names the files that exist, whole names only, in any part of the path (#494): `*.zhang` at the root
    /// and `data/*/accounts.zhang`, with a literal last part, stopped the load before, and `*.zhang` took
    /// `01.zhang.bak`, whose entries were loaded twice. The main file matches its own pattern and is still read once;
    /// the hidden `.#01.zhang` is left out. `nothing/*.zhang`, which matches no file, is an error on its `include`.
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
        assert_eq!(errors_of(&ledger), vec![include_not_found("nothing/*.zhang")]);
        let store = ledger.store.read().unwrap();
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
        Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap())
    }

    /// the errors of `ledger`, by the text of their directive: the kind, the metas as `key=value` in order, and the text
    fn errors_of(ledger: &Ledger) -> Vec<(zhang_ast::error::ErrorKind, Vec<String>, String)> {
        let store = ledger.store.read().unwrap();
        let mut errors: Vec<_> = store
            .errors
            .iter()
            .map(|it| {
                let mut metas: Vec<String> = it.metas.iter().map(|(key, value)| format!("{}={}", key, value)).collect();
                metas.sort();
                let text = it.span.as_ref().map(|span| span.content.trim().to_owned()).unwrap_or_default();
                (it.error_type.clone(), metas, text)
            })
            .collect();
        errors.sort_by(|a, b| a.2.cmp(&b.2));
        errors
    }

    /// an `IncludeNotFound` error on the `include` of `path`
    fn include_not_found(path: &str) -> (zhang_ast::error::ErrorKind, Vec<String>, String) {
        (
            zhang_ast::error::ErrorKind::IncludeNotFound,
            vec![format!("path={}", path)],
            format!("include \"{}\"", path),
        )
    }

    /// An `include` naming a file that is not there is an error on that `include`, with the path as it is written, and
    /// the rest of the ledger loads (#494): the file was read as an empty one, without an error, and listed in the file
    /// editor. A relative path is looked for next to the file holding the `include`. A pattern that matches no file is
    /// reported the same way, as beancount reports both. Creating a missing file makes the ledger stale
    #[tokio::test]
    async fn an_include_naming_no_file_is_an_error_on_it() {
        let ledger = load_remote(&[
            (
                "main.zhang",
                "include \"accounts.zhang\"\ninclude \"acounts/typo.zhang\"\ninclude \"nothing/*.zhang\"\ninclude \"data/2024.zhang\"\n",
            ),
            ("accounts.zhang", OPENS),
            (
                "data/2024.zhang",
                "include \"sibling.zhang\"\n2024-01-01 * \"shop\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n",
            ),
        ])
        .await;

        assert_eq!(
            errors_of(&ledger),
            vec![
                include_not_found("acounts/typo.zhang"),
                include_not_found("nothing/*.zhang"),
                include_not_found("sibling.zhang")
            ]
        );
        let files: Vec<_> = {
            let store = ledger.store.read().unwrap();
            store.errors.iter().map(|it| it.span.as_ref().and_then(|span| span.filename.clone())).collect()
        };
        assert_eq!(
            files,
            vec![Some("main.zhang".into()), Some("main.zhang".into()), Some("data/2024.zhang".into())],
            "each error is on its `include`"
        );
        let visited: Vec<_> = ledger.visited_files.iter().map(|it| it.strip_prefix("/ledger").unwrap().to_owned()).collect();
        assert_eq!(
            visited,
            vec![Path::new("main.zhang"), Path::new("accounts.zhang"), Path::new("data/2024.zhang")]
        );
        assert_eq!(ledger.store.read().unwrap().transactions.len(), 1, "the files that are there load");
        assert_eq!(
            ledger.extra_inputs.iter().cloned().collect::<Vec<_>>(),
            vec![ExtraInput::File("acounts/typo.zhang".into()), ExtraInput::File("data/sibling.zhang".into())]
        );
    }

    /// A ledger whose main file is not there yet is empty, without an error, as `zhang serve` starts on a new folder
    /// (the main file is no `include`). The first entry recorded in the web UI writes it
    #[tokio::test]
    async fn a_missing_main_file_is_an_empty_ledger() {
        let empty = load_remote(&[]).await;
        assert!(errors_of(&empty).is_empty(), "{:?}", errors_of(&empty));
        assert_eq!(empty.visited_files, vec![Path::new("/ledger/main.zhang")]);

        let dir = tempdir().unwrap();
        let ledger = append_coffee(dir.path(), "main.zhang").await;

        let main = std::fs::read_to_string(dir.path().join("main.zhang")).expect("the main file is written");
        assert_eq!(main.trim(), "include \"data/2024/01.zhang\"");
        assert!(std::fs::read_to_string(dir.path().join("data/2024/01.zhang")).unwrap().contains("Coffee"));
        assert_eq!(ledger.store.read().unwrap().transactions.len(), 1);
    }

    /// An `include` of an absolute path outside the ledger's directory, which the `Fs` service can never read, names no
    /// file of the ledger: it is an error on the `include`, as for a file that is not there, and the rest of the ledger
    /// loads (#494). It was a panic that aborted `zhang serve` or killed its reload task (#492), then a load error
    #[tokio::test]
    async fn an_include_outside_the_ledger_is_an_error_on_it() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("x.zhang").display().to_string();
        std::fs::write(&outside_file, "1970-01-01 open Assets:Outside CNY\n").unwrap();
        std::fs::write(dir.path().join("main.zhang"), format!("{OPENS}include \"{}\"\n", outside_file)).unwrap();
        let source = local_source(dir.path(), "main.zhang").await;

        let ledger = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source)
            .await
            .expect("the rest of the ledger loads");

        assert_eq!(errors_of(&ledger), vec![include_not_found(&outside_file)]);
        let store = ledger.store.read().unwrap();
        assert!(store.accounts.contains_key("Assets:Cash"));
        assert!(!store.accounts.contains_key("Assets:Outside"));
        assert!(ledger.extra_inputs.is_empty(), "a file outside the ledger's directory is not watched");
    }

    /// The local source the tests load through loads a ledger as the one `zhang serve` runs does: the same files, named
    /// alike in the spans of their directives, and the same errors. An `include` outside the ledger's directory is an
    /// error on it in both, a pattern matching nothing too, and a file named twice is read once. The local source read
    /// the files outside, named every file by its full path, and failed the load of a ledger without its main file.
    #[tokio::test]
    async fn the_local_source_loads_a_ledger_as_zhang_serve_does() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("ledger");
        let outside = root.with_file_name("outside.zhang");
        std::fs::write(&outside, "1970-01-01 open Assets:Outside\n").unwrap();
        let write = |path: &str, content: &str| {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), content).unwrap();
        };
        write(
            "main.zhang",
            &format!(
                "include \"accounts.zhang\"\ninclude \"./data/../accounts.zhang\"\ninclude \"data/*.zhang\"\ninclude \"none/*.zhang\"\n\
                 include \"../outside.zhang\"\ninclude \"{}\"\ninclude \"typo.zhang\"\n",
                outside.display()
            ),
        );
        write("accounts.zhang", OPENS);
        write(
            "data/2024.zhang",
            "include \"sibling.zhang\"\n2024-01-01 * \"shop\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n",
        );
        write("data/sibling.zhang", "2024-01-02 * \"shop\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n");

        let served = Ledger::async_load(root.clone(), "main.zhang".to_owned(), local_source(&root, "main.zhang").await)
            .await
            .unwrap();
        let local = Arc::new(zhang_core::data_source::LocalFileSystemDataSource::new(ZhangDataType {}));
        let tested = Ledger::async_load(root.clone(), "main.zhang".to_owned(), local.clone()).await.unwrap();

        let loaded = |ledger: &Ledger| {
            let spans: Vec<_> = ledger.directives.iter().chain(&ledger.metas).map(|it| it.span.filename.clone()).collect();
            let accounts: std::collections::BTreeSet<_> = ledger.store.read().unwrap().accounts.keys().cloned().collect();
            (ledger.visited_files.clone(), spans, errors_of(ledger), accounts)
        };
        assert_eq!(loaded(&tested), loaded(&served));
        let (visited, _, errors, accounts) = loaded(&served);
        assert_eq!(
            visited,
            ["main.zhang", "accounts.zhang", "data/2024.zhang", "data/sibling.zhang"].map(|it| root.join(it))
        );
        let outside = outside.display().to_string();
        assert_eq!(
            errors.iter().map(|it| it.2.as_str()).collect::<Vec<_>>(),
            vec![
                "include \"../outside.zhang\"".to_owned(),
                format!("include \"{}\"", outside),
                "include \"none/*.zhang\"".to_owned(),
                "include \"typo.zhang\"".to_owned(),
            ]
        );
        assert!(!accounts.contains("Assets:Outside"));

        let empty = tempdir().unwrap();
        let tested = Ledger::async_load(empty.path().to_path_buf(), "main.zhang".to_owned(), local).await;
        assert!(errors_of(&tested.expect("a ledger without its main file is empty")).is_empty());
    }

    /// the same on a remote source, whose root holds every file it can read
    #[tokio::test]
    async fn an_include_outside_a_remote_ledger_is_an_error_on_it() {
        let ledger = load_remote(&[("main.zhang", "include \"/elsewhere/x.zhang\"\n")]).await;

        assert_eq!(errors_of(&ledger), vec![include_not_found("/elsewhere/x.zhang")]);
    }

    /// A `plugin` whose module is not there stops the load, naming it, and caches nothing (#487): the module was read as
    /// an empty one, cached as a 0-byte `.cache/plugins/<hash>.wasm`, and the load failed on it with a wasm error. On
    /// the local disk, an absolute path is looked for within the ledger's directory, as every path the source reads:
    /// a module given so is not found
    #[tokio::test]
    async fn a_missing_plugin_module_stops_the_load_and_caches_nothing() {
        let dir = tempdir().unwrap();
        let unique = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        std::fs::create_dir(dir.path().join("plugins")).unwrap();
        std::fs::write(dir.path().join("plugins/there.wasm"), b"(module)").unwrap();
        let absolute = dir.path().join("plugins/there.wasm").display().to_string();
        let source = local_source(dir.path(), "main.zhang").await;

        for module in [format!("plugins/missing-{}.wasm", unique), absolute] {
            std::fs::write(
                dir.path().join("main.zhang"),
                format!("option \"features.plugin\" \"true\"\nplugin \"{}\"\n{}", module, OPENS),
            )
            .unwrap();

            let loaded = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source.clone()).await;

            let Err(error) = loaded else { panic!("{} loaded", module) };
            assert!(error.to_string().contains(&format!("plugin module not found: {}", module)), "{}", error);
            let cached = Path::new(".cache/plugins").join(format!("{}.wasm", zhang_server::util::sha256_hex(module.as_bytes())));
            assert!(!cached.exists(), "{} is cached as {}", module, cached.display());
        }
    }

    /// The file editor is answered 404 for a file that is not there, which it was shown as an empty file (#494). The
    /// main file of a ledger started without one is the empty ledger served, shown empty to be written
    #[tokio::test]
    async fn the_editor_is_answered_404_for_a_missing_file() {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use zhang_server::routes::file::get_file_content;
        use zhang_server::routes::Base64Path;
        use zhang_server::state::SharedLedger;

        let dir = tempdir().unwrap();
        let source = local_source(dir.path(), "main.zhang").await;
        let ledger = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source)
            .await
            .expect("an empty ledger");
        let state = State(SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));

        for (path, status, content) in [("accounts.zhang", 404, None), ("main.zhang", 200, Some(""))] {
            let response = get_file_content(state.clone(), Base64Path(path.to_owned())).await.into_response();
            assert_eq!(response.status().as_u16(), status, "{}", path);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            match content {
                Some(content) => assert_eq!(body["data"]["content"], content, "{}", path),
                None => assert!(body["message"].as_str().unwrap_or_default().contains(path), "{}", body),
            }
        }
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
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
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
            let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());

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

    /// A file that is not UTF-8 text, as one with a latin-1 `é` in a comment, stops the load with an error naming the
    /// file and the line, at the start and on a reload alike: `zhang serve` panicked at the start (exit code 101), and a
    /// reload failure named no file, its message a dump of the file's bytes.
    #[tokio::test]
    async fn a_file_that_is_not_utf8_is_a_load_error_naming_it() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), format!("{}include \"bad.zhang\"\n", OPENS)).unwrap();
        std::fs::write(dir.path().join("bad.zhang"), b"1970-01-01 open Assets:Bank\n; \xe9t\xe9\n").unwrap();
        let source = local_source(dir.path(), "main.zhang").await;

        let error = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source.clone())
            .await
            .err()
            .expect("the ledger does not load");
        assert!(matches!(&error, ZhangError::InvalidUtf8 { path, line: 2 } if path == "bad.zhang"), "{}", error);

        std::fs::write(dir.path().join("bad.zhang"), "1970-01-01 open Assets:Bank\n; été\n").unwrap();
        let mut ledger = Ledger::async_load(dir.path().to_path_buf(), "main.zhang".to_owned(), source)
            .await
            .expect("the file fixed loads");
        std::fs::write(dir.path().join("bad.zhang"), b"; \xe9t\xe9\n1970-01-01 open Assets:Bank\n").unwrap();
        let error = ledger.async_reload().await.expect_err("the reload fails");
        let failure = zhang_server::state::ReloadFailure::from(&error);
        assert_eq!(failure.file.as_deref(), Some("bad.zhang"));
        assert_eq!(
            failure.message,
            "the file bad.zhang is not UTF-8 text: line 1 holds a byte that is not UTF-8. Save the file with the UTF-8 encoding"
        );
        assert!(ledger.store.read().unwrap().accounts.contains_key("Assets:Bank"), "the ledger stays as loaded");
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
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
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
            let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
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
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
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
        /// paths whose write fails with an error of this kind
        failing_writes: Arc<std::sync::Mutex<Vec<(String, opendal::ErrorKind)>>>,
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
            if let Some((_, kind)) = self.layer.failing_writes.lock().unwrap().iter().find(|(failing, _)| failing == path) {
                return Err(opendal::Error::new(*kind, "the service failed"));
            }
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
            (403, "{\"message\":\"the storage refused to read attachments/refused.pdf\"}".to_owned())
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

    /// A refusal of the storage is one error whatever reads or writes the file: a read refused is
    /// [`ZhangError::ReadRefused`] and a write refused [`ZhangError::WriteRefused`], naming the file only, never
    /// opendal's details, and the API answers both with a 403. The file editor answered an unreadable file with a 500
    /// holding opendal's details, while a document download answered 403.
    #[tokio::test]
    async fn a_refusal_of_the_storage_is_one_error_whatever_meets_it() {
        use axum::response::IntoResponse;
        use zhang_server::routes::file::{get_file_content, update_file_content};
        use zhang_server::routes::Base64Path;

        let counting = Counting::default();
        let operator = Operator::new(Memory::default()).unwrap().layer(counting.clone());
        operator
            .write("main.zhang", b"include \"more.zhang\"\n1970-01-01 open Assets:Cash\n".to_vec())
            .await
            .unwrap();
        operator.write("more.zhang", b"1970-01-01 open Expenses:Food\n".to_vec()).await.unwrap();
        let source = Arc::new(OpendalDataSource {
            operator,
            data_type: Box::new(ZhangDataType {}),
            local_root: None,
        });
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let ledger = Ledger::async_load(std::path::PathBuf::from(format!("/refused-{}", nanos)), "main.zhang".to_owned(), source.clone())
            .await
            .unwrap();
        counting
            .failing_reads
            .lock()
            .unwrap()
            .push(("more.zhang".to_owned(), opendal::ErrorKind::PermissionDenied));
        counting
            .failing_writes
            .lock()
            .unwrap()
            .push(("more.zhang".to_owned(), opendal::ErrorKind::PermissionDenied));

        let read_refused = |result: ZhangResult<()>| matches!(result, Err(ZhangError::ReadRefused(path)) if path == "more.zhang");
        assert!(read_refused(source.get("more.zhang".to_owned()).map(drop)));
        assert!(read_refused(source.get_limited("more.zhang".to_owned(), 1024).map(drop)));
        assert!(read_refused(source.async_get("more.zhang".to_owned()).await.map(drop)));
        assert!(read_refused(source.async_get_existing("more.zhang".to_owned()).await.map(drop)));
        assert!(read_refused(source.async_get_unchanged("more.zhang".to_owned(), &[]).await.map(drop)));
        let saved = source.async_save(&ledger, "more.zhang".to_owned(), b"".as_slice()).await;
        assert!(matches!(&saved, Err(ZhangError::WriteRefused(path)) if path == "more.zhang"), "{:?}", saved);

        // the file editor, reading and saving it
        let state = axum::extract::State(zhang_server::state::SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger))));
        let (sender, _) = tokio::sync::mpsc::channel(1);
        let reload = axum::extract::State(zhang_server::state::SharedReloadSender(Arc::new(zhang_server::ReloadSender::new(sender))));
        let answer = |response: axum::response::Response| async move {
            let status = response.status().as_u16();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&body).into_owned())
        };
        let read = get_file_content(state.clone(), Base64Path("more.zhang".to_owned())).await.into_response();
        assert_eq!(answer(read).await, (403, "{\"message\":\"the storage refused to read more.zhang\"}".to_owned()));
        let request = serde_json::from_value(serde_json::json!({ "content": "1970-01-01 open Expenses:Food\n" })).unwrap();
        let saved = update_file_content(state, reload, Base64Path("more.zhang".to_owned()), axum::extract::Json(request))
            .await
            .into_response();
        assert_eq!(
            answer(saved).await,
            (403, "{\"message\":\"the storage refused to write more.zhang\"}".to_owned())
        );
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
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
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
        let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
        let request = axum::Json(FileUpdateRequest {
            content: "1970-01-01 open Assets:Cash\n".to_owned(),
            expected_sha256: None,
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
        assert_eq!(status, 403, "{}", body);
        let message = body["message"].as_str().unwrap_or_default();
        assert!(message.contains("main.zhang"), "{}", body);
    }

    /// `zhang serve` given the root through a symlink, as `/tmp` is on macOS, lists the files it loads by their
    /// canonical path, the one the watcher compares with the paths the filesystem reports: an edit reloads (#492)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_local_root_through_a_symlink_lists_its_files_canonically() {
        use zhang_server::ServeConfig;

        let dir = tempdir().unwrap();
        let ledger_dir = dir.path().join("ledger");
        std::fs::create_dir(&ledger_dir).unwrap();
        std::fs::write(ledger_dir.join("main.zhang"), OPENS).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&ledger_dir, &link).unwrap();
        let mut opts = ServerOpts {
            path: link.clone(),
            endpoint: "main.zhang".to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: None,
            source: None,
            no_report: true,
        };
        let source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
        let mut config = ServeConfig {
            path: opts.path,
            endpoint: opts.endpoint,
            addr: opts.addr,
            port: opts.port,
            no_report: true,
            data_source: source,
            auth_credential: None,
            passkey_secret: None,
            passkey_rp_id: None,
            passkey_origin: None,
            session_secret: None,
        };
        let ledger = zhang_server::load_served_ledger(&mut config).await.unwrap();

        let canonical = ledger_dir.canonicalize().unwrap();
        assert_eq!(ledger.entry.0, canonical, "the ledger is served from the canonical root");
        assert_eq!(config.path, canonical, "the server is configured with the canonical root");
        assert_eq!(
            ledger.visited_files,
            vec![canonical.join("main.zhang")],
            "the files loaded are listed canonically"
        );
        assert!(
            ledger.data_source.async_get("main.zhang".to_owned()).await.is_ok(),
            "the files are still read relative to the root"
        );
    }
}
