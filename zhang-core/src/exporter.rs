use zhang_ast::*;

use crate::ledger::Ledger;
use crate::ZhangResult;

pub trait Exporter {
    type Output;
    fn export_directive(&self, directive: Directive) -> Self::Output;
}
