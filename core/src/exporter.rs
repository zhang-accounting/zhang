use std::path::PathBuf;
use log::debug;
use zhang_ast::Directive;
use crate::ZhangResult;

pub trait Exporter {
    type Output;
    fn to_target(self) -> Self::Output;

    /// define how the exporter append a directive to target file
    fn append_directive(file: PathBuf, directive:Directive) -> ZhangResult<()>;
}

pub struct DebugExporter;

impl Exporter for DebugExporter {
    type Output = ();

    fn to_target(self) -> Self::Output {
        ()
    }

    fn append_directive(file: PathBuf, directive: Directive) -> ZhangResult<()> {
        debug!("append directive [{:?}] to path [{:?}]", directive, file);
        Ok(())
    }
}