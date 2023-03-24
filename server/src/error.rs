use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("core error: {0}")]
    CoreError(#[from] zhang_core::error::ZhangError),

    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),

    #[error("database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}
