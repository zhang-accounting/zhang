use crate::{Directive, parse_avaro};

pub mod account;
pub mod amount;
pub mod data;
pub mod inventory;

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
