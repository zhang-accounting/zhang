use zhang_ast::{Open, SpanInfo};

use crate::domains::schemas::{AccountStatus, MetaType};
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{process, ZhangResult};

impl DirectiveProcess for Open {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        for currency in &self.commodities {
            process::check_commodity_define(currency, ledger, span)?;
        }
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.insert_or_update_account(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            self.account.clone(),
            AccountStatus::Open,
            self.meta.get_one("alias").map(|it| it.as_str()),
        )?;

        operations.insert_meta(MetaType::AccountMeta, self.account.name(), self.meta.clone())?;

        Ok(())
    }
}
