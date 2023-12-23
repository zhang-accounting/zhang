use std::path::PathBuf;

pub mod bigdecimal_ext;
pub mod calculable;
pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod logging;
pub mod price_grip;
pub mod string_;

pub fn has_path_visited<'a>(visited: impl IntoIterator<Item = &'a PathBuf>, path: &PathBuf) -> bool {
    visited.into_iter().any(|pathbuf| pathbuf.eq(path))
}
