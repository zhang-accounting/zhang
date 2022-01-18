macro_rules! parse {
    ($content: expr) => {{
        use itertools::Itertools;
        let x = $content.lines().nth(1).unwrap();
        let space_offset = x.len() - x.trim_start().len();
        let string1 = $content
            .lines()
            .map(|it| it.replacen(" ", "", space_offset))
            .join("\n");
        crate::p::parse_avaro(string1.trim()).unwrap()
    }};
}

pub mod booking;
pub mod error;
pub mod models;
pub mod to_file;

pub(crate) mod utils;

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod p;

pub mod cli;
pub mod core;
pub mod importer;
pub mod exporter;

pub mod target;
use crate::models::Directive;
pub use p::parse_avaro;
