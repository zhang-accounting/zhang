use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use chrono::Datelike;
use log::{debug, warn};
use minijinja::{context, Environment};
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};

use crate::data_type::{document_path_in_file, is_beancount_endpoint, DataType};
use crate::error::IoErrorIntoZhangError;
use crate::ledger::Ledger;
use crate::utils::{has_path_visited, BOM};
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

    /// The content of the file at `path`, relative to the ledger root and written with `/`, or `None` when there is
    /// no file there. [`ZhangError::ReadRefused`] when the source refuses to read it, which tells nothing of whether it
    /// is there. [`DataSource::async_get`] reads a missing file as empty on some sources, to append to it.
    async fn async_get_existing(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        match self.async_get(path.clone()).await {
            Ok(content) => Ok(Some(content)),
            Err(ZhangError::FileNotFound) => Ok(None),
            Err(ZhangError::IoError(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(ZhangError::IoError(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => Err(ZhangError::ReadRefused(path)),
            Err(error) => Err(error),
        }
    }

    /// The text of the file at `path`, to edit the directives at `spans` in place: each of them must still be
    /// what the ledger loaded there ([`SpanInfo::content`]). [`ZhangError::FileChanged`] when one is not, the file
    /// having changed since the ledger was loaded: the ledger must be reloaded for places that are not stale. A
    /// writer holds the ledger exclusively from this read until it saved the file, so no other write comes between
    async fn async_get_unchanged(&self, path: String, spans: &[SpanInfo]) -> ZhangResult<FileText> {
        let content = FileText::new(String::from_utf8(self.async_get(path.clone()).await?)?);
        unchanged(&path, &content.text, spans)?;
        Ok(content)
    }
    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append(ledger, directives)
    }

    async fn async_save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save(ledger, path, content)
    }
}

/// the file an `include` directive names, as it is written, for a data source to load next; `None` for any other directive
pub fn included_file(directive: &Spanned<Directive>) -> Option<String> {
    match &directive.data {
        Directive::Include(include) => Some(include.file.clone().to_plain_string()),
        _ => None,
    }
}

/// the file a new `directive` is appended to: the one the `directive_output_path` option renders for its date, in the
/// ledger's directory, or the main file for a directive without a date. The template gets `type` (the kind of
/// directive), `year`, `month`, `month_str` and `day`, `day_str` (zero-padded) and `ext`, the extension of the main
/// file (`zhang` without one)
pub fn directive_output_file(ledger: &Ledger, directive: &Directive) -> ZhangResult<PathBuf> {
    let (entry, main_file_endpoint) = &ledger.entry;
    let Some(datetime) = directive.datetime() else {
        return Ok(entry.join(main_file_endpoint));
    };
    let date = datetime.date();
    let mut env = Environment::new();
    env.add_template("directive_output_path", &ledger.options.directive_output_path).map_err(|e| {
        warn!("{}", e);
        ZhangError::InvalidOptionValue
    })?;
    let template = env.get_template("directive_output_path").map_err(|e| {
        warn!("{}", e);
        ZhangError::InvalidOptionValue
    })?;
    let path = template
        .render(context! {
            type => directive.directive_type().to_string(),
            year => date.year(),
            month => date.month(),
            month_str => date.format("%m").to_string(),
            day => date.day(),
            day_str => date.format("%d").to_string(),
            ext => Path::new(main_file_endpoint).extension().and_then(|it| it.to_str()).unwrap_or("zhang"),
        })
        .map_err(|_| ZhangError::InvalidOptionValue)?;
    Ok(entry.join(path))
}

/// Include a new output file once per append batch. Files already loaded by the ledger or
/// included earlier in the batch need no new directive. The path is relative to the ledger root
/// when possible, as both local and remote sources write it into the main file.
pub fn include_for_append(ledger: &Ledger, endpoint: &PathBuf, included: &mut Vec<PathBuf>) -> Option<Directive> {
    if has_path_visited(&ledger.visited_files, endpoint) || has_path_visited(included.iter(), endpoint) {
        return None;
    }
    included.push(endpoint.clone());
    let path = endpoint.strip_prefix(&ledger.entry.0).unwrap_or(endpoint);
    Some(Directive::Include(Include {
        file: ZhangString::QuoteString(path.to_str().unwrap().to_owned()),
    }))
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

/// The text of a file of the ledger, read to edit it in place: the file's content after the byte order mark it may
/// start with ([`BOM`]), which the parsers skip too, so the spans of the ledger index the text; the mark is written
/// back before it ([`FileText::into_bytes`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    /// whether the file starts with a byte order mark
    pub bom: bool,
    /// the text after the mark
    pub text: String,
}

impl FileText {
    /// the text of a file whose content is `content`
    pub fn new(mut content: String) -> Self {
        let bom = content.starts_with(BOM);
        if bom {
            content.drain(..BOM.len());
        }
        FileText { bom, text: content }
    }

    /// the content to write back: the mark the file had, then the text
    pub fn into_bytes(self) -> Vec<u8> {
        match self.bom {
            true => [BOM.as_bytes(), self.text.as_bytes()].concat(),
            false => self.text.into_bytes(),
        }
    }
}

/// whether the directives at `spans` are still what the ledger loaded in `content`, the text of the file at `path`
pub fn unchanged(path: &str, content: &str, spans: &[SpanInfo]) -> ZhangResult<()> {
    match spans.iter().all(|span| content.get(span.start..span.end) == Some(span.content.as_str())) {
        true => Ok(()),
        false => Err(ZhangError::FileChanged(path.to_owned())),
    }
}

/// The file of `ledger` the directive at `span` was read from, named as [`DataSource::async_get`] takes it: where
/// the directive is edited in place. `None` when `span` is no place in a file of the ledger: it names no file, or
/// one the ledger did not load, or it holds no text. That is the span of a directive a plugin made, which is in no
/// file; the text at its span, if there is any there, is some other directive's, and must not be edited as it.
///
/// Whether the text there is still what the ledger loaded is [`DataSource::async_get_unchanged`]'s to check.
pub fn loaded_file(ledger: &Ledger, span: &SpanInfo) -> Option<String> {
    let file = span.filename.as_ref()?;
    if span.start >= span.end || span.content.is_empty() {
        return None;
    }
    // a local source names a file by its full path, a remote one by its path within the ledger
    let entry = &ledger.entry.0;
    let loaded = ledger
        .visited_files
        .iter()
        .any(|visited| visited == file || visited.strip_prefix(entry).is_ok_and(|within| within == file));
    loaded.then(|| file.to_string_lossy().to_string())
}

/// An `include` path with `*` in it: a pattern naming every file that matches, as beancount's `include` does. `*`
/// stands for any run of characters other than `/` within one part of the path, in any part and any number of times;
/// every other character is literal, and a part matches a whole name. A leading `*` does not match a hidden name, one
/// starting with `.`, as in a shell. The last part names files, the parts before it directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludePattern {
    /// whether the pattern starts at the root of the file system (or of the data source)
    absolute: bool,
    /// the parts of the path, `.` left out and `..` folded into the literal part before it when there is one
    parts: Vec<String>,
}

impl IncludePattern {
    /// the pattern `path` is, or `None` when it has no `*` and names one file
    pub fn parse(path: &Path) -> Option<IncludePattern> {
        if !path.to_string_lossy().contains('*') {
            return None;
        }
        let mut absolute = false;
        let mut parts: Vec<String> = vec![];
        for component in path.components() {
            match component {
                Component::RootDir | Component::Prefix(_) => absolute = true,
                Component::CurDir => {}
                Component::ParentDir => match parts.last() {
                    Some(last) if last != ".." && !last.contains('*') => {
                        parts.pop();
                    }
                    _ => parts.push("..".to_owned()),
                },
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            }
        }
        Some(IncludePattern { absolute, parts })
    }

    /// the files the pattern names under `base`, which an absolute pattern ignores, in the order of their names
    /// within each directory. `list` gives the entries of a directory, none for one that is not there. The parts
    /// before the first `*` are joined without a look; from there on, a part names only entries the listing holds, so
    /// `data/*/accounts.zhang` names the `accounts.zhang` files that exist
    pub fn expand(&self, base: &Path, mut list: impl FnMut(&Path) -> ZhangResult<Vec<SourceEntry>>) -> ZhangResult<Vec<PathBuf>> {
        let mut paths = vec![if self.absolute { PathBuf::from("/") } else { base.to_path_buf() }];
        let mut listing = false;
        for (index, part) in self.parts.iter().enumerate() {
            let is_last = index + 1 == self.parts.len();
            listing |= part.contains('*');
            if !listing {
                paths = paths.into_iter().map(|path| path.join(part)).collect();
                continue;
            }
            let mut matched = vec![];
            for path in paths {
                let mut entries = list(&path)?;
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                for entry in entries {
                    if entry.is_dir != is_last && segment_matches(part, &entry.name) {
                        matched.push(path.join(&entry.name));
                    }
                }
            }
            paths = matched;
        }
        Ok(paths)
    }
}

/// whether `name`, one part of a path, is what `pattern`, one part of an [`IncludePattern`], names: `*` stands for any
/// run of characters, a leading one for a run that does not start with `.`, and the rest is literal and whole
pub fn segment_matches(pattern: &str, name: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else {
        // no `*`: the whole name
        return rest.is_empty();
    };
    if first.is_empty() && name.starts_with('.') {
        return false;
    }
    for piece in middle {
        match rest.find(piece) {
            Some(at) => rest = &rest[at + piece.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
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
    pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) -> ZhangResult<()> {
        match filename.parent() {
            Some(folder) => std::fs::create_dir_all(folder).with_path(folder),
            None => Ok(()),
        }
    }

    /// the entries of the directory at `dir` on the local disk, none when there is no directory there
    fn entries(dir: &Path) -> ZhangResult<Vec<SourceEntry>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => return Ok(vec![]),
            Err(e) => return Err(e).with_path(dir),
        };
        entries
            .map(|entry| {
                let entry = entry?;
                Ok(SourceEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_dir: entry.path().is_dir(),
                })
            })
            .collect::<Result<Vec<_>, std::io::Error>>()
            .with_path(dir)
    }

    /// append `directive` to `file`, or to the file `directive_output_path` gives it ([`directive_output_file`]), which
    /// the main file then includes unless the ledger or this append (`included`) has it already. Without `included`, no
    /// include
    fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, included: Option<&mut Vec<PathBuf>>) -> ZhangResult<()> {
        let (entry, _) = &ledger.entry;

        let endpoint = match file {
            Some(file) => file,
            None => directive_output_file(ledger, &directive)?,
        };

        LocalFileSystemDataSource::create_folder_if_not_exist(&endpoint)?;

        if let Some(include) = included.and_then(|included| include_for_append(ledger, &endpoint, included)) {
            self.append_directive(ledger, include, None, None)?;
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
            if let Some(pattern) = IncludePattern::parse(&pathbuf) {
                load_queue.extend(pattern.expand(&entry, Self::entries)?);
                continue;
            }
            debug!("visited entry file: {:?}", pathbuf.display());

            if has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get(pathbuf.to_string_lossy().to_string())?;
            let entity_directives = self
                .data_type
                .transform(String::from_utf8_lossy(&file_content).to_string(), Some(pathbuf.to_string_lossy().to_string()))?;

            entity_directives.iter().filter_map(included_file).for_each(|buf| {
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
mod get_existing_test {
    use super::DataSource;
    use crate::{ZhangError, ZhangResult};

    /// a source answering each path in its own way
    struct Answering;

    impl DataSource for Answering {
        fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
            match path.as_str() {
                "here.pdf" => Ok(b"content".to_vec()),
                "empty.pdf" => Ok(vec![]),
                "gone.pdf" => Err(ZhangError::FileNotFound),
                "gone too.pdf" => Err(ZhangError::IoError(std::io::Error::from(std::io::ErrorKind::NotFound))),
                "denied.pdf" => Err(ZhangError::IoError(std::io::Error::from(std::io::ErrorKind::PermissionDenied))),
                _ => Err(ZhangError::CustomError("the source failed".to_owned())),
            }
        }
    }

    /// the output of `future`, which is ready at once
    fn ready<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => panic!("the future is not ready"),
        }
    }

    /// A file is missing only when the source says it is not there; a refusal to read it is a refusal, and any other
    /// error stays an error.
    #[test]
    fn a_file_is_missing_only_when_the_source_says_so() {
        let get = |path: &str| ready(Answering.async_get_existing(path.to_owned()));
        assert_eq!(get("here.pdf").unwrap(), Some(b"content".to_vec()));
        assert_eq!(get("empty.pdf").unwrap(), Some(vec![]));
        assert_eq!(get("gone.pdf").unwrap(), None);
        assert_eq!(get("gone too.pdf").unwrap(), None);
        assert!(matches!(get("denied.pdf"), Err(ZhangError::ReadRefused(path)) if path == "denied.pdf"));
        assert!(matches!(get("broken.pdf"), Err(ZhangError::CustomError(_))));
    }
}

#[cfg(test)]
mod include_pattern_test {
    use std::path::Path;

    use super::{segment_matches, IncludePattern, SourceEntry};
    use crate::ZhangResult;

    /// A part matches a whole name: `*` is any run of characters, a leading `*` none starting with `.`, the rest is
    /// literal, `.` included.
    #[test]
    fn a_part_matches_whole_names() {
        for (pattern, name, matches) in [
            ("*.zhang", "01.zhang", true),
            ("*.zhang", "01.zhang.bak", false),
            ("*.zhang", ".#01.zhang", false),
            (".*", ".hidden", true),
            ("*", "anything", true),
            ("2024-*-*.zhang", "2024-01-15.zhang", true),
            ("2024-*-*.zhang", "2024-0115.zhang", false),
            ("report(*).zhang", "report(1).zhang", true),
            ("report(*).zhang", "report1.zhang", false),
            ("a*b*c", "abc", true),
            ("a*b*c", "axxbyyc", true),
            ("a*b*c", "ac", false),
            ("*ab*b", "ab", false),
            ("accounts.zhang", "accounts.zhang", true),
            ("accounts.zhang", "accounts.zhang.bak", false),
            ("x.zhang", "x_zhang", false),
        ] {
            assert_eq!(segment_matches(pattern, name), matches, "{pattern} against {name}");
        }
    }

    /// A pattern is the parts of its path: a path without `*` is none, `.` is dropped, `..` folds into the literal
    /// part before it, and a leading `/` makes it absolute.
    #[test]
    fn a_pattern_is_the_parts_of_its_path() {
        assert_eq!(IncludePattern::parse(Path::new("data/2024.zhang")), None);
        let pattern = IncludePattern::parse(Path::new("./data/2024/../*/x*.zhang")).unwrap();
        assert_eq!(
            (pattern.absolute, pattern.parts),
            (false, vec!["data".to_owned(), "*".to_owned(), "x*.zhang".to_owned()])
        );
        let pattern = IncludePattern::parse(Path::new("/ledger/*.zhang")).unwrap();
        assert_eq!((pattern.absolute, pattern.parts), (true, vec!["ledger".to_owned(), "*.zhang".to_owned()]));
        assert_eq!(
            IncludePattern::parse(Path::new("../*.zhang")).unwrap().parts,
            vec!["..".to_owned(), "*.zhang".to_owned()]
        );
    }

    /// Over a listing, the last part names files and the parts before it directories, in name order; a part without
    /// `*` after one with it names only what exists; a directory that is not there lists nothing.
    #[test]
    fn a_pattern_names_the_files_a_listing_holds() {
        let tree: &[(&str, &[(&str, bool)])] = &[
            (
                "",
                &[
                    ("data", true),
                    ("main.zhang", false),
                    ("accounts.zhang", false),
                    ("notes.txt", false),
                    (".#accounts.zhang", false),
                ],
            ),
            ("data", &[("2025", true), ("2024", true), ("readme.zhang", false)]),
            (
                "data/2024",
                &[
                    ("b.zhang", false),
                    ("a.zhang", false),
                    ("a.zhang.bak", false),
                    ("accounts.zhang", false),
                    ("old", true),
                ],
            ),
            ("data/2025", &[("c.zhang", false)]),
        ];
        let list = |dir: &Path| -> ZhangResult<Vec<SourceEntry>> {
            let dir = dir.to_string_lossy();
            Ok(tree
                .iter()
                .find(|(path, _)| *path == dir)
                .map(|(_, entries)| {
                    entries
                        .iter()
                        .map(|(name, is_dir)| SourceEntry {
                            name: name.to_string(),
                            is_dir: *is_dir,
                        })
                        .collect()
                })
                .unwrap_or_default())
        };
        let expand = |pattern: &str| -> Vec<String> {
            IncludePattern::parse(Path::new(pattern))
                .unwrap()
                .expand(Path::new(""), list)
                .unwrap()
                .iter()
                .map(|it| it.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(expand("*.zhang"), vec!["accounts.zhang", "main.zhang"]);
        assert_eq!(
            expand("data/*/*.zhang"),
            vec!["data/2024/a.zhang", "data/2024/accounts.zhang", "data/2024/b.zhang", "data/2025/c.zhang"]
        );
        assert_eq!(expand("data/*/accounts.zhang"), vec!["data/2024/accounts.zhang"]);
        assert_eq!(expand("data/*"), vec!["data/readme.zhang"]);
        assert_eq!(expand("data/20*/a.zhang"), vec!["data/2024/a.zhang"]);
        assert_eq!(expand("missing/*.zhang"), Vec::<String>::new());
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
        assert_eq!(main.matches("include \"data/2024/01.zhang\"").count(), 1, "{main}");
        let reloaded = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        let store = reloaded.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 2);
    }

    /// A file starting with a UTF-8 byte order mark, as some Windows editors save it, loads (#505): the mark is
    /// skipped, and the spans index the text after it, which the write paths slice. An append keeps the mark, once.
    #[test]
    fn a_file_starting_with_a_byte_order_mark_loads_and_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.zhang");
        let opens = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";
        std::fs::write(&main, format!("\u{feff}{opens}")).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source.clone()).unwrap();

        assert!(ledger.store.read().unwrap().errors.is_empty(), "{:?}", ledger.store.read().unwrap().errors);
        let spans = ledger.directives.iter().map(|it| &it.span).collect::<Vec<_>>();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].start, 0, "the first directive starts where the text after the mark does");
        for span in spans {
            assert_eq!(opens.get(span.start..span.end), Some(span.content.as_str()), "{span:?}");
        }

        let coffee: Vec<Directive> = ZhangDataType {}
            .transform("2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n".to_owned(), None)
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect();
        ledger.data_source.append(&ledger, coffee).unwrap();

        let written = std::fs::read_to_string(&main).unwrap();
        assert!(written.starts_with(&format!("\u{feff}{opens}")), "{written:?}");
        assert_eq!(written.matches('\u{feff}').count(), 1, "{written:?}");
        assert_eq!(written.matches("include \"data/2024/01.zhang\"").count(), 1, "{written}");
        assert!(
            !std::fs::read_to_string(dir.path().join("data/2024/01.zhang")).unwrap().contains('\u{feff}'),
            "a new file gets no mark"
        );
        let reloaded = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        let store = reloaded.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 1);
    }

    /// A pattern names the files that exist, whole names only, in any part of the path (#494): `*.zhang` next to the
    /// main file, `data/*/*.zhang`, and `data/*/accounts.zhang` with a literal last part. The main file matches its
    /// own pattern and is still read once; `01.zhang.bak` and the hidden `.#01.zhang` are left out.
    #[test]
    fn an_include_pattern_names_matching_files_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let write = |path: &str, content: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        };
        let transaction = |day: &str| format!("2024-01-{day} * \"shop\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n");
        write(
            "main.zhang",
            "include \"*.zhang\"\ninclude \"data/*/*.zhang\"\ninclude \"data/*/accounts.zhang\"\ninclude \"nothing/*.zhang\"\n",
        );
        write("accounts.zhang", "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n");
        write("data/2024/01.zhang", &transaction("01"));
        write("data/2024/01.zhang.bak", &transaction("02"));
        write("data/2024/.#01.zhang", &transaction("03"));
        write("data/2025/02.zhang", &transaction("04"));
        write("data/2025/accounts.zhang", "1970-01-01 open Assets:Bank\n");
        write("data/other.zhang", &transaction("05"));
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();

        let mut visited: Vec<String> = ledger
            .visited_files
            .iter()
            .map(|it| it.strip_prefix(&root).unwrap().to_string_lossy().into_owned())
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
}
