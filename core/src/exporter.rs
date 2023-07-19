use crate::ledger::Ledger;
use crate::ZhangResult;
use log::debug;
use zhang_ast::Directive;

pub trait Exporter: AppendableExporter {
    type Output;
    fn export_directive(&self, directive: Directive) -> Self::Output;
}

pub trait AppendableExporter: Send + Sync {
    /// define how the exporter append directives
    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;
}

pub struct DebugExporter;

impl AppendableExporter for DebugExporter {
    fn append_directives(&self, _: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        debug!("append directive [{:?}]", directives);
        Ok(())
    }
}

impl Exporter for DebugExporter {
    type Output = ();

    fn export_directive(&self, directive: Directive) -> Self::Output {
        debug!("export directive: {:?}", directive);
    }
}
