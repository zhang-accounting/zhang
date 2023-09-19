use std::path::PathBuf;

use zhang_ast::{Directive, Spanned};

use crate::text::parser::parse;
use crate::transform::TextFileBasedTransformer;
use crate::{ZhangError, ZhangResult};

#[derive(Clone, Default)]
pub struct TextTransformer {}

impl TextFileBasedTransformer for TextTransformer {
    type FileOutput = Spanned<Directive>;

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>> {
        parse(content, path).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn go_next(&self, directive: &Self::FileOutput) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>> {
        Ok(directives)
    }
}
