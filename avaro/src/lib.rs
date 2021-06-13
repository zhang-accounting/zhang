use lalrpop_util::lalrpop_mod;
pub mod error;
pub mod models;
pub mod to_file;

pub(crate) mod utils;
lalrpop_mod!(#[allow(clippy::all)] pub parser);

pub mod p;

pub use parser::EntryParser;