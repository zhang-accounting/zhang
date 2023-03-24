use thiserror::Error;
use zhang_ast::account::InvalidAccountError;
use zhang_core::ZhangError;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("core error: {0}")]
    CoreError(#[from] ZhangError),

    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),

    #[error("database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}


impl From<InvalidAccountError> for ServerError {
    fn from(value: InvalidAccountError) -> Self {
        Self::CoreError(ZhangError::InvalidAccount)
    }
}