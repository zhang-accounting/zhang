use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Datelike;
use log::debug;
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};

use crate::data_type::{document_path_in_file, is_beancount_endpoint, DataType};
use crate::error::IoErrorIntoZhangError;
use crate::ledger::Ledger;
use crate::utils::has_path_visited;
use crate::{ZhangError, ZhangResult};

/// `DataSource` is the protocol to describe how the `DataType` be stored and be transformed into standard directives.
/// The Data Source have two capabilities:
/// - given the endpoint, `DataSource` need to retrieve the raw data from source and feed it to associated `DataType` and get the directives from `DataType` processor.
/// - given the directive, `DataSource` need to update or insert the given directive into source, which is the place where the raw data is stored.
#[async_trait::async_trait]
pub trait DataSource
where
    Self: Send + Sync,
{
    // used to export directive into u8 sequence, if the datasource support
    fn export(&self, _directive: Directive) -> ZhangResult<Vec<u8>> {
        unimplemented!()
    }
    fn get(&self, _path: String) -> ZhangResult<Vec<u8>> {
        unimplemented!()
    }

    /// The directory on the local disk holding the ledger whose root is `entry`, when this source reads the local
    /// disk; `None` (the default) for a remote source.
    ///
    /// Plugins read local files through it with a capability handle, so a symlink cannot lead them outside what
    /// they were granted. On a remote source they read through [`DataSource::get_limited`] and
    /// [`DataSource::list`] instead.
    fn local_root(&self, _entry: &Path) -> Option<PathBuf> {
        None
    }

    /// The content of the file at `path`, relative to the ledger root and written with `/`, when it holds at most
    /// `max_len` bytes; [`ZhangError::TooLarge`] when it holds more, and [`ZhangError::FileNotFound`] when it does
    /// not exist.
    ///
    /// The default reads the whole file with [`DataSource::get`] before checking its size. A remote source should
    /// check the size before downloading, stop reading past `max_len`, and give up after a while: a plugin waits
    /// for this call, and its own timeout cannot interrupt it.
    fn get_limited(&self, path: String, max_len: u64) -> ZhangResult<Vec<u8>> {
        let content = self.get(path.clone())?;
        if content.len() as u64 > max_len {
            return Err(ZhangError::TooLarge(format!("the file {path:?} holds more than {max_len} bytes")));
        }
        Ok(content)
    }

    /// The entries of the directory at `path`, relative to the ledger root and written with `/` (empty for the root
    /// itself), in any order; [`ZhangError::TooLarge`] when it has more than `max_entries`, which a source should
    /// detect without listing them all. The default is [`ZhangError::Unsupported`]: a source that cannot list
    /// directories keeps it.
    fn list(&self, path: String, _max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
        Err(ZhangError::Unsupported(format!("listing the directory {path:?}")))
    }

    /// The paths of the files in the directory `dir` (relative to the ledger root and written with `/`, `""` for the
    /// root itself), and in its sub-directories too when `recursive`: relative to the root, written with `/`. `None`
    /// (the default) when this source cannot tell: it cannot list, the listing fails, or it holds more than
    /// `max_files` files, or takes longer than `timeout`. A source on the local disk is not asked: [`has_file`] looks
    /// at [`DataSource::local_root`] instead.
    fn files_in(&self, _dir: String, _recursive: bool, _max_files: usize, _timeout: Duration) -> Option<HashSet<String>> {
        None
    }

    fn load(&self, _entry: String, _endpoint: String) -> ZhangResult<LoadResult> {
        unimplemented!()
    }

    fn save(&self, _ledger: &Ledger, _path: String, _content: &[u8]) -> ZhangResult<()> {
        unimplemented!()
    }

    fn append(&self, _ledger: &Ledger, _directives: Vec<Directive>) -> ZhangResult<()> {
        unimplemented!()
    }

    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        self.load(entry, endpoint)
    }

    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.get(path)
    }

    /// The content of the file at `path`, to edit the directives at `spans` in place: each of them must still be
    /// what the ledger loaded there ([`SpanInfo::content`]). [`ZhangError::FileChanged`] when one is not, the file
    /// having changed since the ledger was loaded: the ledger must be reloaded for places that are not stale. A
    /// writer holds the ledger exclusively from this read until it saved the file, so no other write comes between
    async fn async_get_unchanged(&self, path: String, spans: &[SpanInfo]) -> ZhangResult<String> {
        let content = String::from_utf8(self.async_get(path.clone()).await?)?;
        unchanged(&path, &content, spans)?;
        Ok(content)
    }
    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append(ledger, directives)
    }

    async fn async_save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save(ledger, path, content)
    }
}

/// `directive` as it is written into `file`, a file of `ledger` named by its path within it. In a beancount ledger, the
/// path of a `document`, within the ledger, is written relative to the directory of that file, as beancount reads it
pub fn written_into(ledger: &Ledger, directive: Directive, file: &Path) -> Directive {
    match directive {
        Directive::Document(mut document) if is_beancount_endpoint(&ledger.entry.1) => {
            let path = document.filename.clone().to_plain_string();
            document.filename = ZhangString::QuoteString(document_path_in_file(&path, file));
            Directive::Document(document)
        }
        directive => directive,
    }
}

/// The longest the listings of a remote source may take in all, for one load: past it, whether a file exists is not
/// known
pub const LISTING_BUDGET: Duration = Duration::from_secs(20);

/// The most files a listing of a remote source reads: a directory holding more is not known
pub const LISTING_MAX_FILES: usize = 100_000;

/// What a load found of the files of a remote source, so it tells whether a file exists without asking the source file
/// by file. The first file asked for in a directory at the ledger's root lists that directory once, its
/// sub-directories too; a file at the root lists the root once, without them. A source lists a directory at the cost
/// of a request per page of files, whatever their depth, so a load with many documents costs a few requests. All the
/// listings of a load share one budget of time, [`LISTING_BUDGET`]: past it, or when a listing fails, whether a file in
/// a directory not listed yet exists is not known.
pub struct ListedFiles {
    /// the files of each directory at the root listed (`""` for the root), `None` for one the source could not list
    listed: HashMap<String, Option<HashSet<String>>>,
    /// how long the listings took
    spent: Duration,
    budget: Duration,
}

impl Default for ListedFiles {
    fn default() -> Self {
        ListedFiles {
            listed: HashMap::new(),
            spent: Duration::ZERO,
            budget: LISTING_BUDGET,
        }
    }
}

impl ListedFiles {
    /// Whether there is a file at `path`, within the ledger; `None` when it is not known. `list` lists a directory
    /// (within the ledger, recursively or not) in the time it is given, as [`DataSource::files_in`].
    fn has(&mut self, path: &str, list: impl FnOnce(String, bool, Duration) -> Option<HashSet<String>>) -> Option<bool> {
        let parts = Path::new(path)
            .components()
            .map(|it| match it {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                // absolute, or out of the ledger: not listed
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let (dir, recursive) = match parts.len() {
            0 => return None,
            1 => (String::new(), false),
            _ => (parts[0].clone(), true),
        };
        if !self.listed.contains_key(&dir) {
            let left = self.budget.saturating_sub(self.spent);
            let listed = match left.is_zero() {
                true => None,
                false => {
                    let started = Instant::now();
                    let listed = list(dir.clone(), recursive, left);
                    self.spent += started.elapsed();
                    listed
                }
            };
            self.listed.insert(dir.clone(), listed);
        }
        self.listed[&dir].as_ref().map(|listed| listed.contains(&parts.join("/")))
    }
}

/// Whether `ledger` has a file at `path`, within it or absolute; `None` when its source cannot tell. On the local disk,
/// the file is looked at; a remote source is asked through the listings of the load ([`ListedFiles`]).
pub fn has_file(ledger: &mut Ledger, path: &str) -> Option<bool> {
    if let Some(root) = ledger.data_source.local_root(&ledger.entry.0) {
        return Some(root.join(path).is_file());
    }
    let data_source = ledger.data_source.clone();
    ledger
        .listed_files
        .has(path, |dir, recursive, timeout| data_source.files_in(dir, recursive, LISTING_MAX_FILES, timeout))
}

/// whether the directives at `spans` are still what the ledger loaded in `content`, the content of the file at `path`
pub fn unchanged(path: &str, content: &str, spans: &[SpanInfo]) -> ZhangResult<()> {
    match spans.iter().all(|span| content.get(span.start..span.end) == Some(span.content.as_str())) {
        true => Ok(()),
        false => Err(ZhangError::FileChanged(path.to_owned())),
    }
}

/// `LocalFileSystemDataSource` is the data source that store the data in the local file system.
///
/// # Warning
/// This data source is not fully tested yet and may contain bugs. Use with caution.
///
pub struct LocalFileSystemDataSource {
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
}

impl LocalFileSystemDataSource {
    pub fn new<DT: DataType<Carrier = String> + Send + Sync + 'static>(data_type: DT) -> Self {
        LocalFileSystemDataSource {
            data_type: Box::new(data_type),
        }
    }
    fn go_next(&self, directive: &Spanned<Directive>) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }

    pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) -> ZhangResult<()> {
        match filename.parent() {
            Some(folder) => std::fs::create_dir_all(folder).with_path(folder),
            None => Ok(()),
        }
    }

    /// append `directive` to `file`, or to the file of its month, which the main file then includes unless the ledger
    /// or this append (`included`) has it already. Without `included`, no include
    fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, included: Option<&mut Vec<PathBuf>>) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;

        let endpoint = file.unwrap_or_else(|| {
            if let Some(datetime) = directive.datetime() {
                entry.join(PathBuf::from(format!("data/{}/{}.zhang", datetime.year(), datetime.month())))
            } else {
                entry.join(main_file_endpoint)
            }
        });

        LocalFileSystemDataSource::create_folder_if_not_exist(&endpoint)?;

        // a file new to the ledger and to this append
        let new_file = included.filter(|included| !has_path_visited(&ledger.visited_files, &endpoint) && !has_path_visited(included.iter(), &endpoint));
        if let Some(included) = new_file {
            included.push(endpoint.clone());
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
                None,
            )?;
        }

        let content = match ledger.data_source.get(endpoint.to_string_lossy().to_string()) {
            Ok(content) => String::from_utf8(content)?,
            // a file this append creates
            Err(ZhangError::IoError(e)) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };

        let directive = written_into(ledger, directive, endpoint.strip_prefix(entry).unwrap_or(&endpoint));
        let appended_content = format!("{}\n{}\n", content, self.data_type.export(Spanned::new(directive, SpanInfo::default())));

        ledger
            .data_source
            .save(ledger, endpoint.to_string_lossy().to_string(), appended_content.as_bytes())?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl DataSource for LocalFileSystemDataSource {
    fn export(&self, directive: Directive) -> ZhangResult<Vec<u8>> {
        Ok(self.data_type.export(Spanned::new(directive, SpanInfo::default())).into_bytes())
    }

    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        Ok(std::fs::read(PathBuf::from(path))?)
    }

    /// the ledger root itself: this source reads the paths it is given from the local disk
    fn local_root(&self, entry: &Path) -> Option<PathBuf> {
        Some(entry.to_path_buf())
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let entry = PathBuf::from(entry);
        let entry = entry.canonicalize().with_path(&entry)?;
        let main_endpoint = entry.join(endpoint);
        let main_endpoint = main_endpoint.canonicalize().with_path(&main_endpoint)?;

        let mut load_queue: VecDeque<PathBuf> = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited: Vec<PathBuf> = Vec::new();
        let mut directives = vec![];
        while let Some(pathbuf) = load_queue.pop_front() {
            debug!("visited entry file: {:?}", pathbuf.display());

            if has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get(pathbuf.to_string_lossy().to_string())?;
            let entity_directives = self
                .data_type
                .transform(String::from_utf8_lossy(&file_content).to_string(), Some(pathbuf.to_string_lossy().to_string()))?;

            entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                let fullpath = if buf.starts_with('/') {
                    PathBuf::from(&buf)
                } else {
                    pathbuf.parent().map(|it| it.join(buf)).unwrap()
                };
                load_queue.push_back(fullpath);
            });
            directives.extend(entity_directives);
            visited.push(pathbuf);
        }
        Ok(LoadResult {
            directives,
            visited_files: visited,
        })
    }

    fn save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        std::fs::write(&path, content).with_path(PathBuf::from(path).as_path())
    }

    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        // the files this append includes: the ledger it was given does not know them yet
        let mut included = vec![];
        for directive in directives {
            self.append_directive(ledger, directive, None, Some(&mut included))?;
        }
        Ok(())
    }
}

/// an entry of a directory, as [`DataSource::list`] lists it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    /// the entry's name, without the directory's path or a trailing `/`
    pub name: String,
    /// whether the entry is a directory; otherwise it is a file
    pub is_dir: bool,
}

pub struct LoadResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}

#[cfg(test)]
mod listed_files_test {
    use std::collections::HashSet;
    use std::time::Duration;

    use super::ListedFiles;

    fn files(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|it| it.to_string()).collect()
    }

    #[test]
    fn a_directory_at_the_root_is_listed_once_for_all_its_files() {
        let mut listed = ListedFiles::default();
        let mut calls = vec![];
        let mut has = |path: &str| {
            listed.has(path, |dir, recursive, _| {
                calls.push((dir.clone(), recursive));
                match dir.as_str() {
                    "attachments" => Some(files(&["attachments/u1/a.pdf", "attachments/u2/b.pdf"])),
                    "" => Some(files(&["main.bean", "c.pdf"])),
                    // a listing that fails
                    _ => None,
                }
            })
        };
        assert_eq!(has("attachments/u1/a.pdf"), Some(true));
        assert_eq!(has("attachments/u2/b.pdf"), Some(true));
        assert_eq!(has("attachments/u3/c.pdf"), Some(false));
        assert_eq!(has("c.pdf"), Some(true));
        assert_eq!(has("d.pdf"), Some(false));
        assert_eq!(has("receipts/a.pdf"), None);
        assert_eq!(has("receipts/b.pdf"), None);
        // never listed
        assert_eq!(has("/srv/a.pdf"), None);
        assert_eq!(has("../a.pdf"), None);
        assert_eq!(
            calls,
            vec![("attachments".to_owned(), true), ("".to_owned(), false), ("receipts".to_owned(), true)]
        );
    }

    #[test]
    fn past_the_budget_no_directory_is_listed() {
        let mut listed = ListedFiles {
            budget: Duration::from_millis(50),
            ..ListedFiles::default()
        };
        let mut timeouts = vec![];
        let mut has = |path: &str| {
            listed.has(path, |_, _, timeout| {
                timeouts.push(timeout);
                // longer than the whole budget
                std::thread::sleep(Duration::from_millis(80));
                Some(files(&["a/x.pdf"]))
            })
        };
        assert_eq!(has("a/x.pdf"), Some(true));
        assert_eq!(has("b/x.pdf"), None);
        assert_eq!(has("c/x.pdf"), None);
        assert_eq!(has("a/y.pdf"), Some(false));
        assert_eq!(timeouts, vec![Duration::from_millis(50)]);
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use zhang_ast::Directive;

    use super::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;

    #[test]
    fn an_append_includes_each_new_file_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source.clone()).unwrap();
        let directives: Vec<Directive> = ZhangDataType {}
            .transform(
                "2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n2024-01-16 * \"tea\"\n  Assets:Cash -3 CNY\n  Expenses:Food\n".to_owned(),
                None,
            )
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect();

        ledger.data_source.append(&ledger, directives).unwrap();

        let main = std::fs::read_to_string(dir.path().join("main.zhang")).unwrap();
        assert_eq!(main.matches("include \"data/2024/1.zhang\"").count(), 1, "{main}");
        let reloaded = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        let store = reloaded.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 2);
    }
}
