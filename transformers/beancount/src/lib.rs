use itertools::{Either, Itertools};
use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};
use zhang_core::transform::TextFileBasedTransformer;
use zhang_core::{ZhangError, ZhangResult};

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod parser;

pub mod directives;

use crate::directives::BeancountDirective;
pub use parser::parse;

#[derive(Clone, Default)]
pub struct BeancountTransformer {}

impl TextFileBasedTransformer for BeancountTransformer {
    type FileOutput = Spanned<Either<Directive, BeancountDirective>>;

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>> {
        parse(content, path).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn go_next(&self, directive: &Self::FileOutput) -> Option<String> {
        match &directive.data {
            Either::Left(Directive::Include(include)) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut ret = vec![];
        let mut tags_stack:Vec<String> = vec![];
        for directives in directives {
            let Spanned { span, data } = directives;
            match data {
                Either::Left(zhang_directive) => match zhang_directive {
                    Directive::Transaction(mut trx) => {
                        for tag in &tags_stack {
                            trx.tags.insert(tag.to_owned());
                        }
                        ret.push(Spanned {
                            span,
                            data: Directive::Transaction(trx),
                        });
                    }
                    _ => ret.push(Spanned {
                        span,
                        data: zhang_directive,
                    }),
                },
                Either::Right(beancount_directive) => match beancount_directive {
                    BeancountDirective::PushTag(tag) => tags_stack.push(tag),
                    BeancountDirective::PopTag(tag) => {
                        tags_stack = tags_stack.into_iter().filter(|it| it.ne(&tag)).collect_vec()
                    }
                    BeancountDirective::Pad(_) => {}
                    BeancountDirective::Balance(_) => {}
                },
            }
        }
        Ok(ret)
    }
}
