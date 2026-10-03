//! Read-only file access for plugins: the `allowed_paths` capability behind the `zhang_read_file` and
//! `zhang_list_dir` host functions (see [`crate::plugin::host`]).
//!
//! A plugin names files relative to the ledger root, and every call goes through these rules before any I/O:
//! 1. [`clean_path`]: an empty, absolute path, a `..` component, a NUL or a backslash is rejected; `.` and empty
//!    components are dropped. The same rule validates the `allowed_paths` values themselves. `zhang_read_file`
//!    also rejects a path ending with `/` or `/.`, which names a directory.
//! 2. The path must lie in a grant, compared component by component: `documents` grants `documents/a.pdf`, not
//!    `documents-private/a.pdf`. Below the grant no component may start with `.`: a hidden file or directory is
//!    readable only when a grant names it. So `"."` grants the whole root but not `.git/config`, `.env` or
//!    `.cache/`, `".config"` grants `.config/…`, `"documents"` does not grant `documents/.secret`, and
//!    `"documents/.receipts"` grants exactly that directory. A listing leaves out the entries this rule denies.
//!    The rule is about the paths a plugin names: a symlink the ledger's owner made inside a grant may still lead
//!    to a hidden file inside it.
//! 3. On the local disk ([`DataSource::local_root`]) the file is opened through a [`cap_std::fs::Dir`] handle for
//!    the grant, itself opened through a handle for the ledger root. A symlink is followed only while it stays
//!    inside that handle, checked as the path is opened, so no symlink, `..` or absolute target leads outside it,
//!    and nothing changing between a check and the open can make it. The handle for a granted file is the
//!    directory holding it, so a granted file that is a symlink resolves anywhere inside that directory; a granted
//!    directory that is a symlink resolves anywhere inside the ledger root, never outside it. Only the ledger's
//!    owner makes such links, so they mean what the owner intended. A hard link inside a grant is read like any
//!    file; making one takes write access to the ledger. Files are opened without blocking and must be regular
//!    files, so a FIFO cannot stall the load.
//! 4. A remote source is read through [`DataSource::get_limited`] and [`DataSource::list`]. Its server may follow
//!    links of its own (the GitHub contents API follows symlinks to files inside the repository, and a WebDAV
//!    server may follow server-side links), but what it serves stays inside the remote root: the repository or the
//!    bucket. Its failures reach the plugin as a generic message, since the source's own can name the bucket, the
//!    endpoint or response headers; the host logs them instead.
//! 5. A file larger than [`MAX_FILE_SIZE`] or a directory with more than [`MAX_DIR_ENTRIES`] entries is
//!    `too_large`. A remote source checks the size before downloading.
//!
//! Every outcome is a value ([`FileError`] on failure), never a trap. A call that passes rules 1 and 2 records the
//! file or directory as an [`ExtraInput`], whatever the outcome, so changing a file a plugin read, or creating one
//! it missed, makes the ledger stale. Reading changes nothing on disk, and file watchers do not report reads, so it
//! cannot make the ledger reload itself.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use log::warn;
use serde::Serialize;

use crate::data_source::{DataSource, SourceEntry};
use crate::inputs::{normalize_relative, ExtraInput};
use crate::ZhangError;

/// the largest file a plugin can read, 16 MiB
pub const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// the most entries a directory a plugin lists may have
pub const MAX_DIR_ENTRIES: usize = 10_000;

/// why a path a plugin or its directive wrote is not acceptable (rule 1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Nul,
    Backslash,
    Absolute,
    ParentDir,
}

impl PathError {
    /// a path leading outside the ledger root is denied; a path that is not well-formed is invalid
    fn kind(self) -> FileErrorKind {
        match self {
            PathError::Absolute | PathError::ParentDir => FileErrorKind::Denied,
            PathError::Empty | PathError::Nul | PathError::Backslash => FileErrorKind::Invalid,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            PathError::Empty => "the path is empty",
            PathError::Nul => "the path holds a NUL character",
            PathError::Backslash => "the path holds a backslash; separate components with `/`",
            PathError::Absolute => "the path is absolute; name files relative to the ledger root",
            PathError::ParentDir => "the path holds a `..` component",
        }
    }
}

/// `raw`, a path relative to the ledger root, normalized lexically (rule 1): `.` and empty components are dropped,
/// so `"."` is the root itself (an empty path). It is rejected when empty, absolute, or when it holds a `..`
/// component, a NUL or a backslash.
pub fn clean_path(raw: &str) -> Result<PathBuf, PathError> {
    if raw.is_empty() {
        return Err(PathError::Empty);
    }
    if raw.contains('\0') {
        return Err(PathError::Nul);
    }
    if raw.contains('\\') {
        return Err(PathError::Backslash);
    }
    let path = Path::new(raw);
    if path.has_root() || path.components().any(|it| matches!(it, Component::RootDir | Component::Prefix(_))) {
        return Err(PathError::Absolute);
    }
    if path.components().any(|it| it == Component::ParentDir) {
        return Err(PathError::ParentDir);
    }
    normalize_relative(path).ok_or(PathError::Absolute)
}

/// the kind of a failed file call, as the plugin receives it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileErrorKind {
    /// the plugin may not read the path: it is not granted, leads outside the grant or the ledger root, or the
    /// operating system refused it
    Denied,
    /// the path does not exist, or the source failed to read it
    NotFound,
    /// the file is larger than [`MAX_FILE_SIZE`], or the directory has more than [`MAX_DIR_ENTRIES`] entries
    TooLarge,
    /// the ledger's source cannot do this, e.g. list a directory
    Unsupported,
    /// the request is not well-formed: an unreadable path, a directory to read, a file to list
    Invalid,
}

/// a failed file call: `{"kind": "...", "message": "..."}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileError {
    pub kind: FileErrorKind,
    pub message: String,
}

impl FileError {
    pub fn new(kind: FileErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    fn io(path: &str, error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let kind = match error.kind() {
            // cap-std reports a path leading outside its handle as permission denied
            ErrorKind::PermissionDenied => FileErrorKind::Denied,
            ErrorKind::IsADirectory => FileErrorKind::Invalid,
            _ => FileErrorKind::NotFound,
        };
        FileError::new(kind, format!("{path}: {error}"))
    }

    /// a remote source's failure. Its own message can name the bucket, the endpoint or response headers, so the
    /// plugin gets a generic one and the host logs the details
    fn remote(path: &str, error: ZhangError) -> Self {
        use std::io::ErrorKind;
        let (kind, message) = match &error {
            ZhangError::Unsupported(_) => (FileErrorKind::Unsupported, "the ledger's source cannot list directories"),
            ZhangError::FileNotFound => (FileErrorKind::NotFound, "no such file or directory"),
            ZhangError::IoError(e) | ZhangError::FileError { e, .. } if e.kind() == ErrorKind::NotFound => {
                (FileErrorKind::NotFound, "no such file or directory")
            }
            _ => {
                warn!("a plugin could not read {path} from the ledger's source: {error}");
                (FileErrorKind::NotFound, "the ledger's source could not read it")
            }
        };
        FileError::new(kind, format!("{path}: {message}"))
    }
}

/// how [`FileContent::content`] is encoded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// the file's bytes, which are valid UTF-8
    Utf8,
    /// the file's bytes in standard base64 with padding, for a file that is not valid UTF-8, or one whose JSON
    /// escaping would more than double it (mostly control characters)
    Base64,
}

/// a file a plugin read: `{"content": "...", "encoding": "utf8" | "base64"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileContent {
    pub content: String,
    pub encoding: Encoding,
}

impl FileContent {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let bytes = match String::from_utf8(bytes) {
            Ok(content) if json_escaped_len(&content) <= 2 * content.len() => {
                return FileContent {
                    content,
                    encoding: Encoding::Utf8,
                }
            }
            Ok(content) => content.into_bytes(),
            Err(e) => e.into_bytes(),
        };
        FileContent {
            content: base64::engine::general_purpose::STANDARD.encode(bytes),
            encoding: Encoding::Base64,
        }
    }
}

/// the length of `text` escaped as a JSON string, without the quotes: a control character takes up to 6 bytes
fn json_escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

/// the kind of a listed entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
}

/// an entry of a listed directory: `{"name": "...", "kind": "file" | "dir"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedEntry {
    pub name: String,
    pub kind: EntryKind,
}

/// a directory a plugin listed: `{"entries": [...]}`, sorted by name
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirListing {
    pub entries: Vec<ListedEntry>,
}

/// the outcome of one file call and the input it recorded, if the path was granted
#[derive(Debug)]
pub struct FileCall<T> {
    pub result: Result<T, FileError>,
    pub input: Option<ExtraInput>,
}

impl<T> FileCall<T> {
    /// a call refused before any I/O, which records nothing
    pub fn rejected(error: FileError) -> Self {
        FileCall {
            result: Err(error),
            input: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    file_size: u64,
    dir_entries: usize,
}

const LIMITS: Limits = Limits {
    file_size: MAX_FILE_SIZE,
    dir_entries: MAX_DIR_ENTRIES,
};

/// what one plugin may read: the paths its `allowed_paths` grant, from the ledger's data source
#[derive(Clone)]
pub struct FileAccess {
    /// the granted paths relative to the ledger root, as [`clean_path`] returns them (empty for the whole root)
    grants: Vec<PathBuf>,
    source: Arc<dyn DataSource>,
    /// the ledger root on the local disk, if the source reads the local disk
    local_root: Option<PathBuf>,
    limits: Limits,
}

impl FileAccess {
    /// access to `grants`, cleaned with [`clean_path`], in the ledger rooted at `ledger_root` read from `source`
    pub fn new(grants: Vec<PathBuf>, source: Arc<dyn DataSource>, ledger_root: &Path) -> Self {
        let local_root = source.local_root(ledger_root);
        FileAccess {
            grants,
            source,
            local_root,
            limits: LIMITS,
        }
    }

    /// `zhang_read_file(path)`
    pub fn read_file(&self, raw: &str) -> FileCall<FileContent> {
        let (path, grant) = match self.check(raw) {
            Ok(granted) => granted,
            Err(error) => return FileCall::rejected(error),
        };
        // a path ending with `/` or `/.` names a directory, as `.` does
        if matches!(raw.rsplit('/').next(), Some("" | ".")) {
            return FileCall::rejected(FileError::new(
                FileErrorKind::Invalid,
                format!("{raw:?} names a directory; list it with zhang_list_dir, or name the file without a trailing `/`"),
            ));
        }
        let name = display(&path);
        let bytes = match &self.local_root {
            Some(root) => read_local(root, grant, &path, &name, self.limits),
            None => read_remote(self.source.as_ref(), &name, self.limits),
        };
        FileCall {
            result: bytes.map(FileContent::from_bytes),
            input: Some(ExtraInput::File(path)),
        }
    }

    /// `zhang_list_dir(path)`
    pub fn list_dir(&self, raw: &str) -> FileCall<DirListing> {
        let (path, grant) = match self.check(raw) {
            Ok(granted) => granted,
            Err(error) => return FileCall::rejected(error),
        };
        let name = display(&path);
        let entries = match &self.local_root {
            Some(root) => list_local(root, grant, &path, &name, self.limits),
            None => list_remote(self.source.as_ref(), &name, self.limits),
        };
        let result = entries.map(|entries| {
            let mut entries = entries
                .into_iter()
                // a hidden entry no grant names is left out (rule 2)
                .filter(|it| self.grant_for(&path.join(&it.name)).is_some())
                .map(|it| ListedEntry {
                    name: it.name,
                    kind: if it.is_dir { EntryKind::Dir } else { EntryKind::File },
                })
                .collect::<Vec<_>>();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            DirListing { entries }
        });
        FileCall {
            result,
            input: Some(ExtraInput::Dir(path)),
        }
    }

    /// rules 1 and 2: the cleaned path and the broadest grant holding it
    fn check(&self, raw: &str) -> Result<(PathBuf, &Path), FileError> {
        let path = clean_path(raw).map_err(|e| FileError::new(e.kind(), format!("{raw:?}: {}", e.reason())))?;
        match self.grant_for(&path) {
            Some(grant) => Ok((path, grant)),
            None if self.grants.iter().any(|grant| path.starts_with(grant)) => Err(FileError::new(
                FileErrorKind::Denied,
                format!("{raw:?} is hidden: a name starting with `.` is readable only when an allowed_paths value names it"),
            )),
            None => Err(FileError::new(FileErrorKind::Denied, format!("{raw:?} is not in the plugin's allowed_paths"))),
        }
    }

    /// the broadest grant holding `path` (rule 2): the path starts with it, compared component by component, and
    /// no component below it starts with `.`
    fn grant_for(&self, path: &Path) -> Option<&Path> {
        self.grants
            .iter()
            .filter(|grant| path.strip_prefix(grant).is_ok_and(|below| !below.components().any(is_hidden)))
            .min_by_key(|grant| grant.components().count())
            .map(PathBuf::as_path)
    }
}

/// whether a path component names a hidden file or directory
fn is_hidden(component: Component) -> bool {
    component.as_os_str().as_encoded_bytes().first() == Some(&b'.')
}

/// a cleaned path written with `/`, the root as `.`
fn display(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return ".".to_owned();
    }
    path.components().map(|it| it.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// the handle a path in `grant` is opened through, and the path inside it (rule 3)
fn open_grant(root: &Path, grant: &Path, path: &Path, name: &str) -> Result<(Dir, PathBuf), FileError> {
    let io = |e| FileError::io(name, e);
    let root_dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(io)?;
    if grant.as_os_str().is_empty() {
        return Ok((root_dir, path.to_path_buf()));
    }
    // opened through the root's handle, so a grant that is a symlink leading outside the root grants nothing
    let base = if root_dir.metadata(grant).map_err(io)?.is_dir() {
        grant
    } else {
        // a granted file is opened through the directory holding it
        grant.parent().unwrap_or(Path::new(""))
    };
    let dir = if base.as_os_str().is_empty() {
        root_dir
    } else {
        root_dir.open_dir(base).map_err(io)?
    };
    let inside = path.strip_prefix(base).expect("a granted path starts with its grant").to_path_buf();
    Ok((dir, inside))
}

fn read_local(root: &Path, grant: &Path, path: &Path, name: &str, limits: Limits) -> Result<Vec<u8>, FileError> {
    let io = |e| FileError::io(name, e);
    let (dir, inside) = open_grant(root, grant, path, name)?;
    if inside.as_os_str().is_empty() {
        return Err(FileError::new(FileErrorKind::Invalid, format!("{name} is a directory; list it instead")));
    }
    // what was opened is checked, not what the path named a moment before
    let file = dir.open_with(&inside, &read_options()).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if metadata.is_dir() {
        return Err(FileError::new(FileErrorKind::Invalid, format!("{name} is a directory; list it instead")));
    }
    if !metadata.is_file() {
        return Err(FileError::new(FileErrorKind::Invalid, format!("{name} is not a regular file")));
    }
    if metadata.len() > limits.file_size {
        return Err(too_large_file(name, limits));
    }
    let mut bytes = vec![];
    // bounded again while reading, in case the file grew since
    file.take(limits.file_size + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limits.file_size {
        return Err(too_large_file(name, limits));
    }
    Ok(bytes)
}

/// read-only and, on Unix, non-blocking: opening a FIFO would wait for a writer, so it is opened at once and then
/// rejected as not a regular file. Reading a regular file ignores the flag
fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options
}

fn list_local(root: &Path, grant: &Path, path: &Path, name: &str, limits: Limits) -> Result<Vec<SourceEntry>, FileError> {
    let io = |e| FileError::io(name, e);
    let (dir, inside) = open_grant(root, grant, path, name)?;
    let entries = if inside.as_os_str().is_empty() {
        dir.entries().map_err(io)?
    } else {
        if !dir.metadata(&inside).map_err(io)?.is_dir() {
            return Err(FileError::new(FileErrorKind::Invalid, format!("{name} is not a directory")));
        }
        dir.read_dir(&inside).map_err(io)?
    };
    let mut listed = vec![];
    for (index, entry) in entries.enumerate() {
        if index >= limits.dir_entries {
            return Err(too_large_dir(name, limits));
        }
        let entry = entry.map_err(io)?;
        // a name a plugin could not pass back is left out
        let Ok(entry_name) = entry.file_name().into_string() else {
            continue;
        };
        if entry_name.contains('\\') {
            continue;
        }
        let file_type = entry.file_type().map_err(io)?;
        let is_dir = if file_type.is_symlink() {
            // what the link leads to inside the grant; a link leading outside it, or nowhere, is left out
            match dir.metadata(inside.join(&entry_name)) {
                Ok(target) if target.is_dir() => true,
                Ok(target) if target.is_file() => false,
                _ => continue,
            }
        } else if file_type.is_dir() {
            true
        } else if file_type.is_file() {
            false
        } else {
            // sockets, FIFOs and devices cannot be read
            continue;
        };
        listed.push(SourceEntry { name: entry_name, is_dir });
    }
    Ok(listed)
}

fn read_remote(source: &dyn DataSource, name: &str, limits: Limits) -> Result<Vec<u8>, FileError> {
    let bytes = match source.get_limited(name.to_owned(), limits.file_size) {
        Ok(bytes) => bytes,
        Err(ZhangError::TooLarge(_)) => return Err(too_large_file(name, limits)),
        Err(error) => return Err(FileError::remote(name, error)),
    };
    // checked again, whatever the source did
    if bytes.len() as u64 > limits.file_size {
        return Err(too_large_file(name, limits));
    }
    Ok(bytes)
}

fn list_remote(source: &dyn DataSource, name: &str, limits: Limits) -> Result<Vec<SourceEntry>, FileError> {
    let path = if name == "." { String::new() } else { name.to_owned() };
    let entries = match source.list(path, limits.dir_entries) {
        Ok(entries) => entries,
        Err(ZhangError::TooLarge(_)) => return Err(too_large_dir(name, limits)),
        Err(error) => return Err(FileError::remote(name, error)),
    };
    if entries.len() > limits.dir_entries {
        return Err(too_large_dir(name, limits));
    }
    Ok(entries)
}

fn too_large_file(name: &str, limits: Limits) -> FileError {
    FileError::new(
        FileErrorKind::TooLarge,
        format!("{name} is larger than {} bytes, the most a plugin can read", limits.file_size),
    )
}

fn too_large_dir(name: &str, limits: Limits) -> FileError {
    FileError::new(
        FileErrorKind::TooLarge,
        format!("{name} has more than {} entries, the most a plugin can list", limits.dir_entries),
    )
}

#[cfg(test)]
mod test {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::{clean_path, EntryKind, FileAccess, FileContent, FileErrorKind, Limits, ListedEntry, PathError};
    use crate::data_source::{DataSource, LocalFileSystemDataSource, SourceEntry};
    use crate::data_type::text::ZhangDataType;
    use crate::inputs::ExtraInput;
    use crate::{ZhangError, ZhangResult};

    /// a ledger root holding `documents/receipt.txt`, `documents/2024/a.csv`, `documents-private/secret.txt`,
    /// `statements/2024.csv` and `statements/2025.csv`
    fn ledger() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (file, content) in [
            ("documents/receipt.txt", "lunch 10 CNY"),
            ("documents/2024/a.csv", "a,b"),
            ("documents-private/secret.txt", "secret"),
            ("statements/2024.csv", "2024"),
            ("statements/2025.csv", "2025"),
        ] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    fn local(root: &Path, grants: &[&str]) -> FileAccess {
        let grants = grants.iter().map(|it| clean_path(it).unwrap()).collect();
        FileAccess::new(grants, Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})), root)
    }

    fn read(access: &FileAccess, path: &str) -> Result<String, FileErrorKind> {
        access.read_file(path).result.map(|it| it.content).map_err(|it| it.kind)
    }

    fn list(access: &FileAccess, path: &str) -> Result<Vec<(String, EntryKind)>, FileErrorKind> {
        access
            .list_dir(path)
            .result
            .map(|it| it.entries.into_iter().map(|ListedEntry { name, kind }| (name, kind)).collect())
            .map_err(|it| it.kind)
    }

    #[test]
    fn should_clean_relative_paths_lexically() {
        for (raw, cleaned) in [
            ("documents", "documents"),
            ("./documents//2024/", "documents/2024"),
            ("documents/./receipt.txt", "documents/receipt.txt"),
            (".", ""),
            ("./", ""),
        ] {
            assert_eq!(clean_path(raw), Ok(PathBuf::from(cleaned)), "{raw:?}");
        }
    }

    #[test]
    fn should_reject_paths_breaking_rule_one() {
        for (raw, error) in [
            ("", PathError::Empty),
            ("/etc/passwd", PathError::Absolute),
            ("//documents", PathError::Absolute),
            ("..", PathError::ParentDir),
            ("../outside.txt", PathError::ParentDir),
            ("documents/../documents-private/secret.txt", PathError::ParentDir),
            ("documents/..", PathError::ParentDir),
            ("documents\\receipt.txt", PathError::Backslash),
            ("..\\outside.txt", PathError::Backslash),
            ("documents/receipt.txt\0.png", PathError::Nul),
        ] {
            assert_eq!(clean_path(raw), Err(error), "{raw:?}");
        }
    }

    #[test]
    fn should_deny_every_rule_one_violation_before_any_io() {
        let dir = ledger();
        let access = local(dir.path(), &["."]);

        for (raw, kind) in [
            ("../outside.txt", FileErrorKind::Denied),
            ("/etc/passwd", FileErrorKind::Denied),
            ("documents/../statements/2024.csv", FileErrorKind::Denied),
            ("documents\\receipt.txt", FileErrorKind::Invalid),
            ("documents/receipt.txt\0", FileErrorKind::Invalid),
        ] {
            let call = access.read_file(raw);
            assert_eq!(call.result.map_err(|it| it.kind), Err(kind), "{raw:?}");
            assert_eq!(call.input, None, "{raw:?} records nothing");
        }
    }

    #[test]
    fn should_match_grants_component_by_component() {
        let dir = ledger();
        let access = local(dir.path(), &["documents"]);

        assert_eq!(read(&access, "documents/receipt.txt"), Ok("lunch 10 CNY".to_owned()));
        assert_eq!(read(&access, "./documents//2024/a.csv"), Ok("a,b".to_owned()));
        assert_eq!(read(&access, "documents-private/secret.txt"), Err(FileErrorKind::Denied));
        assert_eq!(read(&access, "statements/2024.csv"), Err(FileErrorKind::Denied));
        assert_eq!(list(&access, "."), Err(FileErrorKind::Denied), "the root holds the grant, it is not in it");
    }

    #[test]
    fn should_deny_everything_without_grants() {
        let dir = ledger();
        let access = local(dir.path(), &[]);

        let call = access.read_file("documents/receipt.txt");
        let error = call.result.unwrap_err();
        assert_eq!(error.kind, FileErrorKind::Denied);
        assert!(error.message.contains("allowed_paths"), "{}", error.message);
        assert_eq!(call.input, None);
        assert_eq!(list(&access, "."), Err(FileErrorKind::Denied));
    }

    #[test]
    fn should_grant_a_single_file_only() {
        let dir = ledger();
        let access = local(dir.path(), &["statements/2024.csv"]);

        assert_eq!(read(&access, "statements/2024.csv"), Ok("2024".to_owned()));
        assert_eq!(read(&access, "statements/2025.csv"), Err(FileErrorKind::Denied));
        assert_eq!(read(&access, "statements/2024.csv/more"), Err(FileErrorKind::NotFound));
        assert_eq!(list(&access, "statements/2024.csv"), Err(FileErrorKind::Invalid));
        assert_eq!(list(&access, "statements"), Err(FileErrorKind::Denied));
    }

    #[test]
    fn should_grant_the_whole_root_with_a_dot() {
        let dir = ledger();
        let access = local(dir.path(), &["."]);

        assert_eq!(read(&access, "documents-private/secret.txt"), Ok("secret".to_owned()));
        assert_eq!(
            list(&access, "."),
            Ok(vec![
                ("documents".to_owned(), EntryKind::Dir),
                ("documents-private".to_owned(), EntryKind::Dir),
                ("statements".to_owned(), EntryKind::Dir),
            ])
        );
        assert_eq!(access.list_dir(".").input, Some(ExtraInput::Dir(PathBuf::new())));
    }

    #[test]
    fn should_record_every_call_on_a_granted_path() {
        let dir = ledger();
        let access = local(dir.path(), &["documents"]);

        let read = access.read_file("./documents//receipt.txt");
        assert!(read.result.is_ok());
        assert_eq!(read.input, Some(ExtraInput::File("documents/receipt.txt".into())));

        // a receipt the plugin missed reloads the ledger once it is added
        let missing = access.read_file("documents/missing.pdf");
        assert_eq!(missing.result.unwrap_err().kind, FileErrorKind::NotFound);
        assert_eq!(missing.input, Some(ExtraInput::File("documents/missing.pdf".into())));

        let listed = access.list_dir("documents/");
        assert_eq!(
            listed.result.unwrap().entries,
            vec![
                ListedEntry {
                    name: "2024".to_owned(),
                    kind: EntryKind::Dir
                },
                ListedEntry {
                    name: "receipt.txt".to_owned(),
                    kind: EntryKind::File
                },
            ]
        );
        assert_eq!(listed.input, Some(ExtraInput::Dir("documents".into())));

        assert_eq!(access.read_file("documents-private/secret.txt").input, None);
    }

    #[test]
    fn should_reject_a_trailing_slash_when_reading_but_not_when_listing() {
        let dir = ledger();
        let access = local(dir.path(), &["documents"]);

        for path in ["documents/receipt.txt/", "documents/receipt.txt/.", "documents/2024/"] {
            let call = access.read_file(path);
            assert_eq!(call.result.map_err(|it| it.kind), Err(FileErrorKind::Invalid), "{path}");
            assert_eq!(call.input, None, "{path}");
        }
        assert_eq!(list(&access, "documents/2024/"), Ok(vec![("a.csv".to_owned(), EntryKind::File)]));
        assert_eq!(list(&access, "documents/2024/."), Ok(vec![("a.csv".to_owned(), EntryKind::File)]));
    }

    /// `ledger()` plus hidden files: `.env`, `.git/config`, `.cache/plugins/x.wasm`, `.config/app.toml`,
    /// `.config/.nested`, `documents/.secret` and `documents/.receipts/a.pdf`
    fn ledger_with_hidden_files() -> TempDir {
        let dir = ledger();
        for file in [
            ".env",
            ".git/config",
            ".cache/plugins/x.wasm",
            ".config/app.toml",
            ".config/.nested",
            "documents/.secret",
            "documents/.receipts/a.pdf",
        ] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file).unwrap();
        }
        dir
    }

    #[test]
    fn should_hide_dot_entries_a_grant_does_not_name() {
        let dir = ledger_with_hidden_files();
        let root = local(dir.path(), &["."]);

        for path in [".env", ".git/config", ".cache/plugins/x.wasm", "documents/.secret", "documents/.receipts/a.pdf"] {
            let call = root.read_file(path);
            let error = call.result.unwrap_err();
            assert_eq!(error.kind, FileErrorKind::Denied, "{path}");
            assert!(error.message.contains("hidden"), "{path}: {}", error.message);
            assert_eq!(call.input, None, "{path}");
        }
        assert_eq!(list(&root, ".git"), Err(FileErrorKind::Denied));
        assert_eq!(
            list(&root, "."),
            Ok(vec![
                ("documents".to_owned(), EntryKind::Dir),
                ("documents-private".to_owned(), EntryKind::Dir),
                ("statements".to_owned(), EntryKind::Dir),
            ])
        );
        assert_eq!(
            list(&root, "documents"),
            Ok(vec![("2024".to_owned(), EntryKind::Dir), ("receipt.txt".to_owned(), EntryKind::File)])
        );
    }

    #[test]
    fn should_grant_a_dot_entry_a_grant_names() {
        let dir = ledger_with_hidden_files();

        let config = local(dir.path(), &[".config"]);
        assert_eq!(read(&config, ".config/app.toml"), Ok(".config/app.toml".to_owned()));
        assert_eq!(read(&config, ".config/.nested"), Err(FileErrorKind::Denied), "hidden below the grant");
        assert_eq!(list(&config, ".config"), Ok(vec![("app.toml".to_owned(), EntryKind::File)]));

        let receipts = local(dir.path(), &["documents/.receipts"]);
        assert_eq!(read(&receipts, "documents/.receipts/a.pdf"), Ok("documents/.receipts/a.pdf".to_owned()));
        assert_eq!(read(&receipts, "documents/receipt.txt"), Err(FileErrorKind::Denied));

        // a listing shows a hidden entry another grant names
        let both = local(dir.path(), &[".", ".config"]);
        assert_eq!(
            list(&both, ".").unwrap().into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
            vec![".config", "documents", "documents-private", "statements"]
        );
    }

    #[test]
    fn should_hide_dot_entries_of_a_remote_source_too() {
        let access = remote(
            Remote {
                files: vec![(".env", b"TOKEN=1".to_vec())],
                entries: Some(vec![
                    SourceEntry {
                        name: ".git".to_owned(),
                        is_dir: true,
                    },
                    SourceEntry {
                        name: "main.zhang".to_owned(),
                        is_dir: false,
                    },
                ]),
            },
            &["."],
        );

        assert_eq!(read(&access, ".env"), Err(FileErrorKind::Denied));
        assert_eq!(list(&access, "."), Ok(vec![("main.zhang".to_owned(), EntryKind::File)]));
    }

    #[test]
    fn should_reject_reading_a_directory_and_listing_a_file() {
        let dir = ledger();
        let access = local(dir.path(), &["."]);

        assert_eq!(read(&access, "documents"), Err(FileErrorKind::Invalid));
        assert_eq!(read(&access, "."), Err(FileErrorKind::Invalid));
        assert_eq!(access.read_file(".").input, None);
        assert_eq!(list(&access, "documents/receipt.txt"), Err(FileErrorKind::Invalid));
        assert_eq!(list(&access, "missing"), Err(FileErrorKind::NotFound));
    }

    #[test]
    fn should_return_text_as_utf8_and_other_bytes_as_base64() {
        let dir = ledger();
        std::fs::write(dir.path().join("documents/scan.bin"), [0xff, 0x00, 0xfe]).unwrap();
        std::fs::write(dir.path().join("documents/note.txt"), "收据 ✓").unwrap();
        let access = local(dir.path(), &["documents"]);

        assert_eq!(
            access.read_file("documents/note.txt").result,
            Ok(FileContent {
                content: "收据 ✓".to_owned(),
                encoding: super::Encoding::Utf8
            })
        );
        assert_eq!(
            access.read_file("documents/scan.bin").result,
            Ok(FileContent {
                content: "/wD+".to_owned(),
                encoding: super::Encoding::Base64
            })
        );
        let json = serde_json::to_value(access.read_file("documents/scan.bin").result).unwrap();
        assert_eq!(json, serde_json::json!({"Ok": {"content": "/wD+", "encoding": "base64"}}));
    }

    #[test]
    fn should_use_base64_for_text_whose_escaping_would_more_than_double_it() {
        assert_eq!(super::json_escaped_len("plain"), 5);
        assert_eq!(super::json_escaped_len("a\n\"\\"), 7);
        assert_eq!(super::json_escaped_len("\u{1}\u{1f}"), 12);
        assert_eq!(super::json_escaped_len("收"), 3);

        let control = FileContent::from_bytes(vec![1, 2, 3, b'a']);
        assert_eq!(control.encoding, super::Encoding::Base64);
        assert_eq!(control.content, "AQIDYQ==");
        // escaping every byte to two still fits
        assert_eq!(FileContent::from_bytes(b"\n\n\t".to_vec()).encoding, super::Encoding::Utf8);
        assert_eq!(FileContent::from_bytes(b"a,b\r\n".to_vec()).encoding, super::Encoding::Utf8);
    }

    #[test]
    fn should_refuse_files_and_directories_over_the_limits() {
        let dir = ledger();
        let mut access = local(dir.path(), &["documents"]);
        access.limits = Limits { file_size: 12, dir_entries: 2 };

        assert_eq!(read(&access, "documents/receipt.txt"), Ok("lunch 10 CNY".to_owned()), "exactly at the limit");
        std::fs::write(dir.path().join("documents/receipt.txt"), "lunch 100 CNY").unwrap();
        assert_eq!(read(&access, "documents/receipt.txt"), Err(FileErrorKind::TooLarge));

        assert!(list(&access, "documents").is_ok(), "two entries");
        std::fs::write(dir.path().join("documents/third.txt"), "").unwrap();
        assert_eq!(list(&access, "documents"), Err(FileErrorKind::TooLarge));
    }

    #[test]
    fn should_refuse_a_file_over_sixteen_mebibytes() {
        let dir = ledger();
        let big = std::fs::File::create(dir.path().join("documents/big.pdf")).unwrap();
        // sparse: nothing is written
        big.set_len(super::MAX_FILE_SIZE + 1).unwrap();
        let access = local(dir.path(), &["documents"]);

        let call = access.read_file("documents/big.pdf");
        assert_eq!(call.result.unwrap_err().kind, FileErrorKind::TooLarge);
        assert_eq!(call.input, Some(ExtraInput::File("documents/big.pdf".into())));
    }

    #[cfg(unix)]
    mod symlinks {
        use std::os::unix::fs::symlink;

        use super::{ledger, list, local, read};
        use crate::plugin::files::{EntryKind, FileErrorKind};

        #[test]
        fn should_follow_a_symlink_that_stays_inside_the_grant() {
            let dir = ledger();
            symlink("receipt.txt", dir.path().join("documents/latest.txt")).unwrap();
            symlink("2024", dir.path().join("documents/current")).unwrap();
            let access = local(dir.path(), &["documents"]);

            assert_eq!(read(&access, "documents/latest.txt"), Ok("lunch 10 CNY".to_owned()));
            assert_eq!(read(&access, "documents/current/a.csv"), Ok("a,b".to_owned()));
            assert_eq!(
                list(&access, "documents"),
                Ok(vec![
                    ("2024".to_owned(), EntryKind::Dir),
                    ("current".to_owned(), EntryKind::Dir),
                    ("latest.txt".to_owned(), EntryKind::File),
                    ("receipt.txt".to_owned(), EntryKind::File),
                ])
            );
            assert_eq!(list(&access, "documents/current"), Ok(vec![("a.csv".to_owned(), EntryKind::File)]));
        }

        #[test]
        fn should_deny_a_symlink_leading_outside_the_grant() {
            let dir = ledger();
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("passwd"), "root").unwrap();
            // into the ledger root but out of the grant, relative and absolute; and out of the ledger root
            symlink("../documents-private/secret.txt", dir.path().join("documents/relative.txt")).unwrap();
            symlink(dir.path().join("documents-private/secret.txt"), dir.path().join("documents/absolute.txt")).unwrap();
            symlink(outside.path().join("passwd"), dir.path().join("documents/passwd")).unwrap();
            symlink(outside.path(), dir.path().join("documents/outside")).unwrap();
            let access = local(dir.path(), &["documents"]);

            for path in [
                "documents/relative.txt",
                "documents/absolute.txt",
                "documents/passwd",
                "documents/outside/passwd",
            ] {
                assert_eq!(read(&access, path), Err(FileErrorKind::Denied), "{path}");
            }
            assert_eq!(list(&access, "documents/outside"), Err(FileErrorKind::Denied));
            // listed only when the link stays inside the grant
            assert_eq!(
                list(&access, "documents"),
                Ok(vec![("2024".to_owned(), EntryKind::Dir), ("receipt.txt".to_owned(), EntryKind::File)])
            );
        }

        #[test]
        fn should_deny_a_grant_that_is_a_symlink_leading_outside_the_root() {
            let dir = ledger();
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("passwd"), "root").unwrap();
            symlink(outside.path(), dir.path().join("linked")).unwrap();
            symlink(outside.path().join("passwd"), dir.path().join("passwd")).unwrap();
            let access = local(dir.path(), &["linked", "passwd"]);

            assert_eq!(read(&access, "linked/passwd"), Err(FileErrorKind::Denied));
            assert_eq!(list(&access, "linked"), Err(FileErrorKind::Denied));
            assert_eq!(read(&access, "passwd"), Err(FileErrorKind::Denied));
        }

        #[test]
        fn should_reject_a_fifo_without_blocking_on_it() {
            let dir = ledger();
            let fifo = dir.path().join("documents/pipe");
            let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0, "mkfifo");
            let access = local(dir.path(), &["documents"]);

            // nothing ever writes to the FIFO: a blocking open would wait forever
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let call = access.read_file("documents/pipe");
                sender.send(call.result.map_err(|it| (it.kind, it.message))).ok();
            });
            let result = receiver
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("reading a FIFO returns at once");
            let (kind, message) = result.unwrap_err();
            assert_eq!(kind, FileErrorKind::Invalid);
            assert!(message.contains("not a regular file"), "{message}");
        }

        #[test]
        fn should_follow_a_symlinked_file_grant_inside_its_directory() {
            let dir = ledger();
            symlink("2025.csv", dir.path().join("statements/latest.csv")).unwrap();
            symlink("../documents/receipt.txt", dir.path().join("statements/receipt.txt")).unwrap();
            let access = local(dir.path(), &["statements/latest.csv", "statements/receipt.txt"]);

            assert_eq!(read(&access, "statements/latest.csv"), Ok("2025".to_owned()));
            assert_eq!(read(&access, "statements/receipt.txt"), Err(FileErrorKind::Denied));
        }
    }

    /// a remote source holding `files`, listing directories only when `entries` is set
    struct Remote {
        files: Vec<(&'static str, Vec<u8>)>,
        entries: Option<Vec<SourceEntry>>,
    }

    impl DataSource for Remote {
        fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
            self.files
                .iter()
                .find(|(name, _)| *name == path)
                .map(|(_, content)| content.clone())
                .ok_or(ZhangError::FileNotFound)
        }

        fn list(&self, path: String, max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
            match &self.entries {
                Some(entries) if entries.len() > max_entries => Err(ZhangError::TooLarge(path)),
                Some(entries) => Ok(entries.clone()),
                None => Err(ZhangError::Unsupported(format!("listing the directory {path:?}"))),
            }
        }
    }

    /// a remote source failing with the kind of message a real one gives: it names the bucket and its endpoint
    struct Failing;

    impl DataSource for Failing {
        fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
            Err(ZhangError::CustomError(format!(
                "fail to get file content [{path}] : Unexpected (persistent) at read, context: {{ uri: \
                 https://s3.secret-bucket.example.com/ledger/{path}, x-amz-request-id: 4F2A }}"
            )))
        }

        fn list(&self, path: String, _max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
            Err(ZhangError::CustomError(format!("fail to list [{path}] : bucket secret-bucket, root /ledger")))
        }
    }

    #[test]
    fn should_hide_the_details_of_a_remote_failure() {
        let access = FileAccess::new(vec![clean_path("documents").unwrap()], Arc::new(Failing), Path::new("/"));

        for error in [
            access.read_file("documents/receipt.txt").result.unwrap_err(),
            access.list_dir("documents").result.unwrap_err(),
        ] {
            assert_eq!(error.kind, FileErrorKind::NotFound);
            assert!(!error.message.contains("secret-bucket"), "{}", error.message);
            assert!(!error.message.contains("x-amz"), "{}", error.message);
            assert!(error.message.contains("the ledger's source could not read it"), "{}", error.message);
        }
        let missing = remote(Remote { files: vec![], entries: None }, &["documents"]).read_file("documents/a.pdf");
        assert_eq!(missing.result.unwrap_err().message, "documents/a.pdf: no such file or directory");
    }

    fn remote(remote: Remote, grants: &[&str]) -> FileAccess {
        let grants = grants.iter().map(|it| clean_path(it).unwrap()).collect();
        FileAccess::new(grants, Arc::new(remote), Path::new("/"))
    }

    #[test]
    fn should_read_a_remote_source_through_get_with_slash_paths() {
        let access = remote(
            Remote {
                files: vec![("documents/2024/receipt.txt", b"lunch".to_vec())],
                entries: None,
            },
            &["documents"],
        );

        assert_eq!(read(&access, "./documents//2024/receipt.txt"), Ok("lunch".to_owned()));
        assert_eq!(read(&access, "documents/missing.txt"), Err(FileErrorKind::NotFound));
        assert_eq!(read(&access, "documents-private/secret.txt"), Err(FileErrorKind::Denied));
        assert_eq!(read(&access, "documents/../secret.txt"), Err(FileErrorKind::Denied));
    }

    #[test]
    fn should_say_a_remote_source_without_listing_is_unsupported() {
        let access = remote(Remote { files: vec![], entries: None }, &["documents"]);

        let call = access.list_dir("documents");
        assert_eq!(call.result.unwrap_err().kind, FileErrorKind::Unsupported);
        assert_eq!(call.input, Some(ExtraInput::Dir("documents".into())));
    }

    #[test]
    fn should_list_a_remote_source_sorted_and_limited() {
        let entries = vec![
            SourceEntry {
                name: "b.pdf".to_owned(),
                is_dir: false,
            },
            SourceEntry {
                name: "a".to_owned(),
                is_dir: true,
            },
        ];
        let mut access = remote(
            Remote {
                files: vec![("documents/big.pdf", vec![b'x'; 3])],
                entries: Some(entries),
            },
            &["documents"],
        );
        assert_eq!(read(&access, "documents/big.pdf"), Ok("xxx".to_owned()));

        assert_eq!(
            list(&access, "documents"),
            Ok(vec![("a".to_owned(), EntryKind::Dir), ("b.pdf".to_owned(), EntryKind::File)])
        );
        access.limits = Limits { file_size: 2, dir_entries: 1 };
        assert_eq!(list(&access, "documents"), Err(FileErrorKind::TooLarge));
        assert_eq!(read(&access, "documents/big.pdf"), Err(FileErrorKind::TooLarge));
    }

    #[test]
    fn should_not_use_a_local_root_for_a_remote_source() {
        let source = Remote { files: vec![], entries: None };
        assert_eq!(source.local_root(Path::new("/ledger")), None);
        let local = LocalFileSystemDataSource::new(ZhangDataType {});
        assert_eq!(local.local_root(Path::new("/ledger")), Some(PathBuf::from("/ledger")));
    }
}
