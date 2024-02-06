use crate::ZhangResult;
use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};

pub mod text;

/// `DataType` is the protocol to describe how the raw data be transformed into standard directives and vice versa.
/// `Carrier` is the type of raw data, it can be plain text, bytes, or even sql.
pub trait DataType {
    type Carrier;

    fn transform<Source: Into<Option<String>>>(&self, raw_data: Self::Carrier, source: Source) -> ZhangResult<Vec<Spanned<Directive>>>;

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier;
}
