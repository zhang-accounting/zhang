use std::path::PathBuf;

use zhang_ast::{Directive, Spanned};

pub struct LoadResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}
