use crate::core::models::Directive;
use crate::core::utils::span::Spanned;
use crate::error::ZhangResult;
use std::path::PathBuf;

pub mod account;
pub mod amount;
pub mod data;
pub mod database;
pub mod domains;
pub mod ledger;
pub mod models;
pub mod options;
pub(crate) mod process;
pub mod utils;

pub type Currency = String;
pub type AccountName = String;

pub const DEFAULT_COMMODITY_PRECISION: i32 = 2;

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}

pub trait Transformer
where
    Self: Send + Sync,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult>;
}
