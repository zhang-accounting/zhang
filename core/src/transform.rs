use crate::ZhangResult;
use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}

pub trait Transformer
where
    Self: Send + Sync,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult>;
}
