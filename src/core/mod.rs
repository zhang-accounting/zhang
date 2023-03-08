pub mod account;
pub mod amount;
pub mod data;
pub mod database;
pub mod ledger;
pub mod models;
pub mod operations;
pub mod options;
pub(crate) mod process;
pub mod utils;

pub type Currency = String;
pub type AccountName = String;

pub const DEFAULT_COMMODITY_PRECISION: i32 = 2;
