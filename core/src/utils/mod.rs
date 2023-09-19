use std::path::Path;

use glob::Pattern;

pub mod bigdecimal_ext;
pub mod calculable;
pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod logging;
pub mod price_grip;
pub mod string_;

pub fn has_path_visited<'a>(visited: impl IntoIterator<Item = &'a Pattern>, path: &Path) -> bool {
    visited.into_iter().any(|pattern| pattern.matches_path(path))
}
