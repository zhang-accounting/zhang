use std::collections::HashMap;

use zhang_ast::error::ErrorKind;
use zhang_ast::SpanInfo;

use crate::ledger::Ledger;
use crate::utils::hashmap::HashMapOfExt;

pub(crate) mod budget;
pub(crate) mod commodity;
pub(crate) mod document;
pub(crate) mod open;
pub(crate) mod plugin;
pub(crate) mod price;
pub(crate) mod transaction;

/// report `commodity_name` at `span` when it is not defined yet
fn check_commodity_define(commodity_name: &str, ledger: &mut Ledger, span: &SpanInfo) {
    // the commodities the options define, and those of the `commodity` directives processed so far
    let defined = ledger.options.commodities().any(|it| it.name == commodity_name) || ledger.defined_commodities.contains(commodity_name);
    if !defined {
        ledger.report(
            ErrorKind::CommodityDoesNotDefine,
            span,
            HashMap::of("commodity_name", commodity_name.to_string()),
        );
    }
}
