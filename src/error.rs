
use thiserror::Error;

pub type ZhangResult<T> = Result<T, ZhangError>;

#[derive(Debug, Error)]
pub enum ZhangError {
    #[error("date is invalid")]
    InvalidDate,
    #[error("account is invalid")]
    InvalidAccount,

    #[error("file error: {0}")]
    FileError(#[from] std::io::Error),

    #[error("csv error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("toml error: {0}")]
    TomlDeError(#[from] toml::de::Error),

    #[error("toml ser error: {0}")]
    TomlSerError(#[from] toml::ser::Error),

    #[error("strum error: {0}")]
    StrumError(#[from] strum::ParseError),

    #[error("pest error: {0}")]
    PestError(String),
}
