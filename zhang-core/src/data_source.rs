use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use chrono::Datelike;
use log::{debug, warn};
use minijinja::{context, Environment};
use zhang_ast::{Directive, DirectiveType, Include, SpanInfo, Spanned, ZhangString};

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
    /// with [`DataSource::get`] and writing it back with [`DataSource::save`], once per file ([`DataSource::append_files`])
    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append_files(ledger, directives).map(|_| ()).map_err(AppendError::into_error)
    }

    /// Append `directives` to the files of `ledger`, one write per file: every directive planned for a file
    /// ([`append_plan`]) goes into it at once, in the plan's order of files, so a file is either as it was or holds
    /// all of its new text. The files written, in order; or, when a read or a write fails, [`AppendError`]: the caller
    /// tells what landed, and the rest was not touched.
    ///
    /// Every file is read before any is written ([`DataSource::get_to_replace`]): a path holding something other than
    /// a file (a directory where the file goes), a file the storage refuses to read, or one that is not text fails the
    /// append with nothing written, so the main file never gets an `include` of it, which would fail every later load. When a write fails all the same, the `include` lines
    /// this append wrote of the files it did not write are taken out again ([`AppendError::taken_out`]): the ledger
    /// stays as loadable as it was
    fn append_files(&self, ledger: &Ledger, directives: Vec<Directive>) -> Result<Vec<String>, AppendError> {
        let untouched = AppendError::untouched;
        // every text first, so an export that fails writes nothing
        let mut files: Vec<AppendedFile> = vec![];
        for (path, directive) in append_plan(ledger, directives).map_err(untouched)? {
            let includes = match &directive {
                Directive::Include(include) => Some(include.file.clone().to_plain_string()),
                _ => None,
            };
            let text = self.export(directive).map_err(untouched)?;
            let index = match files.iter().position(|file| file.path == path) {
                Some(index) => index,
                None => {
                    files.push(AppendedFile {
                        path,
                        original: None,
                        texts: vec![],
                    });
                    files.len() - 1
                }
            };
            files[index].texts.push((text, includes));
        }
        // then every file read, before any is written; none is there for a file this append creates
        for file in &mut files {
            file.original = match self.get_to_replace(file.path.clone()).map_err(untouched)? {
                Some(content) => Some(FileText::decode(content, &file.path).map_err(untouched)?),
                None => None,
            };
        }
        let mut written = vec![];
        for (index, file) in files.iter().enumerate() {
            if let Err(error) = self.save(ledger, file.path.clone(), &file.content(|_| true)) {
                return Err(take_out_includes(self, ledger, &files, index, written, error));
            }
            written.push(file.path.clone());
        }
        Ok(written)
    }

    /// The content of the file at `path`, relative to the ledger root and written with `/`, that a write is about to
    /// replace: `None` when nothing is there, which the write creates, and an error ([`not_a_file`]) when something
    /// other than a file is there, a directory say, which no write may replace. The default reads with
    /// [`DataSource::get`], which errs on reading a directory on the local disk; a source that may read a directory as
    /// content (a WebDAV service answers a page listing it) checks what is there first
    fn get_to_replace(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        match self.get(path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.is_file_not_found() => Ok(None),
            Err(error) => Err(error),
        }
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
    /// writer keeps every other writer of the ledger out from this read until it saved the file, so no other write
    /// comes between
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
    output_file(ledger, directive.datetime(), directive.directive_type())
}

/// [`directive_output_file`] for a directive of the kind `directive_type` dated `datetime` (`None` for one without a
/// date), before the directive exists: where a directive a request describes would go, even one the request cannot
/// build yet
pub fn output_file(ledger: &Ledger, datetime: Option<chrono::NaiveDateTime>, directive_type: DirectiveType) -> ZhangResult<PathBuf> {
    let (entry, main_file_endpoint) = &ledger.entry;
    let Some(datetime) = datetime else {
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
            type => directive_type.to_string(),
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
/// That is the file [`directive_output_file`] gives it. The main file gets an `include` of a file the ledger did not
/// load first, once per append, so an append never leaves a file no `include` names, whose entries a retry would add
/// a second time without anyone noticing: when the file's write fails, [`DataSource::append_files`] takes the
/// `include` out again, and one it cannot take out is an `include` of a file that is not there, which the next load
/// reports as `IncludeNotFound` where the user sees it. A file outside the ledger's directory is an
/// error, and nothing is written: a source writes nothing there, and an `include` of it would name no file of the
/// ledger.
pub fn append_plan(ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<Vec<(String, Directive)>> {
    let within = |file: &Path| {
        ledger
            .path_in_ledger(file)
            .filter(|it| !it.as_os_str().is_empty())
            .ok_or_else(|| ZhangError::CustomError(format!("{} is not in the ledger's directory", file.display())))
    };
    let main = within(&ledger.entry.0.join(&ledger.entry.1))?;
    // the files the ledger loaded, those an `include` names that are not there yet (a write that stopped after its
    // include, reported as `IncludeNotFound`: the file this append creates is included already), by the path the load
    // resolved, never the path as the `include` writes it, which is relative to the file holding it; and those this
    // append includes
    let mut included: Vec<PathBuf> = ledger.visited_files.iter().filter_map(|it| ledger.path_in_ledger(it)).collect();
    included.extend(ledger.missing_files.iter().cloned());
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

/// An append that failed: the error, the files that hold new text now, and the `include` lines it wrote and took out
/// again or could not. Files are named by their paths within the ledger. The files after the failure were not touched
#[derive(Debug)]
pub struct AppendError {
    pub error: ZhangError,
    /// the files written whole, in order; a file whose only new text was taken out again is as it was and not listed
    pub written: Vec<String>,
    /// the `include` lines this append wrote and took out again, after the file they name could not be written: the
    /// file that held the include (the main file), and the file it named
    pub taken_out: Vec<(String, String)>,
    /// the `include` lines this append wrote that stayed, though the file they name was not written: taking them out
    /// failed too. A load reports one as `IncludeNotFound` while nothing is there, and fails while the path is not a
    /// file; a retry writes the file without a second include
    pub dangling: Vec<(String, String)>,
}

impl AppendError {
    /// a failure before anything was written
    pub fn untouched(error: ZhangError) -> AppendError {
        AppendError {
            error,
            written: vec![],
            taken_out: vec![],
            dangling: vec![],
        }
    }

    /// The error of a caller that tells no more than one error: the storage's, and when an `include` of a file that
    /// was not written stayed, what to take out
    pub fn into_error(self) -> ZhangError {
        match self.dangling.is_empty() {
            true => self.error,
            false => ZhangError::CustomError(format!("{}; {}", self.error, dangling_includes(&self.dangling))),
        }
    }
}

/// the error of a write to `path`, a path of the ledger where something other than a file is: a directory, or another
/// kind of entry, which no write replaces
pub fn not_a_file(path: &str) -> ZhangError {
    ZhangError::CustomError(format!("cannot write {path}: it is a directory or another entry, not a file"))
}

/// what to do about `dangling`, `include` lines naming files that were not written, each with the file holding it
pub fn dangling_includes(dangling: &[(String, String)]) -> String {
    dangling
        .iter()
        .map(|(main, file)| format!("{main} still includes {file}, which was not written: take that include out of {main}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// a file an append writes: its path within the ledger, its text when the append read it (none when it is not there),
/// and the new texts, each with the file it is the `include` of, when it is one
struct AppendedFile {
    path: String,
    original: Option<FileText>,
    texts: Vec<(Vec<u8>, Option<String>)>,
}

impl AppendedFile {
    /// the file's content with the texts `keep` keeps appended, each on a line of its own; the byte order mark the file
    /// may start with stays ([`FileText`])
    fn content(&self, keep: impl Fn(&(Vec<u8>, Option<String>)) -> bool) -> Vec<u8> {
        let mut bytes = self.original.clone().unwrap_or_else(|| FileText::new(String::new())).into_bytes();
        for (text, _) in self.texts.iter().filter(|it| keep(it)) {
            bytes.extend_from_slice(b"\n");
            bytes.extend_from_slice(text);
            bytes.extend_from_slice(b"\n");
        }
        bytes
    }
}

/// After the write of `files[failed]` failed with `error`, `written` being the files written before it: the `include`
/// lines of it and of the files after it, none of them written, taken out of the files written (the main file), each
/// written again with the rest of its new text. A file left with no new text is as it was, and no longer written
fn take_out_includes<S: DataSource + ?Sized>(
    source: &S, ledger: &Ledger, files: &[AppendedFile], failed: usize, mut written: Vec<String>, error: ZhangError,
) -> AppendError {
    let unwritten: Vec<&str> = files[failed..].iter().map(|file| file.path.as_str()).collect();
    let of_unwritten = |(_, includes): &(Vec<u8>, Option<String>)| includes.as_deref().is_some_and(|file| unwritten.contains(&file));
    let (mut taken_out, mut dangling) = (vec![], vec![]);
    for file in &files[..failed] {
        let includes: Vec<(String, String)> = file
            .texts
            .iter()
            .filter(|text| of_unwritten(text))
            .filter_map(|(_, includes)| includes.clone())
            .map(|included| (file.path.clone(), included))
            .collect();
        if includes.is_empty() {
            continue;
        }
        match source.save(ledger, file.path.clone(), &file.content(|text| !of_unwritten(text))) {
            Ok(()) => {
                if file.texts.iter().all(of_unwritten) {
                    written.retain(|it| *it != file.path);
                }
                taken_out.extend(includes);
            }
            Err(rollback) => {
                warn!("the include lines of files not written stay in {}: {rollback}", file.path);
                dangling.extend(includes);
            }
        }
    }
    AppendError {
        error,
        written,
        taken_out,
        dangling,
    }
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
/// climbs above its start. Inputs a plugin's file functions record are cleaned the same way, and a source writing to
/// the local disk keeps a write within its root with it
pub fn normalize_relative(relative: &Path) -> Option<PathBuf> {
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

    /// what is at the path is looked at first, its links followed: a directory, or another entry that is no file, is
    /// [`not_a_file`]
    fn get_to_replace(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        let file = self.resolve(&path).ok_or(ZhangError::FileNotFound)?;
        match std::fs::metadata(&file) {
            Ok(metadata) if !metadata.is_file() => return Err(not_a_file(&path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            // anything else is the read's to tell
            _ => {}
        }
        match self.get(path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.is_file_not_found() => Ok(None),
            Err(error) => Err(error),
        }
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

    /// The file is written whole or not at all when the directory allows it ([`save_file`])
    fn save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        let file = self
            .resolve(&path)
            .ok_or_else(|| ZhangError::CustomError(format!("{path} is not in the ledger's directory")))?;
        save_file(&file, &path, content)
    }
}

/// Write `content` to `file` on the local disk, the file at `path` of a ledger, whole or not at all when the directory
/// allows it: the content goes to a temporary file next to the file (the real file, behind any symbolic link, which
/// stays a link), with the file's owner and permissions, flushed to the disk, then renamed over the file
/// ([`write_atomically`]); a crash or a full disk while writing leaves the file as it was. Where no such temporary
/// file can be made or renamed, the file is written in place, as before. The directories the file needs are made. The
/// error names `path`, as the ledger knows the file, and a refusal is [`ZhangError::WriteRefused`]; every source
/// writing to the local disk saves through here, so a file is replaced the same way whichever way the ledger is served
pub fn save_file(file: &Path, path: &str, content: &[u8]) -> ZhangResult<()> {
    let failed = |error: std::io::Error| storage_error(path, Access::Write, error.kind().into(), error);
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(failed)?;
    }
    write_atomically(file, content).map_err(failed)
}

/// Write `content` to `file` whole or not at all, and where a plain write would be refused, refuse it too.
///
/// - The real file is `file` with its symbolic links followed ([`resolve_links`]): a link stays a link, and its target
///   gets the content. A file this process may not write is refused, as a plain write refuses it: the rename below
///   would replace a read-only file, or a file of another user, without asking.
/// - A file with more than one hard link (unix) is written in place: a rename would give this name a new file and
///   leave the others with the old content.
/// - The content goes to a new temporary file next to the real file, so the rename stays on one file system. It is
///   made with `create_new` (`O_CREAT | O_EXCL`), which never opens what is already there, nor follows a symbolic
///   link planted at its name, under a name nobody can guess ([`temporary_file`]).
/// - Before any content, the temporary file gets the owner (unix, only where it differs) and the permissions of the
///   real file through its own handle (`fchown`, `fchmod`), never by its path. Where it cannot get them, a file of
///   another user say, or a file system that refuses an owner or a mode (a rootless container, some FUSE or SMB
///   mounts), the file is written in place instead, so it keeps both.
/// - The content is written and flushed to the disk, the temporary file renamed over the real file, which replaces it
///   in one step on POSIX systems and on Windows, and (unix) the directory flushed to the disk, so the rename
///   survives a crash too. The directory's flush is best effort: a file system that cannot flush a directory has
///   the file whole either way, and the save is not failed for it.
///
/// The file replaced is a new file: what the old one held besides its content, owner and permissions (extended
/// attributes, ACLs, macOS Finder tags, its creation time) is not carried over; std has no portable way to copy them.
/// The temporary file is removed when a step fails. Where it cannot be made or renamed for want of permission, or the
/// rename is refused (the file busy, across file systems), the file is written in place, as a plain write would: a
/// write or flush that fails, a full or failing disk at any step, leaves the file as it was and is the error, since a
/// plain write would truncate the file and fail the same way ([`falls_back_in_place`])
fn write_atomically(file: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let target = resolve_links(file);
    let existing = std::fs::metadata(&target).ok().filter(|it| it.is_file());
    if existing.is_some() {
        // a file this process may not write, refused as a plain write refuses it; opening it changes nothing
        match std::fs::OpenOptions::new().write(true).open(&target) {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Err(error),
            _ => {}
        }
    }
    #[cfg(unix)]
    if existing.as_ref().is_some_and(|it| std::os::unix::fs::MetadataExt::nlink(it) > 1) {
        return std::fs::write(&target, content);
    }
    let directory = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    // the plain write that takes over when `step` failed with `error`, or the error
    let in_place = |step: SaveStep, error: std::io::Error| match falls_back_in_place(step, &error) {
        true => std::fs::write(&target, content),
        false => Err(error),
    };
    let name = target.file_name().and_then(|it| it.to_str()).unwrap_or("file");
    let (mut writer, temp) = match temporary_file(&directory, existing.is_some(), || temporary_name(name)) {
        Ok(created) => created,
        Err(error) => return in_place(SaveStep::Create, error),
    };
    if let Some(existing) = &existing {
        if let Err(error) = stand_in_for(&writer, existing) {
            drop(writer);
            std::fs::remove_file(&temp).ok();
            return in_place(SaveStep::Attributes, error);
        }
    }
    let written = writer.write_all(content).and_then(|()| writer.sync_all());
    // closed before the rename, which Windows refuses for an open file
    drop(writer);
    if let Err(error) = written {
        std::fs::remove_file(&temp).ok();
        return in_place(SaveStep::Write, error);
    }
    if let Err(error) = std::fs::rename(&temp, &target) {
        std::fs::remove_file(&temp).ok();
        return in_place(SaveStep::Rename, error);
    }
    #[cfg(unix)]
    if let Ok(directory) = std::fs::File::open(&directory) {
        // best effort, see above
        directory.sync_all().ok();
    }
    Ok(())
}

/// Give `temp`, the temporary file standing in for a file whose metadata is `existing`, the owner (unix) and the
/// permissions of that file, through its handle. The owner first, and only the ids that differ: a process that is not
/// root may not change an owner at all, and a change of owner clears the set-user-id and set-group-id bits the
/// permissions may hold
fn stand_in_for(temp: &std::fs::File, existing: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let made = temp.metadata()?;
        let uid = (made.uid() != existing.uid()).then(|| existing.uid());
        let gid = (made.gid() != existing.gid()).then(|| existing.gid());
        if uid.is_some() || gid.is_some() {
            std::os::unix::fs::fchown(temp, uid, gid)?;
        }
    }
    temp.set_permissions(existing.permissions())
}

/// A new temporary file in `directory`, opened for writing with `create_new`, by the first of the names `name` gives
/// that nothing holds yet: a file or a symbolic link already there is left alone, never opened, written or followed,
/// and the next name is tried. A temporary file standing in for an existing file (`private`) is made readable by
/// this process alone (unix), until it gets the file's permissions; a new file gets the permissions a plain write
/// gives it. `AlreadyExists` after 16 names taken
fn temporary_file(directory: &Path, private: bool, mut name: impl FnMut() -> String) -> std::io::Result<(std::fs::File, PathBuf)> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut taken = None;
    for _ in 0..16 {
        let temp = directory.join(name());
        match options.open(&temp) {
            Ok(file) => return Ok((file, temp)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => taken = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(taken.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::AlreadyExists)))
}

/// The longest a temporary file's name keeps of the name of the file it stands in for, in bytes
const TEMPORARY_STEM: usize = 64;

/// The name of a temporary file for the file named `name`: `.{name}.{16 hex digits}.tmp`, hidden, and telling whose it
/// is: the name cut to [`TEMPORARY_STEM`] bytes at a character boundary, so the temporary name stays within the
/// 255 bytes a file name may hold whenever the name does. The digits are a hash keyed by the random keys the standard
/// library seeds its hash maps with from the operating system, and differ on every call, so nobody can guess the name
/// to plant something there first
fn temporary_name(name: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    let mut end = name.len().min(TEMPORARY_STEM);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!(".{}.{:016x}.tmp", &name[..end], hasher.finish())
}

/// a step of [`write_atomically`] that can fail
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveStep {
    /// making the temporary file
    Create,
    /// giving it the file's owner and permissions
    Attributes,
    /// writing the content to it and flushing it to the disk
    Write,
    /// renaming it over the file
    Rename,
}

/// Whether a plain write of the file takes over after `step` failed with `error`; the file is still as it was then,
/// and the plain write is what every source did before. When the directory refuses the temporary file for want of
/// permission; when the temporary file cannot get the file's owner or permissions, whatever the reason (another
/// user's file, a file system that refuses an owner or a mode, as a rootless container or a FUSE or SMB mount does:
/// `EPERM`, `EINVAL`, `ENOTSUP`, `ENOSYS`); when the rename is refused (not allowed, the file busy, or across file
/// systems). Never when the disk or a quota is full or the disk fails ([`disk_failed`]), at any step: the plain write
/// would truncate the file and fail the same way, and the file is left untouched for the user instead
fn falls_back_in_place(step: SaveStep, error: &std::io::Error) -> bool {
    let refused = error.kind() == std::io::ErrorKind::PermissionDenied;
    match step {
        SaveStep::Create => refused,
        SaveStep::Attributes => !disk_failed(error),
        SaveStep::Write => false,
        SaveStep::Rename => refused || rename_refused(error),
    }
}

/// whether `error` says that the disk or a quota is full (`ENOSPC`, `EDQUOT`, which std reads as these kinds on every
/// platform), or that the disk failed (`EIO`, which has no kind: its unix number)
fn disk_failed(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    const EIO: i32 = 5;
    #[cfg(unix)]
    if error.raw_os_error() == Some(EIO) {
        return true;
    }
    matches!(error.kind(), std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded)
}

/// whether a failed rename says that this rename cannot be done here: the file busy (`EBUSY`) or the temporary file on
/// another file system (`EXDEV`), which std reads as these kinds on every platform
fn rename_refused(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::ResourceBusy | std::io::ErrorKind::CrossesDevices)
}

/// `file` with its symbolic links followed, as far as they lead to something: the real file a write replaces, so
/// the link itself stays. A link that leads nowhere is followed to where it points, which the write then creates
fn resolve_links(file: &Path) -> PathBuf {
    let mut path = file.to_path_buf();
    for _ in 0..32 {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let Ok(link) = std::fs::read_link(&path) else { break };
                path = match link.is_absolute() {
                    true => link,
                    false => path.parent().unwrap_or(Path::new("")).join(link),
                };
            }
            _ => break,
        }
    }
    path
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
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use zhang_ast::error::ErrorKind;
    use zhang_ast::Directive;

    use super::{path_in_ledger, DataSource, FileText, LoadResult, LocalFileSystemDataSource};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::inputs::ExtraInput;
    use crate::ledger::Ledger;
    use crate::{ZhangError, ZhangResult};

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

    /// An append writes the `include` of a new data file into the main file before the data file, and each file
    /// once: when the main file cannot be written (here: not even read), nothing is written at all, so a retry adds
    /// nothing twice; an include that lands without its file would be a visible `IncludeNotFound`, never a silent
    /// duplicate. The retry, once the file is writable again, writes both
    #[cfg(unix)]
    #[test]
    fn an_append_writes_the_include_before_the_data_file_and_nothing_when_the_main_file_refuses() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.zhang");
        std::fs::write(&main, OPENS).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&main).is_ok() {
            eprintln!("skipping: this user reads a file without permissions (root)");
            return;
        }

        let failure = ledger.data_source.append_files(&ledger, coffee()).expect_err("the include cannot be written");

        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(failure.error.to_string().contains("main.zhang"), "{}", failure.error);
        assert_eq!(failure.written, Vec::<String>::new(), "the main file comes first, so nothing landed");
        assert!(
            !dir.path().join("data/2024/01.zhang").exists(),
            "the data file is not written without its include"
        );
        assert!(!std::fs::read_to_string(&main).unwrap().contains("include"), "the main file is as it was");
        let written = ledger.data_source.append_files(&ledger, coffee()).unwrap();
        assert_eq!(written, vec!["main.zhang".to_owned(), "data/2024/01.zhang".to_owned()]);
        assert_eq!(std::fs::read_to_string(&main).unwrap().matches("include \"data/2024/01.zhang\"").count(), 1);
    }

    /// An append writes each file once, with every directive planned for it, and reports what it wrote: of two
    /// directives for one new file the include goes to the main file first, then both texts to the file in one
    /// write. When a later file cannot be written, the files before it are whole and named, the rest untouched
    #[test]
    fn an_append_writes_each_file_once_and_names_what_it_wrote_before_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), OPENS).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();
        let directives = |text: &str| -> Vec<Directive> {
            ZhangDataType {}
                .transform(text.to_owned(), None)
                .unwrap()
                .into_iter()
                .map(|it| it.data)
                .collect()
        };
        let january = "2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n2024-01-16 * \"tea\"\n  Assets:Cash -3 CNY\n  Expenses:Food\n";

        let written = ledger.data_source.append_files(&ledger, directives(january)).unwrap();

        assert_eq!(written, vec!["main.zhang".to_owned(), "data/2024/01.zhang".to_owned()]);
        let file = std::fs::read_to_string(root.join("data/2024/01.zhang")).unwrap();
        assert_eq!(
            file,
            "\n2024-01-15 * \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n\n2024-01-16 * \"tea\"\n  Assets:Cash -3 CNY\n  Expenses:Food\n"
        );
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap().matches("include").count(), 1);

        // a February file that cannot be written: January lands, February does not, and the include of February the
        // main file got is taken out again, so the ledger loads as it did
        let refusing = RefusingSaves::new([("data/2024/02.zhang", 0)]);
        let reloaded = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), refusing.clone()).unwrap();
        let both = format!("{january}2024-02-01 * \"lunch\"\n  Assets:Cash -9 CNY\n  Expenses:Food\n");
        let main_before = std::fs::read_to_string(root.join("main.zhang")).unwrap();
        let failure = reloaded
            .data_source
            .append_files(&reloaded, directives(&both))
            .expect_err("February cannot be written");
        assert!(failure.error.to_string().contains("02.zhang"), "{}", failure.error);
        // January is included already, so its file comes first; then the main file, for the include of February,
        // which is as it was again
        assert_eq!(failure.written, vec!["data/2024/01.zhang".to_owned()]);
        assert_eq!(failure.taken_out, vec![("main.zhang".to_owned(), "data/2024/02.zhang".to_owned())]);
        assert_eq!(failure.dangling, vec![]);
        assert_eq!(
            std::fs::read_to_string(root.join("data/2024/01.zhang")).unwrap().matches("coffee").count(),
            2,
            "January was written again, whole"
        );
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap(), main_before);
        let loaded = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), ledger.data_source.clone()).unwrap();
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

        // the retry, once February can be written, writes its include and its file
        let written = loaded
            .data_source
            .append_files(&loaded, directives("2024-02-01 * \"lunch\"\n  Assets:Cash -9 CNY\n  Expenses:Food\n"))
            .unwrap();
        assert_eq!(written, vec!["main.zhang".to_owned(), "data/2024/02.zhang".to_owned()]);
        let whole = Ledger::load_with_data_source(root, "main.zhang".to_owned(), ledger.data_source.clone()).unwrap();
        assert!(whole.errors.is_empty(), "{:?}", whole.errors);
        assert_eq!(whole.transactions().len(), 5);
    }

    /// The local disk, refusing some saves: of a file listed with `n`, the first `n` saves are written and every later
    /// one refused, as a full disk or a read-only mount refuses them
    struct RefusingSaves {
        disk: LocalFileSystemDataSource,
        allowed: Mutex<HashMap<String, usize>>,
    }

    impl RefusingSaves {
        fn new<const N: usize>(allowed: [(&str, usize); N]) -> Arc<RefusingSaves> {
            Arc::new(RefusingSaves {
                disk: LocalFileSystemDataSource::new(ZhangDataType {}),
                allowed: Mutex::new(allowed.into_iter().map(|(path, saves)| (path.to_owned(), saves)).collect()),
            })
        }
    }

    impl DataSource for RefusingSaves {
        fn export(&self, directive: Directive) -> ZhangResult<Vec<u8>> {
            self.disk.export(directive)
        }

        fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
            self.disk.get(path)
        }

        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            self.disk.load(entry, endpoint)
        }

        fn save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
            if let Some(left) = self.allowed.lock().unwrap().get_mut(&path) {
                match *left {
                    0 => return Err(ZhangError::WriteRefused(path)),
                    _ => *left -= 1,
                }
            }
            self.disk.save(ledger, path, content)
        }
    }

    /// A directory where the file of an append goes (a security review finding): every file is read before any is
    /// written, so the append fails with nothing written. Before, the main file got the include first, and every load
    /// after it failed on the directory, which blocked the writes until someone edited the main file by hand.
    #[test]
    fn an_append_whose_file_is_a_directory_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), OPENS).unwrap();
        std::fs::create_dir_all(root.join("data/2024/01.zhang")).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();

        let failure = ledger
            .data_source
            .append_files(&ledger, coffee())
            .expect_err("a directory is no file to append to");

        assert!(failure.error.to_string().contains("data/2024/01.zhang"), "{}", failure.error);
        assert_eq!(
            (failure.written, failure.taken_out, failure.dangling),
            (vec![], vec![], vec![]),
            "refused before anything was written"
        );
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap(), OPENS);
        let reloaded = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), ledger.data_source.clone()).unwrap();
        assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
        // `append`, as the API calls it, the same
        assert!(ledger.data_source.append(&ledger, coffee()).is_err());
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap(), OPENS);
    }

    /// When a write fails and taking its include out of the main file fails too, the include stays and the error says
    /// so: the load reports it as `IncludeNotFound`, and a retry writes the file without a second include
    #[test]
    fn an_include_that_cannot_be_taken_out_again_is_told_and_a_retry_completes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), OPENS).unwrap();
        // the main file is written once, for the include; the data file never
        let refusing = RefusingSaves::new([("main.zhang", 1), ("data/2024/01.zhang", 0)]);
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), refusing.clone()).unwrap();

        let failure = ledger.data_source.append_files(&ledger, coffee()).expect_err("the data file cannot be written");

        let include = ("main.zhang".to_owned(), "data/2024/01.zhang".to_owned());
        assert_eq!(failure.written, vec!["main.zhang".to_owned()]);
        assert_eq!((failure.taken_out.clone(), failure.dangling.clone()), (vec![], vec![include]));
        let error = failure.into_error().to_string();
        assert!(
            error.contains("main.zhang still includes data/2024/01.zhang, which was not written: take that include out of main.zhang"),
            "{error}"
        );
        let dangling = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), refusing.clone()).unwrap();
        assert_eq!(dangling.errors.iter().filter(|it| it.error_type == ErrorKind::IncludeNotFound).count(), 1);

        let disk = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let dangling = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), disk).unwrap();
        let written = dangling.data_source.append_files(&dangling, coffee()).unwrap();
        assert_eq!(written, vec!["data/2024/01.zhang".to_owned()]);
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap().matches("include").count(), 1);
        let whole = Ledger::load_with_data_source(root, "main.zhang".to_owned(), dangling.data_source.clone()).unwrap();
        assert!(whole.errors.is_empty(), "{:?}", whole.errors);
        assert_eq!(whole.transactions().len(), 1);
    }

    /// the temporary files a save may have left in `dir`
    fn temporary_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    /// A plain write takes over only where the temporary file is refused, cannot get the file's owner or
    /// permissions, or cannot be renamed; a disk that refuses the content (full, failing) leaves the file as it was
    /// and is the error, at any step
    #[test]
    fn a_save_falls_back_to_a_plain_write_only_when_the_temporary_file_is_refused() {
        use std::io::{Error, ErrorKind};

        use super::{falls_back_in_place, SaveStep};

        let steps = [SaveStep::Create, SaveStep::Attributes, SaveStep::Write, SaveStep::Rename];
        let falls_back = |kind: ErrorKind| steps.map(|step| falls_back_in_place(step, &Error::from(kind)));
        // Create, Attributes, Write, Rename
        assert_eq!(falls_back(ErrorKind::StorageFull), [false; 4], "a full disk");
        assert_eq!(falls_back(ErrorKind::QuotaExceeded), [false; 4], "a full quota");
        assert_eq!(falls_back(ErrorKind::PermissionDenied), [true, true, false, true], "a refusal");
        for kind in [ErrorKind::ResourceBusy, ErrorKind::CrossesDevices] {
            assert_eq!(falls_back(kind), [false, true, false, true], "a rename that cannot be done here: {kind}");
        }
        for kind in [ErrorKind::InvalidInput, ErrorKind::Unsupported] {
            assert_eq!(falls_back(kind), [false, true, false, false], "an owner or a mode refused: {kind}");
        }
        assert_eq!(falls_back(ErrorKind::AlreadyExists), [false, true, false, false], "every temporary name taken");
        assert_eq!(falls_back(ErrorKind::NotFound), [false, true, false, false]);
    }

    /// The fallback by the numbers unix gives, as Linux and macOS number them
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_save_reads_the_unix_error_numbers() {
        use std::io::Error;

        use super::{falls_back_in_place, SaveStep};

        let (eperm, eio, eacces, ebusy, exdev, einval, enospc) = (1, 5, 13, 16, 18, 22, 28);
        #[cfg(target_os = "linux")]
        let (enosys, enotsup, edquot) = (38, 95, 122);
        #[cfg(target_os = "macos")]
        let (enosys, enotsup, edquot) = (78, 45, 69);
        let steps = [SaveStep::Create, SaveStep::Attributes, SaveStep::Write, SaveStep::Rename];
        let falls_back = |code: i32| steps.map(|step| falls_back_in_place(step, &Error::from_raw_os_error(code)));
        for code in [enospc, edquot, eio] {
            assert_eq!(falls_back(code), [false; 4], "a full or failing disk: {code}");
        }
        for code in [eacces, eperm] {
            assert_eq!(falls_back(code), [true, true, false, true], "a refusal: {code}");
        }
        for code in [ebusy, exdev] {
            assert_eq!(falls_back(code), [false, true, false, true], "a rename that cannot be done here: {code}");
        }
        // a rootless container or a FUSE or SMB mount refusing an owner or a mode
        for code in [einval, enotsup, enosys] {
            assert_eq!(falls_back(code), [false, true, false, false], "an owner or a mode refused: {code}");
        }
    }

    /// A save keeps a symbolic link a link and writes its target, and keeps the file's permissions: the temporary
    /// file is made next to the real file, with its mode, and renamed over it, and none is left behind
    #[cfg(unix)]
    #[test]
    fn a_save_keeps_a_symlink_and_the_files_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::create_dir_all(root.join("ledger/data/2024")).unwrap();
        std::fs::write(root.join("real/main.zhang"), OPENS).unwrap();
        std::os::unix::fs::symlink(root.join("real/main.zhang"), root.join("ledger/main.zhang")).unwrap();
        let private = root.join("ledger/data/2024/01.zhang");
        std::fs::write(&private, "").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.join("ledger"), "main.zhang".to_owned(), source).unwrap();
        let inode = |path: &Path| std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(path).unwrap());
        let inodes = [inode(&root.join("real/main.zhang")), inode(&private)];

        ledger.data_source.append(&ledger, coffee()).unwrap();

        assert_ne!(inode(&root.join("real/main.zhang")), inodes[0], "the link's target is replaced, not rewritten");
        assert_ne!(inode(&private), inodes[1], "the private file is replaced, not rewritten");
        let link = std::fs::symlink_metadata(root.join("ledger/main.zhang")).unwrap();
        assert!(link.file_type().is_symlink(), "the main file stays a symbolic link");
        assert!(
            std::fs::read_to_string(root.join("real/main.zhang"))
                .unwrap()
                .contains("include \"data/2024/01.zhang\""),
            "its target got the include"
        );
        assert_eq!(
            std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600,
            "a private file stays private"
        );
        assert!(std::fs::read_to_string(&private).unwrap().contains("coffee"));
        for folder in [root.join("real"), root.join("ledger"), root.join("ledger/data/2024")] {
            assert_eq!(temporary_files(&folder), Vec::<String>::new(), "no temporary file is left behind");
        }
    }

    /// A save writes the file whole or not at all: when the write cannot be completed, here because a directory is in
    /// the file's place, what was there stays as it was and no temporary file is left behind
    #[test]
    fn a_save_that_cannot_complete_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), OPENS).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();
        // a non-empty directory where the file would be: the rename over it fails, after the temporary file is written
        std::fs::create_dir_all(root.join("data/2024/01.zhang/inside")).unwrap();

        let error = ledger
            .data_source
            .save(&ledger, "data/2024/01.zhang".to_owned(), b"2024-01-15 * \"coffee\"\n")
            .expect_err("the file cannot be written");

        assert!(error.to_string().contains("01.zhang"), "{error}");
        assert!(root.join("data/2024/01.zhang/inside").is_dir(), "what was in the file's place stays");
        assert_eq!(
            temporary_files(&root.join("data/2024")),
            Vec::<String>::new(),
            "no temporary file is left behind"
        );
        assert_eq!(std::fs::read_to_string(root.join("main.zhang")).unwrap(), OPENS, "nothing was included");
    }

    /// A save refuses a file this process may not write, as the plain write before it did: renaming a temporary file
    /// over a read-only file, which the directory allows, would replace it without asking. The file stays as it was
    #[cfg(unix)]
    #[test]
    fn a_save_refuses_a_file_it_may_not_write() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let main = root.join("main.zhang");
        std::fs::write(&main, OPENS).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();
        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::OpenOptions::new().write(true).open(&main).is_ok() {
            eprintln!("skipping: this user writes a read-only file (root)");
            return;
        }

        let error = ledger
            .data_source
            .save(&ledger, "main.zhang".to_owned(), b"")
            .expect_err("a read-only file is not replaced");

        assert!(matches!(&error, ZhangError::WriteRefused(path) if path == "main.zhang"), "{error:?}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), OPENS, "the file is as it was");
        assert_eq!(std::fs::metadata(&main).unwrap().permissions().mode() & 0o777, 0o444);
        assert_eq!(temporary_files(&root), Vec::<String>::new(), "no temporary file is left behind");
    }

    /// The temporary file is made with `create_new`: whatever is planted at a name it tries, a symbolic link to
    /// another file or a file, is left alone, never followed, opened or truncated, and the next name is tried; with
    /// every name taken, nothing is made
    #[cfg(unix)]
    #[test]
    fn a_temporary_file_never_opens_what_is_planted_at_its_name() {
        use std::io::Write;

        use super::temporary_file;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let victim = root.join("victim.zhang");
        std::fs::write(&victim, OPENS).unwrap();
        std::os::unix::fs::symlink(&victim, root.join(".main.zhang.link.tmp")).unwrap();
        std::fs::write(root.join(".main.zhang.file.tmp"), "planted").unwrap();
        let mut names = [".main.zhang.link.tmp", ".main.zhang.file.tmp", ".main.zhang.free.tmp"].into_iter();

        let (mut file, temp) = temporary_file(&root, true, || names.next().unwrap().to_owned()).unwrap();
        file.write_all(b"new content").unwrap();
        drop(file);

        assert_eq!(temp, root.join(".main.zhang.free.tmp"), "the first name nothing holds");
        assert_eq!(std::fs::read_to_string(&temp).unwrap(), "new content");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), OPENS, "the link was not followed");
        assert_eq!(std::fs::read_link(root.join(".main.zhang.link.tmp")).unwrap(), victim, "the link stays");
        assert_eq!(std::fs::read_to_string(root.join(".main.zhang.file.tmp")).unwrap(), "planted");

        let taken = temporary_file(&root, true, || ".main.zhang.file.tmp".to_owned()).expect_err("every name is taken");
        assert_eq!(taken.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(root.join(".main.zhang.file.tmp")).unwrap(), "planted");
    }

    /// The name of a temporary file tells whose it is and cannot be guessed: a hash with random keys, another one on
    /// every call, where it was the process id and a counter; a long file name is cut, so the name stays a valid one
    #[test]
    fn a_temporary_name_cannot_be_guessed() {
        use super::temporary_name;

        let names: Vec<String> = (0..64).map(|_| temporary_name("main.zhang")).collect();
        for name in &names {
            let digits = name
                .strip_prefix(".main.zhang.")
                .and_then(|it| it.strip_suffix(".tmp"))
                .unwrap_or_else(|| panic!("{name}"));
            assert_eq!(digits.len(), 16, "{name}");
            assert!(digits.chars().all(|it| it.is_ascii_hexdigit()), "{name}");
        }
        let mut distinct = names.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), names.len(), "every call names another file");
        let long = temporary_name(&"a".repeat(300));
        assert_eq!(long.len(), 1 + 64 + 1 + 16 + 4, "{long}");
        // cut by bytes, at a character boundary: 16 emoji of 4 bytes, 21 euro signs of 3
        let emoji = temporary_name(&"\u{1F600}".repeat(61));
        assert!(emoji.starts_with(&format!(".{}.", "\u{1F600}".repeat(16))), "{emoji}");
        assert_eq!(emoji.len(), 1 + 64 + 1 + 16 + 4, "{emoji}");
        let euro = temporary_name(&"\u{20AC}".repeat(30));
        assert!(euro.starts_with(&format!(".{}.", "\u{20AC}".repeat(21))), "{euro}");
        assert_eq!(euro.len(), 1 + 63 + 1 + 16 + 4, "{euro}");
    }

    /// A file whose name fills the 255 bytes a name may hold, in characters of several bytes, is saved: its temporary
    /// name is cut by bytes, and stays a valid name too
    #[test]
    fn a_file_with_a_long_name_of_wide_characters_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // 60 emoji of 4 bytes and the extension: 246 bytes
        let name = format!("{}.zhang", "\u{1F600}".repeat(60));
        assert_eq!(name.len(), 246);
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();
        std::fs::write(root.join(&name), "").unwrap();

        ledger.data_source.save(&ledger, name.clone(), OPENS.as_bytes()).expect("the file is saved");

        assert_eq!(std::fs::read_to_string(root.join(&name)).unwrap(), OPENS);
        assert_eq!(temporary_files(&root), Vec::<String>::new(), "no temporary file is left behind");
    }

    /// A file with more than one hard link, as a sync tool or a backup may make it, is written in place: a rename
    /// would give this name a new file and leave the other names with the old content
    #[cfg(unix)]
    #[test]
    fn a_file_with_hard_links_is_written_in_place() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("ledger")).unwrap();
        std::fs::write(root.join("ledger/main.zhang"), OPENS).unwrap();
        std::fs::hard_link(root.join("ledger/main.zhang"), root.join("mirror.zhang")).unwrap();
        let before = std::fs::metadata(root.join("ledger/main.zhang")).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.join("ledger"), "main.zhang".to_owned(), source).unwrap();

        ledger.data_source.append(&ledger, coffee()).unwrap();

        let after = std::fs::metadata(root.join("ledger/main.zhang")).unwrap();
        assert_eq!(after.ino(), before.ino(), "the file is written in place");
        assert_eq!(after.nlink(), 2, "both names stay one file");
        assert!(
            std::fs::read_to_string(root.join("mirror.zhang"))
                .unwrap()
                .contains("include \"data/2024/01.zhang\""),
            "the other name has the new content too"
        );
        assert_eq!(temporary_files(&root.join("ledger")), Vec::<String>::new(), "no temporary file is left behind");
    }

    /// A missing `include` counts as the include of the file it looks for as the load resolved it, against the file
    /// holding the `include`: one in `sub/x.zhang` naming `data/2024/01.zhang` looks for `sub/data/2024/01.zhang`, so
    /// an append to `data/2024/01.zhang` still includes it from the main file, and the entry is in the ledger reloaded
    #[test]
    fn a_missing_include_counts_by_the_path_it_resolves_to() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("main.zhang"), format!("{OPENS}include \"sub/x.zhang\"\n")).unwrap();
        std::fs::write(root.join("sub/x.zhang"), "include \"data/2024/01.zhang\"\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_owned(), source).unwrap();
        assert_eq!(
            include_errors(&ledger),
            vec![("data/2024/01.zhang".to_owned(), "include \"data/2024/01.zhang\"".to_owned())]
        );
        assert_eq!(ledger.missing_files, vec![PathBuf::from("sub/data/2024/01.zhang")]);

        let written = ledger.data_source.append_files(&ledger, coffee()).unwrap();

        assert_eq!(written, vec!["main.zhang".to_owned(), "data/2024/01.zhang".to_owned()]);
        assert_eq!(
            std::fs::read_to_string(root.join("main.zhang"))
                .unwrap()
                .matches("include \"data/2024/01.zhang\"")
                .count(),
            1,
            "the main file includes the file appended to"
        );
        assert!(!root.join("sub/data/2024/01.zhang").exists());
        let reloaded = Ledger::load_with_data_source(root, "main.zhang".to_owned(), ledger.data_source.clone()).unwrap();
        assert_eq!(reloaded.transactions().len(), 1, "the entry appended is in the ledger");
        assert_eq!(include_errors(&reloaded).len(), 1, "the include in sub/x.zhang still looks for its own file");
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
