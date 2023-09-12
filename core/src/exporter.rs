use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use chrono::Datelike;
use itertools::Itertools;
use log::debug;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::ledger::Ledger;
use crate::utils::has_path_visited;
use crate::utils::string_::escape_with_quote;
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

