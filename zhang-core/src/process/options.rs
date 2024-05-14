use zhang_ast::{Options, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::ZhangResult;

impl DirectiveProcess for Options {
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        let option_value = ledger.options.parse(self.key.as_str(), self.value.as_str(), &mut operations, span)?;
        operations.insert_or_update_options(self.key.as_str(), option_value.as_str())?;
        Ok(())
    }
}
