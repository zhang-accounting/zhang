use zhang_ast::*;

use crate::ledger::Ledger;
use crate::ZhangResult;

pub trait Exporter: AppendableExporter {
    type Output;
    fn export_directive(&self, directive: Directive) -> Self::Output;
}

pub trait AppendableExporter: Send + Sync {
    /// define how the exporter append directives
    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;
}

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}
