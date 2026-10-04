use std::collections::HashMap;

use zhang_ast::error::ErrorKind;
use zhang_ast::{Close, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Close {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();

        // check if account exist
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;

        // Booking already holds the true units at this point in the stream. A close checks the
        // account itself, not its subtree; an assertion does not change these units.
        if ledger.booker_mut().has_non_zero_balance(self.account.name()) {
            operations.new_error(ErrorKind::CloseNonZeroAccount, span, HashMap::default())?;
        }
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.close_account(self.account.name())?;
        Ok(())
    }
}
