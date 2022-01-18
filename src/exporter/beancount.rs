use std::path::PathBuf;
use crate::error::{AvaroError, AvaroResult};
use crate::core;
pub fn run(file: PathBuf, output: Option<PathBuf> ) -> AvaroResult<()> {
    let avaro_content = std::fs::read_to_string(file)?;
    let vec = core::load(&avaro_content)?;
    dbg!(vec);
    Ok(())
}
