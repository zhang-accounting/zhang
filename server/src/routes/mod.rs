

use std::path::PathBuf;






use zhang_core::error::IoErrorIntoZhangError;




use crate::ServerResult;

pub mod account;
pub mod commodity;
pub mod common;
pub mod document;
pub mod file;
pub mod statistics;
pub mod transaction;

#[cfg(feature = "frontend")]
pub mod frontend;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

pub(crate) fn insert_line(file: PathBuf, content: &str, at: usize) -> ServerResult<()> {
    let mut file_content = std::fs::read_to_string(&file).with_path(&file)?;
    file_content.insert(at, '\n');
    file_content.insert_str(at + 1, content);
    Ok(std::fs::write(&file, file_content).with_path(&file)?)
}
