use zhang_ast::*;

pub trait Exporter {
    type Output;
    fn export_directive(&self, directive: Directive) -> Self::Output;
}
