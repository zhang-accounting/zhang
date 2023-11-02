use std::path::{Path, PathBuf};

use glob::Pattern;
use itertools::Either;

pub mod bigdecimal_ext;
pub mod calculable;
pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod logging;
pub mod price_grip;
pub mod string_;

pub fn has_path_visited<'a>(visited: impl IntoIterator<Item = &'a Either<Pattern, PathBuf>>, path: &Path) -> bool {
    visited.into_iter().any(|pattern| match pattern {
        Either::Left(pattern) => pattern.matches_path(path),
        Either::Right(pathbuf) => pathbuf.eq(path),
    })
}

const GLOB_CHAR: char = '*';
pub(crate) fn to_glob_or_path(path: PathBuf) -> Either<Pattern, PathBuf> {
    let path_str = path.as_path().to_str().unwrap();

    if path_str.contains(GLOB_CHAR) {
        Either::Left(Pattern::new(path.as_path().to_str().unwrap()).unwrap())
    } else {
        Either::Right(path)
    }
}
