pub mod error;
pub mod models;
pub mod to_file;
pub mod account;
pub mod amount;
pub mod data;

pub(crate) mod utils;

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod p;

pub use p::parse_avaro;
