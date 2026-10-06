use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use chrono::Datelike;
use log::{debug, warn};
use minijinja::{context, Environment};
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};

use crate::data_type::{document_path_in_file, DataType, Dialect};
use crate::error::{storage_error, Access, IoErrorIntoZhangError};
use crate::ledger::Ledger;
use crate::utils::{has_path_visited, BOM};
use crate::{ZhangError, ZhangResult};

/// `DataSource` is the protocol to describe how the `DataType` be stored and be transformed into standard directives.
/// The Data Source have two capabilities:
/// - given the endpoint, `DataSource` need to retrieve the raw data from source and feed it to associated `DataType` and get the directives from `DataType` processor.
/// - given the directive, `DataSource` need to update or insert the given directive into source, which is the place where the raw data is stored.
///
/// Every method is blocking: a load reads files and runs plugins, whose host functions read files too, all
/// synchronously. An async caller runs a load on a blocking thread.
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

    /// Append `directives` to the files of `ledger` ([`append_plan`]): every source appends alike, reading each file
    /// with [`DataSource::get`] and writing it back with [`DataSource::save`]
    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for (path, directive) in append_plan(ledger, directives)? {
            let content = appended(self.get(path.clone()), &path, &self.export(directive)?)?;
            self.save(ledger, path, &content)?;
        }
        Ok(())
    }

    /// The content of the file at `path`, relative to the ledger root and written with `/`, or `None` when there is
    /// no file there. [`ZhangError::ReadRefused`] when the source refuses to read it, which tells nothing of whether it
    /// is there. [`DataSource::get`] errs on a missing file instead ([`ZhangError::is_file_not_found`]).
    fn get_existing(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        let error = match self.get(path.clone()) {
            Ok(content) => return Ok(Some(content)),
            // an io error a source passed on as it is: read by the one mapping of storage errors
            Err(ZhangError::IoError(error)) => storage_error(&path, Access::Read, error.kind().into(), error),
            Err(error) => error,
        };
        match error {
            ZhangError::FileNotFound => Ok(None),
            error => Err(error),
        }
    }

    /// The text of the file at `path`, to edit the directives at `spans` in place: each of them must still be
    /// what the ledger loaded there ([`SpanInfo::content`]). [`ZhangError::FileChanged`] when one is not, the file
    /// having changed since the ledger was loaded: the ledger must be reloaded for places that are not stale. A
    /// writer holds the ledger exclusively from this read until it saved the file, so no other write comes between
    fn get_unchanged(&self, path: String, spans: &[SpanInfo]) -> ZhangResult<FileText> {
        let content = match self.get(path.clone()) {
            Ok(content) => FileText::decode(content, &path)?,
            // a file removed since the ledger was loaded changed too
            Err(error) if error.is_file_not_found() => return Err(ZhangError::FileChanged(path)),
            Err(error) => return Err(error),
        };
        unchanged(&path, &content.text, spans)?;
        Ok(content)
    }
}

/// A file a data source loads next: the main file, or the file or pattern an `include` names, with a relative path
/// resolved against the directory of the file holding the `include`. Every source queues the files of a load so, and
/// reports an `include` naming no file alike ([`PendingFile::missing`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFile {
    /// the file, or the pattern
    pub path: PathBuf,
    /// the `include` naming it, with its path as written; `None` for the main file
    include: Option<(SpanInfo, String)>,
}

impl PendingFile {
    /// the main file of a ledger, at `path`
    pub fn main(path: PathBuf) -> Self {
        PendingFile { path, include: None }
    }

    /// what the `include`s among `directives`, read from this file, name, in their order
    pub fn includes(&self, directives: &[Spanned<Directive>]) -> Vec<PendingFile> {
        directives
            .iter()
            .filter_map(|directive| match &directive.data {
                Directive::Include(include) => {
                    let written = include.file.clone().to_plain_string();
                    let path = match written.starts_with('/') {
                        true => PathBuf::from(&written),
                        false => self.path.parent().unwrap_or(Path::new("")).join(&written),
                    };
                    Some(PendingFile {
                        path,
                        include: Some((directive.span.clone(), written)),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// `file`, one of those the pattern of this one matches, named by the same `include`
    pub fn matched(&self, file: PathBuf) -> PendingFile {
        PendingFile {
            path: file,
            include: self.include.clone(),
        }
    }

    /// The `include` naming this file, when the source has no file there, or this pattern, when it matches no file:
    /// the load goes on without it, and reports it. `None` for the main file, which no `include` names: whether a
    /// load can go on without it is the source's to decide
    pub fn missing(&self) -> Option<MissingInclude> {
        let (span, path) = self.include.clone()?;
        let file = IncludePattern::parse(&self.path).is_none().then(|| self.path.clone());
        Some(MissingInclude { span, path, file })
    }
}

/// An `include` that names no file the data source has: there is no file at its path, or the source cannot read
/// there, or its pattern matches no file. The ledger loads without it, and reports it on the `include` as
/// [`ErrorKind::IncludeNotFound`](zhang_ast::error::ErrorKind::IncludeNotFound), as beancount does
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingInclude {
    /// the `include` directive
    pub span: SpanInfo,
    /// the path as the `include` writes it
    pub path: String,
    /// the file looked for, its path resolved; `None` for a pattern. Its creation makes the ledger stale
    pub file: Option<PathBuf>,
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

/// What an append of `directives` to `ledger` writes, in order: each directive as it is written there
/// ([`written_into`]), with the file it is appended to, by its path within the ledger as [`DataSource::get`] takes it.
/// That is the file [`directive_output_file`] gives it. The main file gets an `include` of that file first, once per
/// append, unless the ledger loaded it already. A file outside the ledger's directory is an error, and nothing is
/// written: a source writes nothing there, and an `include` of it would name no file of the ledger.
pub fn append_plan(ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<Vec<(String, Directive)>> {
    let within = |file: &Path| {
        ledger
            .path_in_ledger(file)
            .filter(|it| !it.as_os_str().is_empty())
            .ok_or_else(|| ZhangError::CustomError(format!("{} is not in the ledger's directory", file.display())))
    };
    let main = within(&ledger.entry.0.join(&ledger.entry.1))?;
    // the files the ledger loaded, and those this append includes
    let mut included: Vec<PathBuf> = ledger.visited_files.iter().filter_map(|it| ledger.path_in_ledger(it)).collect();
    let mut plan = vec![];
    for directive in directives {
        let file = within(&directive_output_file(ledger, &directive)?)?;
        if !included.contains(&file) {
            let include = Directive::Include(Include {
                file: ZhangString::QuoteString(slashed(&file)),
            });
            plan.push((slashed(&main), include));
            included.push(file.clone());
        }
        let directive = written_into(ledger, directive, &file);
        plan.push((slashed(&file), directive));
    }
    Ok(plan)
}

/// the content of the file at `path`, whose content is `existing` (none when it is not there), with `directive`
/// appended on a line of its own; the byte order mark the file may start with stays ([`FileText`])
fn appended(existing: ZhangResult<Vec<u8>>, path: &str, directive: &[u8]) -> ZhangResult<Vec<u8>> {
    let content = match existing {
        Ok(content) => FileText::decode(content, path)?,
        // a file this append creates
        Err(error) if error.is_file_not_found() => FileText::new(String::new()),
        Err(error) => return Err(error),
    };
    Ok([content.into_bytes().as_slice(), b"\n", directive, b"\n"].concat())
}

/// `path`, a file or directory of the ledger whose root is `root`, by its path within the ledger: the path a
/// [`DataSource`] reads and writes it by, and the spans of the directives of a file hold. Every place that names a
/// file of the ledger, or tells whether a path is one, asks here.
///
/// A path under the root is named relative to it; a relative path is within the ledger already. The path is
/// normalized lexically (`.` dropped, `..` taking out the part before it), so a file has one name however an
/// `include` spells it; the root itself is the empty path. `None` for a path outside the root: an absolute path not
/// under it, or one that climbs above it with `..`. Nothing is looked up on the disk: a root and the paths under it are
/// spelled alike, the root as the ledger was loaded from it.
pub fn path_in_ledger(root: &Path, path: &Path) -> Option<PathBuf> {
    let within = match path.strip_prefix(root) {
        Ok(within) => within,
        Err(_) if path.is_relative() => path,
        Err(_) => return None,
    };
    normalize_relative(within)
}

/// a relative `path` normalized lexically (no `.` or `..` components, no empty ones); `None` when it is absolute or
/// climbs above its start. Inputs a plugin's file functions record are cleaned the same way
pub(crate) fn normalize_relative(relative: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

/// a path within the ledger written with `/`, as a [`DataSource`] takes it and an `include` writes it
pub fn slashed(path: &Path) -> String {
    path.components().map(|it| it.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// `directive` as it is written into `file`, a file of `ledger` named by its path within it. In a beancount ledger, the
/// path of a `document`, within the ledger, is written relative to the directory of that file, as beancount reads it
pub fn written_into(ledger: &Ledger, directive: Directive, file: &Path) -> Directive {
    match directive {
        Directive::Document(mut document) if ledger.dialect == Dialect::Beancount => {
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
    /// The text of the file at `path`, whose bytes are `content`: every reader of a file of the ledger decodes it here,
    /// to load it, to edit it in place, to append to it or to show it in the editor. A file that is not UTF-8 text is
    /// [`ZhangError::InvalidUtf8`], naming the file and the line of the first byte that is not: it is never read with
    /// that byte replaced, which would load other text than the file holds and write it back so.
    pub fn decode(content: Vec<u8>, path: impl AsRef<Path>) -> ZhangResult<FileText> {
        match String::from_utf8(content) {
            Ok(content) => Ok(FileText::new(content)),
            Err(error) => {
                let valid = &error.as_bytes()[..error.utf8_error().valid_up_to()];
                Err(ZhangError::InvalidUtf8 {
                    path: path.as_ref().to_string_lossy().into_owned(),
                    line: valid.iter().filter(|it| **it == b'\n').count() + 1,
                })
            }
        }
    }

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

/// The file of `ledger` the directive at `span` was read from, named as [`DataSource::get`] takes it: where
/// the directive is edited in place. `None` when `span` is no place in a file of the ledger: it names no file, or
/// one the ledger did not load, or it holds no text. That is the span of a directive a plugin made, which is in no
/// file; the text at its span, if there is any there, is some other directive's, and must not be edited as it.
///
/// Whether the text there is still what the ledger loaded is [`DataSource::get_unchanged`]'s to check.
pub fn loaded_file(ledger: &Ledger, span: &SpanInfo) -> Option<String> {
    let file = span.filename.as_ref()?;
    if span.start >= span.end || span.content.is_empty() {
        return None;
    }
    let file = ledger.path_in_ledger(file)?;
    let loaded = ledger
        .visited_files
        .iter()
        .any(|visited| ledger.path_in_ledger(visited).as_ref() == Some(&file));
    loaded.then(|| slashed(&file))
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

/// The files of one load of a ledger, in the order a data source reads them: the main file, then the files named by
/// the `include`s of each file read, a pattern naming the files it matches. Every source loads through it, reading the
/// files and listing the directories it asks for, so a load follows the same rules whatever stores the ledger:
/// - a file is named by its path within the ledger ([`path_in_ledger`]): [`DataSource::get`] reads it by that name,
///   and the spans of its directives hold it. [`LoadResult::visited_files`] lists it joined onto the root;
/// - an `include` naming a file outside the ledger's root, or no file, is reported on it as `IncludeNotFound`, and the
///   rest of the ledger loads ([`MissingInclude`]);
/// - a main file that is not there is an empty ledger, which the web UI writes the first entries of (`zhang serve` on
///   a new folder);
/// - a file is read once, however many `include`s name it;
/// - a file that is not UTF-8 text stops the load, naming it ([`FileText::decode`]).
///
/// ```text
/// let mut files = LedgerFiles::new(root, main);
/// while let Some(file) = files.next(|dir| list(dir))? {
///     let content = read(file.path());
///     files.read(file, content, data_type)?;
/// }
/// let loaded = files.finish();
/// ```
pub struct LedgerFiles {
    /// the root of the ledger, as it was given
    root: PathBuf,
    queue: VecDeque<PendingFile>,
    visited: Vec<PathBuf>,
    directives: Vec<Spanned<Directive>>,
    missing_includes: Vec<MissingInclude>,
}

/// A file of a load, to read next ([`LedgerFiles::next`])
pub struct NextFile {
    pending: PendingFile,
    /// its path within the ledger
    within: PathBuf,
}

impl NextFile {
    /// the path of the file within the ledger, written with `/`, as [`DataSource::get`] takes it
    pub fn path(&self) -> String {
        slashed(&self.within)
    }
}

impl LedgerFiles {
    /// the load of the ledger whose root is `root` and whose main file is `main`, relative to the root
    pub fn new(root: impl Into<PathBuf>, main: &str) -> Self {
        let root = root.into();
        LedgerFiles {
            queue: VecDeque::from([PendingFile::main(root.join(main))]),
            root,
            visited: vec![],
            directives: vec![],
            missing_includes: vec![],
        }
    }

    /// The next file to read, or `None` when every file is read. A pattern is expanded first: `list` gives the entries
    /// of a directory of the ledger, by its path within it (empty for the root), none for one that is not there. An
    /// `include` of a path outside the ledger's root is reported as missing without a read, as every source holds
    /// nothing there; a main file outside it is an error
    pub fn next(&mut self, mut list: impl FnMut(&Path) -> ZhangResult<Vec<SourceEntry>>) -> ZhangResult<Option<NextFile>> {
        while let Some(mut pending) = self.queue.pop_front() {
            let Some(within) = path_in_ledger(&self.root, &pending.path) else {
                match pending.missing() {
                    Some(missing) => self.missing_includes.push(missing),
                    None => {
                        return Err(ZhangError::CustomError(format!(
                            "the main file {} is outside the ledger's directory {}",
                            pending.path.display(),
                            self.root.display()
                        )))
                    }
                }
                continue;
            };
            if let Some(pattern) = IncludePattern::parse(&within) {
                let files = pattern.expand(Path::new(""), &mut list)?;
                if files.is_empty() {
                    self.missing_includes.extend(pending.missing());
                }
                self.queue.extend(files.into_iter().map(|file| pending.matched(self.root.join(file))));
                continue;
            }
            pending.path = self.root.join(&within);
            if has_path_visited(&self.visited, &pending.path) {
                continue;
            }
            debug!("visited entry file: {:?}", pending.path.display());
            return Ok(Some(NextFile { pending, within }));
        }
        Ok(None)
    }

    /// What the source read for `file`: its content, or why it could not; a missing file is
    /// [`ZhangError::is_file_not_found`]. Its directives are read as `data_type` reads them
    pub fn read(&mut self, file: NextFile, content: ZhangResult<Vec<u8>>, data_type: &dyn DataType<Carrier = String>) -> ZhangResult<()> {
        let NextFile { pending, within } = file;
        let name = slashed(&within);
        let text = match content {
            // after the byte order mark it may start with, which the parsers skip too: the spans index the same text
            Ok(content) => FileText::decode(content, &name)?.text,
            Err(error) if error.is_file_not_found() => match pending.missing() {
                Some(missing) => {
                    self.missing_includes.push(missing);
                    return Ok(());
                }
                // the main file, not there yet: an empty ledger
                None => String::new(),
            },
            Err(error) => return Err(error),
        };
        let directives = data_type.transform(text, Some(name))?;
        self.queue.extend(pending.includes(&directives));
        self.directives.extend(directives);
        self.visited.push(pending.path);
        Ok(())
    }

    /// what the load read
    pub fn finish(self) -> LoadResult {
        LoadResult {
            directives: self.directives,
            visited_files: self.visited,
            missing_includes: self.missing_includes,
        }
    }
}

/// `LocalFileSystemDataSource` is the data source that stores the ledger on the local disk, as the `Fs` service of the
/// opendal source `zhang serve` runs does: it loads through [`LedgerFiles`] and appends through [`append_plan`], so a
/// ledger loads and is written alike through both.
///
/// It reads and writes a relative path within the root of the ledger it loaded last, never the working directory, and
/// creates the directories a write needs; an absolute path is read and written where it is.
pub struct LocalFileSystemDataSource {
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
    /// the root of the ledger this source loaded last; `None` before its first load
    root: std::sync::RwLock<Option<PathBuf>>,
}

impl LocalFileSystemDataSource {
    pub fn new<DT: DataType<Carrier = String> + Send + Sync + 'static>(data_type: DT) -> Self {
        LocalFileSystemDataSource {
            data_type: Box::new(data_type),
            root: std::sync::RwLock::new(None),
        }
    }

    /// `path` on the local disk: a relative path within the root of the ledger this source loaded (the working
    /// directory before a load), an absolute path where it is. `None` for a relative path climbing out of the root
    fn resolve(&self, path: &str) -> Option<PathBuf> {
        let path = Path::new(path);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let root = self.root.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().unwrap_or_default();
        normalize_relative(path).map(|within| root.join(within))
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
}

impl DataSource for LocalFileSystemDataSource {
    fn export(&self, directive: Directive) -> ZhangResult<Vec<u8>> {
        Ok(self.data_type.export(Spanned::new(directive, SpanInfo::default())).into_bytes())
    }

    /// a relative path climbing out of the ledger's root names no file of it
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        let file = self.resolve(&path).ok_or(ZhangError::FileNotFound)?;
        std::fs::read(file).map_err(|error| storage_error(&path, Access::Read, error.kind().into(), error))
    }

    /// the ledger root itself: this source reads the paths it is given from the local disk
    fn local_root(&self, entry: &Path) -> Option<PathBuf> {
        Some(entry.to_path_buf())
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let root = PathBuf::from(entry);
        *self.root.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(root.clone());
        let mut files = LedgerFiles::new(root.clone(), &endpoint);
        while let Some(file) = files.next(|dir| Self::entries(&root.join(dir)))? {
            let content = self.get(file.path());
            files.read(file, content, &*self.data_type)?;
        }
        Ok(files.finish())
    }

    fn save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        let file = self
            .resolve(&path)
            .ok_or_else(|| ZhangError::CustomError(format!("{path} is not in the ledger's directory")))?;
        let failed = |error: std::io::Error| storage_error(&path, Access::Write, error.kind().into(), error);
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(failed)?;
        }
        std::fs::write(&file, content).map_err(failed)
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
    /// the `include`s that name no file, which the load went on without; the ledger reports them
    pub missing_includes: Vec<MissingInclude>,
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

    /// A file is missing only when the source says it is not there; a refusal to read it is a refusal, and any other
    /// error stays an error.
    #[test]
    fn a_file_is_missing_only_when_the_source_says_so() {
        let get = |path: &str| Answering.get_existing(path.to_owned());
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
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::Directive;

    use super::{path_in_ledger, FileText, LocalFileSystemDataSource};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::inputs::ExtraInput;
    use crate::ledger::Ledger;
    use crate::ZhangError;

    /// A file is named by its path within the ledger, normalized, whether it is given relative to the root or under
    /// it; nothing outside the root is: an absolute path elsewhere, or one climbing above the root.
    #[test]
    fn a_file_is_named_by_its_path_within_the_ledger() {
        let within = |root: &str, path: &str| path_in_ledger(Path::new(root), Path::new(path));
        let some = |path: &str| Some(PathBuf::from(path));
        assert_eq!(within("/ledger", "/ledger/data/2024.zhang"), some("data/2024.zhang"));
        assert_eq!(within("/ledger", "data/2024.zhang"), some("data/2024.zhang"));
        assert_eq!(within("/ledger", "/ledger/./data/old/../2024.zhang"), some("data/2024.zhang"));
        assert_eq!(within("/ledger", "./data//2024.zhang"), some("data/2024.zhang"));
        assert_eq!(within("/ledger", "/ledger"), some(""));
        assert_eq!(within("/ledger", "/ledger/../outside/o.zhang"), None);
        assert_eq!(within("/ledger", "../outside/o.zhang"), None);
        assert_eq!(within("/ledger", "data/../../o.zhang"), None);
        assert_eq!(within("/ledger", "/elsewhere/o.zhang"), None);
        assert_eq!(within("/ledger", "/ledger-other/o.zhang"), None);
        assert_eq!(within("/", "/data/2024.zhang"), some("data/2024.zhang"));
        assert_eq!(within(".", "./main.zhang"), some("main.zhang"));
    }

    const OPENS: &str = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

    /// the transaction recorded on 2024-01-15, as the web UI appends it
    fn coffee() -> Vec<Directive> {
        ZhangDataType {}
            .transform("2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n".to_owned(), None)
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect()
    }

    /// A ledger whose main file is not there yet is empty, without an error, as `zhang serve` serves a new folder: this
    /// source failed the load with an io error. The first entry appended writes the main file.
    #[test]
    fn a_missing_main_file_is_an_empty_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source.clone()).expect("an empty ledger");

        assert!(ledger.errors.is_empty());
        assert_eq!(ledger.visited_files, vec![root.join("main.zhang")]);
        ledger.data_source.append(&ledger, coffee()).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("main.zhang")).unwrap().trim(),
            "include \"data/2024/01.zhang\""
        );
        std::fs::write(root.join("accounts.zhang"), OPENS).unwrap();
        std::fs::write(root.join("main.zhang"), "include \"accounts.zhang\"\ninclude \"data/2024/01.zhang\"\n").unwrap();
        let reloaded = Ledger::load_with_data_source(root, "main.zhang".to_owned(), source).unwrap();
        assert_eq!(reloaded.transactions().len(), 1);
    }

    /// An `include` of a file outside the ledger's directory, by a relative path climbing out of it or by an absolute
    /// path, names no file of the ledger: it is an error on the `include`, and the rest loads, as `zhang serve`
    /// reports it. This source read it.
    #[test]
    fn an_include_outside_the_ledger_is_an_error_on_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("ledger")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::fs::write(root.join("outside/o.zhang"), "1970-01-01 open Assets:Outside\n").unwrap();
        let absolute = root.join("outside/o.zhang").display().to_string();
        std::fs::write(
            root.join("ledger/main.zhang"),
            format!("{OPENS}include \"../outside/o.zhang\"\ninclude \"{absolute}\"\n"),
        )
        .unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let ledger = Ledger::load_with_data_source(root.join("ledger"), "main.zhang".to_owned(), source).unwrap();

        assert_eq!(
            include_errors(&ledger),
            vec![
                ("../outside/o.zhang".to_owned(), "include \"../outside/o.zhang\"".to_owned()),
                (absolute.clone(), format!("include \"{absolute}\""))
            ]
        );
        assert!(opened(&ledger, "Assets:Cash"));
        assert!(!opened(&ledger, "Assets:Outside"));
        assert_eq!(ledger.visited_files, vec![root.join("ledger/main.zhang")]);
        assert!(ledger.extra_inputs.is_empty(), "a file outside the ledger's directory is not watched");
    }

    /// A relative path this source reads or writes is within the ledger's directory, as the paths of a `DataSource`
    /// are, never the working directory: a document uploaded to `attachments/<id>/<name>`, or a file saved in the
    /// editor by the name the file list gives it. The folders it needs are created. The spans of the directives name
    /// their files so too, as `zhang serve` names them.
    #[test]
    fn a_relative_path_is_read_and_written_within_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("main.zhang"), "include \"data/accounts.zhang\"\n").unwrap();
        std::fs::write(root.join("data/accounts.zhang"), OPENS).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();

        let files: Vec<_> = ledger.directives.iter().map(|it| it.span.filename.clone().unwrap()).collect();
        assert_eq!(files, vec![PathBuf::from("data/accounts.zhang"); 2]);
        let saved = "attachments/0f3c/receipt.txt".to_owned();
        ledger.data_source.save(&ledger, saved.clone(), b"receipt").unwrap();
        assert_eq!(std::fs::read(root.join(&saved)).unwrap(), b"receipt");
        assert_eq!(ledger.data_source.get(saved).unwrap(), b"receipt");
        assert_eq!(ledger.data_source.get("data/accounts.zhang".to_owned()).unwrap(), OPENS.as_bytes());
        assert!(ledger.data_source.get("../outside.zhang".to_owned()).unwrap_err().is_file_not_found());
        assert!(
            !Path::new("attachments/0f3c/receipt.txt").exists(),
            "nothing is written in the working directory"
        );
    }

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
        assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
        assert_eq!(reloaded.transactions().len(), 2);
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

        assert!(ledger.errors.is_empty(), "{:?}", ledger.errors);
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
        assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
        assert_eq!(reloaded.transactions().len(), 1);
    }

    /// A file that is not UTF-8 text, such as one holding a latin-1 `é` in a comment, stops the load with an error
    /// naming the file and the line: this source read it with the byte replaced, and loaded other text than the file
    /// holds, where `zhang serve` panicked.
    #[test]
    fn a_file_that_is_not_utf8_is_a_load_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), "1970-01-01 open Assets:Cash\ninclude \"bad.zhang\"\n").unwrap();
        std::fs::write(dir.path().join("bad.zhang"), b"1970-01-01 open Assets:Bank\n; \xe9t\xe9\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let error = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source)
            .err()
            .expect("the ledger does not load");

        let ZhangError::InvalidUtf8 { path, line } = &error else { panic!("{error}") };
        assert!(path.ends_with("bad.zhang"), "{path}");
        assert_eq!(*line, 2);
        assert!(error.to_string().contains("bad.zhang is not UTF-8 text: line 2"), "{error}");
    }

    /// An append to a file that is not UTF-8 text leaves it as it is, and the error names the file: it was a 500
    /// that named no file
    #[test]
    fn an_append_to_a_file_that_is_not_utf8_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n").unwrap();
        let month = dir.path().join("data/2024/01.zhang");
        std::fs::create_dir_all(month.parent().unwrap()).unwrap();
        std::fs::write(&month, b"; \xe9t\xe9\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        let coffee: Vec<Directive> = ZhangDataType {}
            .transform("2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n".to_owned(), None)
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect();

        let error = ledger.data_source.append(&ledger, coffee).expect_err("the transaction is not appended");

        assert!(
            matches!(&error, ZhangError::InvalidUtf8 { path, line: 1 } if path.ends_with("01.zhang")),
            "{error}"
        );
        assert_eq!(std::fs::read(&month).unwrap(), b"; \xe9t\xe9\n");
    }

    /// A file's text is after the byte order mark it may start with; bytes that are not UTF-8 are an error naming the
    /// file and the line of the first of them, the mark or not
    #[test]
    fn a_file_is_decoded_strictly_after_its_byte_order_mark() {
        let text = FileText::decode(b"\xef\xbb\xbf\xc3\xa9t\xc3\xa9\n".to_vec(), "a.zhang").unwrap();
        assert_eq!((text.bom, text.text.as_str()), (true, "\u{e9}t\u{e9}\n"));
        assert_eq!(text.into_bytes(), b"\xef\xbb\xbf\xc3\xa9t\xc3\xa9\n");
        for (content, line) in [(&b"\xef\xbb\xbfok\n\nok\xe9"[..], 3), (b"\xff", 1), (b"a\nb\r\n\xc3", 3)] {
            match FileText::decode(content.to_vec(), "data/a.zhang") {
                Err(ZhangError::InvalidUtf8 { path, line: at }) => assert_eq!((path.as_str(), at), ("data/a.zhang", line), "{content:?}"),
                other => panic!("{content:?}: {other:?}"),
            }
        }
    }

    /// A pattern names the files that exist, whole names only, in any part of the path (#494): `*.zhang` next to the
    /// main file, `data/*/*.zhang`, and `data/*/accounts.zhang` with a literal last part. The main file matches its
    /// own pattern and is still read once; `01.zhang.bak` and the hidden `.#01.zhang` are left out. `nothing/*.zhang`,
    /// which matches no file, is an error on its `include`.
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
        assert_eq!(
            include_errors(&ledger),
            vec![("nothing/*.zhang".to_owned(), "include \"nothing/*.zhang\"".to_owned())]
        );
        assert_eq!(ledger.transactions().len(), 2);
    }

    /// whether the ledger has an `open` of `account`
    fn opened(ledger: &Ledger, account: &str) -> bool {
        let open = |directive: &Directive| matches!(directive, Directive::Open(open) if open.account.name() == account);
        ledger.directives.iter().any(|it| open(&it.data))
    }

    /// the `IncludeNotFound` errors of `ledger`: the `path` meta and the text of the `include`;
    /// it has no other error
    fn include_errors(ledger: &Ledger) -> Vec<(String, String)> {
        ledger
            .errors
            .iter()
            .map(|it| {
                assert_eq!(it.error_type, ErrorKind::IncludeNotFound, "{:?}", ledger.errors);
                let text = it.span.as_ref().map(|span| span.content.trim().to_owned()).unwrap_or_default();
                (it.metas["path"].clone(), text)
            })
            .collect()
    }

    /// An `include` naming a file that is not there is an error on it, and the rest of the ledger loads (#494): this
    /// source failed the load on it, with an io error that did not name the file. Creating the file makes the ledger
    /// stale
    #[test]
    fn an_include_naming_no_file_is_an_error_on_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), "include \"accounts.zhang\"\ninclude \"acounts/typo.zhang\"\n").unwrap();
        std::fs::write(root.join("accounts.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();

        assert_eq!(
            include_errors(&ledger),
            vec![("acounts/typo.zhang".to_owned(), "include \"acounts/typo.zhang\"".to_owned())]
        );
        assert!(opened(&ledger, "Assets:Cash"));
        assert_eq!(ledger.visited_files, vec![root.join("main.zhang"), root.join("accounts.zhang")]);
        assert_eq!(
            ledger.extra_inputs.iter().cloned().collect::<Vec<_>>(),
            vec![ExtraInput::File("acounts/typo.zhang".into())]
        );
    }
}
