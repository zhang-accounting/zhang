use std::path::PathBuf;

pub mod constants;
pub mod database;
pub mod domains;
pub mod error;
pub mod exporter;
pub mod ledger;
pub mod options;
pub(crate) mod process;
pub mod transform;
pub mod utils;

pub type ZhangResult<T> = Result<T, ZhangError>;
pub use error::ZhangError;
