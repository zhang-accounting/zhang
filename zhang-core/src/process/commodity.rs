use std::str::FromStr;

use zhang_ast::{Commodity, Rounding, SpanInfo};

use crate::constants::{DEFAULT_COMMODITY_PRECISION, DEFAULT_ROUNDING};
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::{ZhangError, ZhangResult};

impl DirectiveProcess for Commodity {
    /// an invalid `rounding` meta stops the load ([`Ledger::commodities`] reads the rest of the directive); the
    /// directives after it may use the commodity
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        // the options handler resolved both defaults before any dated directive
        commodity_precision(self, Some(ledger.options.default_commodity_precision), Some(ledger.options.default_rounding))?;
        ledger.defined_commodities.insert(self.currency.clone());
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
