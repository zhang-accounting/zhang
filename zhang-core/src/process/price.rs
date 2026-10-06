use zhang_ast::{Price, SpanInfo};

use crate::ledger::Ledger;
use crate::process;

/// report the commodities of `price` that are not defined at its date. The budget check and the query engine read the
/// price from the processed stream ([`price_map`](crate::domains::schemas::price_map))
pub(crate) fn check(price: &Price, ledger: &mut Ledger, span: &SpanInfo) {
    process::check_commodity_define(&price.currency, ledger, span);
    process::check_commodity_define(&price.amount.commodity, ledger, span);
}
