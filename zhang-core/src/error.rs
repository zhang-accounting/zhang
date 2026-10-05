use std::net::AddrParseError;
use std::path::{Path, PathBuf};

use thiserror::Error;
use zhang_ast::SpanInfo;

#[derive(Debug, Error)]
pub enum ZhangError {
    #[error("date is invalid")]
    InvalidDate,

    #[error("option value is invalid")]
    InvalidOptionValue,

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("fetch error")]
    FetchError,
    #[error("error on file operation[{path}]: {e}")]
    FileError { e: std::io::Error, path: PathBuf },
    #[error("ip addr error: {0}")]
    IpAddrError(#[from] AddrParseError),

    #[error("cannot parse {path}: {msg}")]
    PestError { path: String, msg: String },
    #[error("Process Error: {kind} \n file: {:?}[{}:{}] \n content: {}", span.filename,span.start, span.end, span.content)]
    ProcessError { span: SpanInfo, kind: zhang_ast::error::ErrorKind },

    #[error("cannot found option given key: {0}")]
    OptionNotFound(String),

    /// a file of the ledger whose content is not UTF-8 text: it is read neither as it is nor with its bytes replaced,
    /// so nothing is loaded from it, and nothing is written into it ([`FileText::decode`](crate::data_source::FileText::decode))
    #[error("the file {path} is not UTF-8 text: line {line} holds a byte that is not UTF-8. Save the file with the UTF-8 encoding")]
    InvalidUtf8 { path: String, line: usize },

    #[error("file not found")]
    FileNotFound,

    /// a main file whose extension tells no format a ledger has ([`Dialect::of`](crate::data_type::Dialect::of))
    #[error("cannot tell the format of the ledger from its main file {0}: name it with the extension .zhang for a zhang ledger, or .bean, .beancount or .bc for a beancount one")]
    UnknownLedgerFormat(String),

    #[error("custom error: {0}")]
    CustomError(String),

    /// an operation the data source does not support, e.g. listing a directory
    #[error("not supported by this data source: {0}")]
    Unsupported(String),

    /// a file or directory larger than the caller allows
    #[error("too large: {0}")]
    TooLarge(String),

    /// a file a writer edits in place changed since the ledger was loaded: the places of its directives are stale, and
    /// nothing is written
    #[error("the file {0} changed since the ledger was loaded, so nothing was written: try again, on the ledger reloaded")]
    FileChanged(String),
    /// the storage of the ledger refused to read the file at this path, as a scoped access policy or an expired token
    /// makes it: whether the file is there is not known
    #[error("the storage refused to read {0}")]
    ReadRefused(String),
    /// the storage of the ledger refused to write the file at this path, as a read-only mount, file permissions or a
    /// scoped access policy make it: nothing was written there
    #[error("the storage refused to write {0}")]
    WriteRefused(String),
}

/// Whether a storage error was met reading or writing a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// What a storage said of a failed access, whatever the storage (the local disk, opendal's services): the file is not
/// there, the storage refused the access, or something else went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFailure {
    NotFound,
    Refused,
    Other,
}

impl From<std::io::ErrorKind> for StorageFailure {
    fn from(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::NotFound => StorageFailure::NotFound,
            std::io::ErrorKind::PermissionDenied => StorageFailure::Refused,
            _ => StorageFailure::Other,
        }
    }
}

/// The error of an `access` to the file at `path` that failed as `failure` says, `detail` being what the storage told:
/// the one mapping of every data source. A missing file is [`ZhangError::FileNotFound`], a refusal
/// [`ZhangError::ReadRefused`] or [`ZhangError::WriteRefused`], which name the file only, and anything else keeps the
/// storage's details.
pub fn storage_error(path: &str, access: Access, failure: StorageFailure, detail: impl std::fmt::Display) -> ZhangError {
    match (failure, access) {
        (StorageFailure::NotFound, _) => ZhangError::FileNotFound,
        (StorageFailure::Refused, Access::Read) => ZhangError::ReadRefused(path.to_owned()),
        (StorageFailure::Refused, Access::Write) => ZhangError::WriteRefused(path.to_owned()),
        (StorageFailure::Other, Access::Read) => ZhangError::CustomError(format!("cannot read {path}: {detail}")),
        (StorageFailure::Other, Access::Write) => ZhangError::CustomError(format!("cannot write {path}: {detail}")),
    }
}

impl ZhangError {
    /// whether the error says that there is no file at the path read: [`ZhangError::FileNotFound`], as a data source
    /// answers a read of a missing file, or an io error of kind `NotFound`
    pub fn is_file_not_found(&self) -> bool {
        match self {
            ZhangError::FileNotFound => true,
            ZhangError::IoError(e) | ZhangError::FileError { e, .. } => e.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }
}

pub trait IoErrorIntoZhangError<T> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError>;
}

impl<T> IoErrorIntoZhangError<T> for Result<T, std::io::Error> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError> {
        self.map_err(|e| ZhangError::FileError { e, path: path.to_path_buf() })
    }
}

#[cfg(test)]
mod test {
    use super::{storage_error, Access, StorageFailure, ZhangError};

    /// One mapping for every storage: missing is missing, a refusal names the file and whether it was read or written,
    /// anything else keeps the storage's details
    #[test]
    fn a_storage_error_is_read_alike_whatever_the_storage() {
        let error = |access, failure| storage_error("data/a.zhang", access, failure, "the service failed").to_string();
        assert!(matches!(
            storage_error("a", Access::Read, StorageFailure::NotFound, ""),
            ZhangError::FileNotFound
        ));
        assert!(matches!(
            storage_error("a", Access::Write, StorageFailure::NotFound, ""),
            ZhangError::FileNotFound
        ));
        assert_eq!(error(Access::Read, StorageFailure::Refused), "the storage refused to read data/a.zhang");
        assert_eq!(error(Access::Write, StorageFailure::Refused), "the storage refused to write data/a.zhang");
        assert_eq!(
            error(Access::Read, StorageFailure::Other),
            "custom error: cannot read data/a.zhang: the service failed"
        );
        assert_eq!(
            error(Access::Write, StorageFailure::Other),
            "custom error: cannot write data/a.zhang: the service failed"
        );
        assert_eq!(StorageFailure::from(std::io::ErrorKind::PermissionDenied), StorageFailure::Refused);
        assert_eq!(StorageFailure::from(std::io::ErrorKind::NotFound), StorageFailure::NotFound);
        assert_eq!(StorageFailure::from(std::io::ErrorKind::Other), StorageFailure::Other);
    }
}
