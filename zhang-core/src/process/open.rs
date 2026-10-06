use zhang_ast::{Open, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Open {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let at = self.date.naive_datetime();
        for currency in &self.commodities {
            // a `commodity` of the same date and time sorts after the `open`, as beancount orders a day, and defines it
            if ledger.commodity_dates.get(currency).is_some_and(|defined| *defined <= at) {
                continue;
            }
            process::check_commodity_define(currency, ledger, span)?;
        }
        Ok(true)
    }

    /// the account lifecycle reads the `open` from the processed stream: `Ledger::account_status`
    fn process(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        Ok(())
    }
}
