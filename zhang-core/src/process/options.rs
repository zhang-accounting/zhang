use zhang_ast::{Options, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::ZhangResult;

impl DirectiveProcess for Options {
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        ledger.options.parse(self.key.as_str(), self.value.as_str(), &mut operations, span)
    }
}
