use std::net::AddrParseError;
use std::path::{Path, PathBuf};

use thiserror::Error;

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

    #[error("Parse Error \nPath: {path}{msg}")]
    PestError { path: String, msg: String },
    #[error("Process Error: {0}")]
    ProcessError(zhang_ast::error::ErrorKind),

    #[error("cannot found option given key: {0}")]
    OptionNotFound(String),

    #[error("invalid content encoding: {0}")]
    ContentEncodingError(#[from] std::string::FromUtf8Error),

    #[error("custom error: {0}")]
    CustomError(&'static str),
}

pub trait IoErrorIntoZhangError<T> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError>;
}

impl<T> IoErrorIntoZhangError<T> for Result<T, std::io::Error> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError> {
        self.map_err(|e| ZhangError::FileError { e, path: path.to_path_buf() })
    }
}
