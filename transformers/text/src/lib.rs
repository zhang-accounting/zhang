use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};
use zhang_core::transform::TextFileBasedTransformer;
use zhang_core::{ZhangError, ZhangResult};

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod p;

pub use p::parse_zhang;

#[derive(Clone, Default)]
pub struct TextTransformer {}

impl TextFileBasedTransformer for TextTransformer {
    type FileOutput = Spanned<Directive>;

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>> {
        parse_zhang(content, path).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn go_next(&self, directive: &Self::FileOutput) -> Option<PathBuf> {
        match &directive.data {
            Directive::Include(include) => {
                let buf = PathBuf::from(include.file.clone().to_plain_string());
                Some(buf)
            }
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>> {
        Ok(directives)
    }
}
