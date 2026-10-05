use std::net::AddrParseError;
use std::path::{Path, PathBuf};

use thiserror::Error;
use zhang_ast::SpanInfo;

#[derive(Debug, Error)]
pub enum ZhangError {
    #[error("date is invalid")]
    InvalidDate,
    #[error("account is invalid")]
    InvalidAccount,

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
