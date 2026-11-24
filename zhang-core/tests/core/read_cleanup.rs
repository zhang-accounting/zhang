//! The load validations retained when #479 removes the old read-side caches.
use zhang_ast::error::ErrorKind;
use zhang_core::ledger::Ledger;
use zhang_testkit::ledger::load_text;

fn errors(ledger: &Ledger) -> Vec<ErrorKind> {
    ledger.errors.iter().map(|error| error.error_type.clone()).collect()
}

#[test]
fn close_sums_lots_in_the_same_commodity_but_keeps_currencies_separate() {
    let ledger = load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity USD
1970-01-01 open Assets:Lots
1970-01-01 open Assets:FX
1970-01-01 open Equity:Opening
2024-01-01 * "different costs and labels, zero USD units"
  Assets:Lots 1 USD {10 CNY, "long"}
  Assets:Lots -1 USD {20 CNY, "short"}
  Equity:Opening 10 CNY
2024-01-02 close Assets:Lots
2024-01-03 * "numbers cancel, currencies do not"
  Assets:FX 1 USD @ 1 CNY
  Assets:FX -1 CNY
2024-01-04 close Assets:FX
"#,
    );
    // Selling a different cost reports a missing lot, but its booked negative lot still
    // cancels the long lot in USD units, so only the FX account fails the close check.
    assert_eq!(errors(&ledger), vec![ErrorKind::NoEnoughCommodityLot, ErrorKind::CloseNonZeroAccount]);
    assert_eq!(ledger.errors[1].span.as_ref().unwrap().content.trim(), "2024-01-04 close Assets:FX");
}

#[test]
fn close_checks_own_units_at_its_place_in_the_stream() {
    let ledger = load_text(
        r#"
1970-01-01 open Assets:Parent
1970-01-01 open Assets:Parent:Child
1970-01-01 open Equity:Opening
2024-01-01 * "only the child holds units"
  Assets:Parent:Child 1 CNY
  Equity:Opening -1 CNY
2024-01-02 close Assets:Parent
2024-01-03 * "the later posting cannot affect the close"
  Assets:Parent 1 CNY
  Equity:Opening -1 CNY
"#,
    );
    assert_eq!(errors(&ledger), vec![ErrorKind::AccountClosed]);
}

#[test]
fn close_uses_stream_order_on_a_daylight_saving_gap_day() {
    let ledger = load_text(
        r#"
option "timezone" "America/New_York"
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Opening
2024-03-10 02:30:00 * "a skipped local time is stored at 03:30"
  Assets:Cash 7 CNY
  Equity:Opening -7 CNY
2024-03-10 03:15:00 * "later in ledger order, earlier as an instant"
  Assets:Cash -7 CNY
  Equity:Opening 7 CNY
2024-03-11 close Assets:Cash
"#,
    );
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn a_failed_assertion_does_not_zero_the_units_a_close_checks() {
    let ledger = load_text(
        r#"
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Opening
2024-01-01 balance Assets:Cash 10 CNY with pad Equity:Opening
2024-01-02 balance Assets:Cash 0 CNY
2024-01-03 close Assets:Cash
"#,
    );
    assert_eq!(errors(&ledger), vec![ErrorKind::AccountBalanceCheckError, ErrorKind::CloseNonZeroAccount]);
}
