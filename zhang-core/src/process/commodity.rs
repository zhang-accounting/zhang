use std::str::FromStr;

use zhang_ast::{Commodity, Rounding, SpanInfo};

use crate::constants::{DEFAULT_COMMODITY_PRECISION, DEFAULT_ROUNDING, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING};
use crate::domains::schemas::MetaType;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{ZhangError, ZhangResult};

impl DirectiveProcess for Commodity {
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let ledger_default_precision = operations.option::<i32>(KEY_DEFAULT_COMMODITY_PRECISION)?;
        let ledger_default_rounding = operations.option::<Rounding>(KEY_DEFAULT_ROUNDING)?;

        let precision = self
            .meta
            .get_one("precision")
            .map(|it| it.as_str().to_owned())
            .map(|it| it.as_str().parse::<i32>())
            .transpose()
            .unwrap_or(None)
            .or(ledger_default_precision)
            .unwrap_or(DEFAULT_COMMODITY_PRECISION);
        let prefix = self.meta.get_one("prefix").map(|it| it.clone().to_plain_string());
        let suffix = self.meta.get_one("suffix").map(|it| it.clone().to_plain_string());
        let rounding = self
            .meta
            .get_one("rounding")
            .map(|it| it.as_str().to_owned())
            .map(|it| Rounding::from_str(it.as_str()))
            .transpose()
            .map_err(|_| ZhangError::InvalidOptionValue)?
            .or(ledger_default_rounding)
            .unwrap_or(DEFAULT_ROUNDING);

        operations.insert_commodity(&self.currency, precision, prefix, suffix, rounding)?;
        operations.insert_meta(MetaType::CommodityMeta, &self.currency, self.meta.clone())?;

        Ok(())
    }
}
