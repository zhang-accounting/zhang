use std::path::{Path, PathBuf};

use thiserror::Error;
use zhang_ast::account::InvalidAccountError;

#[derive(Debug, Error)]
pub enum ZhangError {
    #[error("date is invalid")]
    InvalidDate,
    #[error("account is invalid")]
    InvalidAccount,
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("file error: {e}")]
    FileError { e: std::io::Error, path: PathBuf },
    //
    // #[error("csv error: {0}")]
    // CsvError(#[from] csv::Error),
    //
    // #[error("toml error: {0}")]
    // TomlDeError(#[from] toml::de::Error),
    //
    // #[error("toml ser error: {0}")]
    // TomlSerError(#[from] toml::ser::Error),
    //
    // #[error("strum error: {0}")]
    // StrumError(#[from] strum::ParseError),
    #[error("pest error: {0}")]
    PestError(String),

    #[error("databaseError: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("cannot found option given key: {0}")]
    OptionNotFound(String),
}

pub trait IoErrorIntoZhangError<T> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError>;
}

impl<T> IoErrorIntoZhangError<T> for Result<T, std::io::Error> {
    fn with_path(self, path: &Path) -> Result<T, ZhangError> {
        self.map_err(|e| ZhangError::FileError { e, path: path.to_path_buf() })
    }
}
