use zhang_ast::{Close, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Close {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        // check if account exist
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;

        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.close_account(self.account.name())?;
        Ok(())
    }
}
