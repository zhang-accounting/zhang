use zhang_ast::{Directive, Spanned};

use crate::ZhangResult;

pub mod text;

/// `DataType` is the protocol to describe how the raw data be transformed into standard directives and vice versa.
/// `Carrier` is the type of raw data, it can be plain text, bytes, or even sql.
pub trait DataType
where
    Self: Send + Sync,
{
    type Carrier;

    fn transform(&self, raw_data: Self::Carrier, source: Option<String>) -> ZhangResult<Vec<Spanned<Directive>>>;

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier;
}
