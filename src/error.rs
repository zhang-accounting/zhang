use thiserror::Error;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub enum AvaroError {
    #[error("date is invalid")]
    InvalidDate,
    #[error("account is invalid")]
    InvalidAccount,
}
