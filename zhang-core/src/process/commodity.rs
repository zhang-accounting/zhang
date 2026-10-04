use std::str::FromStr;

use zhang_ast::{Commodity, Rounding, SpanInfo};

use crate::constants::{DEFAULT_COMMODITY_PRECISION, DEFAULT_ROUNDING};
use crate::domains::schemas::MetaType;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{ZhangError, ZhangResult};

impl DirectiveProcess for Commodity {
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        // the options handler resolved both defaults before any dated directive
        let default_precision = Some(ledger.options.default_commodity_precision);
        let default_rounding = Some(ledger.options.default_rounding);
        let (precision, rounding) = commodity_precision(self, default_precision, default_rounding)?;

        let mut operations = ledger.operations();
        let prefix = self.meta.get_one("prefix").map(|it| it.clone().to_plain_string());
        let suffix = self.meta.get_one("suffix").map(|it| it.clone().to_plain_string());

        operations.insert_commodity(&self.currency, precision, prefix, suffix, rounding)?;
        operations.insert_meta(MetaType::CommodityMeta, &self.currency, self.meta.clone())?;
        // booking rounds the implicit posting of a transaction at this precision
        ledger.booker_mut().define_commodity(&self.currency, precision, rounding);

        Ok(())
    }
}

/// a commodity's precision and rounding: its `precision` / `rounding` meta, else the ledger's
/// `default_commodity_precision` / `default_rounding` option, else the built-in default. An
/// invalid `precision` meta is ignored; an invalid `rounding` meta is an error
pub(crate) fn commodity_precision(commodity: &Commodity, default_precision: Option<i32>, default_rounding: Option<Rounding>) -> ZhangResult<(i32, Rounding)> {
    let precision = commodity
        .meta
        .get_one("precision")
        .map(|it| it.as_str().to_owned())
        .map(|it| it.as_str().parse::<i32>())
        .transpose()
        .unwrap_or(None)
        .or(default_precision)
        .unwrap_or(DEFAULT_COMMODITY_PRECISION);
    let rounding = commodity
        .meta
        .get_one("rounding")
        .map(|it| it.as_str().to_owned())
        .map(|it| Rounding::from_str(it.as_str()))
        .transpose()
        .map_err(|_| ZhangError::InvalidOptionValue)?
        .or(default_rounding)
        .unwrap_or(DEFAULT_ROUNDING);
    Ok((precision, rounding))
}
