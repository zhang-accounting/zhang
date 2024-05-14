use zhang_ast::{Price, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Price {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_commodity_define(&self.currency, ledger, span)?;
        process::check_commodity_define(&self.amount.currency, ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.insert_price(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.currency,
            &self.amount.number,
            &self.amount.currency,
        )?;

        Ok(())
    }
}
