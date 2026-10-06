use zhang_ast::{Open, SpanInfo};

use crate::ledger::Ledger;
use crate::process;

/// report the commodities `open` restricts its account to that are not defined at its date and time. The account
/// lifecycle reads the `open` from the processed stream: `Ledger::account_status`
pub(crate) fn check(open: &Open, ledger: &mut Ledger, span: &SpanInfo) {
    let at = open.date.naive_datetime();
    for currency in &open.commodities {
        // a `commodity` of the same date and time sorts after the `open`, as beancount orders a day, and defines it
        if ledger.commodity_dates.get(currency).is_some_and(|defined| *defined <= at) {
            continue;
        }
        process::check_commodity_define(currency, ledger, span);
    }
}
