use zhang_ast::{Price, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Price {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_commodity_define(&self.currency, ledger, span)?;
        process::check_commodity_define(&self.amount.commodity, ledger, span)?;
        Ok(true)
    }

    /// the budget check and the query engine read the price from the processed stream
    /// ([`price_map`](crate::domains::schemas::price_map))
    fn process(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        Ok(())
    }
}
