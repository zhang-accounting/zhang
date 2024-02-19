use crate::data_type::text::exporter::ZhangDataTypeExportable;
use crate::data_type::text::parser::parse;
use crate::data_type::DataType;
use crate::{ZhangError, ZhangResult};
use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod parser;

pub mod exporter;

#[derive(Default)]
pub struct ZhangDataType {}

impl DataType for ZhangDataType {
    type Carrier = String;

    fn transform(&self, raw_data: Self::Carrier, source: Option<String>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let file = source.map(PathBuf::from);
        parse(&raw_data, file).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier {
        directive.data.export()
    }
}
