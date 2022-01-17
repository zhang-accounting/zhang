use thiserror::Error;

pub type AvaroResult<T> = Result<T, AvaroError>;


#[derive(Debug, Error)]
pub enum AvaroError {
    #[error("date is invalid")]
    InvalidDate,
    #[error("account is invalid")]
    InvalidAccount,

    #[error("file error: {0}")]
    FileError(#[from] std::io::Error),

    #[error("csv error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("toml error: {0}")]
    TomlError(#[from] toml::de::Error),

    #[error("strum error: {0}")]
    StrumError(#[from] strum::ParseError),
}
