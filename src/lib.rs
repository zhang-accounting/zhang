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

pub mod account;
pub mod amount;
pub mod booking;
pub mod data;
pub mod error;
pub mod inventory;
pub mod models;
pub mod to_file;

pub(crate) mod utils;

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod p;

pub mod cli;
pub mod importer;
pub mod target;
use crate::models::Directive;
pub use p::parse_avaro;

pub fn load(content: &str) -> Result<Vec<Directive>, crate::error::AvaroError> {
    let mut entities = parse_avaro(content).unwrap();
    // todo: sort entities

    // todo: book
    // booking::book(&mut entities)?;

    // todo: run_transformations

    // todo: validate

    //
    dbg!(&entities);
    Ok(entities)
}
