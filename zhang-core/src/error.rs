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

    #[error("invalid content encoding: {0}")]
    ContentEncodingError(#[from] std::string::FromUtf8Error),

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

pub trait IoErrorIntoZhangError<T> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError>;
}

impl<T> IoErrorIntoZhangError<T> for Result<T, std::io::Error> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError> {
        self.map_err(|e| ZhangError::FileError { e, path: path.to_path_buf() })
    }
}
