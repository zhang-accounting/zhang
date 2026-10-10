//! An `open` that lists commodities restricts its account to them, as in beancount (#496): a posting, a balance
//! assertion or a padding in another commodity is reported as `CommodityNotAllowed` with the `account_name` and
//! `commodity` metas, once for each posting as written. Only the units of a posting count. The ledger still loads,
//! the transaction is still booked and the assertion is still checked.

use std::collections::BTreeMap;

use bigdecimal::BigDecimal;
use indoc::indoc;
use zhang_core::ast::error::ErrorKind;
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_testkit::ledger::load_text;

const HEADER: &str = indoc! {r#"
    1970-01-01 commodity USD
    1970-01-01 commodity EUR
    1970-01-01 commodity AAPL
    1970-01-01 open Assets:Bank USD
    1970-01-01 open Assets:Broker AAPL, USD
    1970-01-01 open Assets:Any
    1970-01-01 open Equity:Opening USD
    1970-01-01 open Equity:Free
"#};

/// load `HEADER` followed by `body` as a single-file ledger
fn load(body: &str) -> Ledger {
    load_text(&format!("{HEADER}{body}"))
}

/// reported errors in store order, with the first line of their span and all their metas
fn errors(ledger: &Ledger) -> Vec<(ErrorKind, String, BTreeMap<String, String>)> {
    ledger
        .errors
        .iter()
        .map(|it| {
            let span = it.span.as_ref().and_then(|span| span.content.lines().next()).unwrap_or_default();
            (it.error_type.clone(), span.to_owned(), it.metas.clone().into_iter().collect())
        })
        .collect()
}

/// a `CommodityNotAllowed` error on the directive whose first line is `span`
fn not_allowed(span: &str, account: &str, commodity: &str) -> (ErrorKind, String, BTreeMap<String, String>) {
    let metas = BTreeMap::from([("account_name".to_owned(), account.to_owned()), ("commodity".to_owned(), commodity.to_owned())]);
    (ErrorKind::CommodityNotAllowed, span.to_owned(), metas)
}

/// the units of `commodity` the postings of `account` alone add up to
fn balance(ledger: &Ledger, account: &str, commodity: &str) -> BigDecimal {
    let txns = ledger.transactions();
    let units = txns
        .iter()
        .flat_map(|(_, txn)| &txn.postings)
        .filter(|posting| posting.account.name() == account);
    units
        .filter_map(|posting| posting.units.as_ref())
        .filter(|units| units.commodity == commodity)
        .map(|units| units.number.clone())
        .sum()
}

#[test]
fn a_posting_in_a_commodity_the_open_does_not_list_is_reported_and_still_booked() {
    let ledger = load(indoc! {r#"
        2024-01-10 * "Deposit in the wrong currency"
          Assets:Bank 100 EUR
          Equity:Free -100 EUR
        2024-01-11 * "Deposit in the listed currency"
          Assets:Bank 5 USD
          Equity:Free -5 USD
    "#});

    assert_eq!(
        errors(&ledger),
        vec![not_allowed(r#"2024-01-10 * "Deposit in the wrong currency""#, "Assets:Bank", "EUR")]
    );
    assert_eq!(ledger.transactions().len(), 2);
    assert_eq!(balance(&ledger, "Assets:Bank", "EUR"), BigDecimal::from(100));
    assert_eq!(balance(&ledger, "Assets:Bank", "USD"), BigDecimal::from(5));
}

#[test]
fn only_the_units_of_a_posting_count_not_its_cost_or_price() {
    // as in beancount: the cost and price commodities are not checked against the list
    let ledger = load(indoc! {r#"
        2024-01-01 * "cost in EUR"
          Assets:Broker 2 AAPL {5 EUR}
          Equity:Free -10 EUR
        2024-01-02 * "price in EUR"
          Assets:Bank 11 USD @ 0.9 EUR
          Equity:Free -9.9 EUR
    "#});

    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn every_offending_posting_is_reported_once_as_written() {
    let ledger = load(indoc! {r#"
        2024-01-01 * "two postings"
          Assets:Bank 1 EUR
          Assets:Bank 2 EUR
          Equity:Free
        2024-01-02 * "an elided amount, inferred in EUR"
          Assets:Bank
          Equity:Free -4 EUR
        2024-01-03 * "two lots"
          Assets:Bank 1 AAPL {10 USD}
          Assets:Bank 1 AAPL {11 USD}
          Equity:Free
        2024-01-04 * "a sale across both lots, one posting as written"
          Assets:Bank -2 AAPL {}
          Equity:Free 21 USD
    "#});

    assert_eq!(
        errors(&ledger),
        vec![
            not_allowed(r#"2024-01-01 * "two postings""#, "Assets:Bank", "EUR"),
            not_allowed(r#"2024-01-01 * "two postings""#, "Assets:Bank", "EUR"),
            not_allowed(r#"2024-01-02 * "an elided amount, inferred in EUR""#, "Assets:Bank", "EUR"),
            not_allowed(r#"2024-01-03 * "two lots""#, "Assets:Bank", "AAPL"),
            not_allowed(r#"2024-01-03 * "two lots""#, "Assets:Bank", "AAPL"),
            not_allowed(r#"2024-01-04 * "a sale across both lots, one posting as written""#, "Assets:Bank", "AAPL"),
        ]
    );
    assert_eq!(balance(&ledger, "Assets:Bank", "EUR"), BigDecimal::from(7));
    assert_eq!(balance(&ledger, "Assets:Bank", "AAPL"), BigDecimal::from(0));
}

#[test]
fn an_open_without_commodities_and_a_sub_account_allow_any() {
    let ledger = load(indoc! {r#"
        2024-01-01 open Assets:Bank:Savings
        2024-01-02 * "anything"
          Assets:Any 1 EUR
          Assets:Bank:Savings 1 EUR
          Equity:Free -2 EUR
    "#});

    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn a_balance_assertion_in_a_commodity_the_open_does_not_list_is_reported_and_still_checked() {
    let ledger = load(indoc! {r#"
        2024-01-01 balance Assets:Bank 0 EUR
        2024-01-01 balance Assets:Bank 0 USD
    "#});

    assert_eq!(errors(&ledger), vec![not_allowed("2024-01-01 balance Assets:Bank 0 EUR", "Assets:Bank", "EUR")]);
    let checks = ledger.outcomes.iter().filter_map(|it| match it.detail {
        Detail::Assertion { passed, .. } => Some(passed),
        _ => None,
    });
    let checks = checks.collect::<Vec<_>>();
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().all(|passed| *passed));
}

#[test]
fn a_pad_that_books_a_commodity_the_open_does_not_list_is_reported_on_the_pad() {
    // the padding transaction of a `pad` is reported on the `pad`, for each of its accounts, as beancount
    // reports it; the assertion it serves is reported on its own
    let ledger = load(indoc! {r#"
        2024-01-01 pad Assets:Any Equity:Opening
        2024-01-02 balance Assets:Any 100 EUR
        2024-01-03 pad Assets:Bank Equity:Free
        2024-01-04 balance Assets:Bank 50 EUR
    "#});

    assert_eq!(
        errors(&ledger),
        vec![
            not_allowed("2024-01-01 pad Assets:Any Equity:Opening", "Equity:Opening", "EUR"),
            not_allowed("2024-01-03 pad Assets:Bank Equity:Free", "Assets:Bank", "EUR"),
            not_allowed("2024-01-04 balance Assets:Bank 50 EUR", "Assets:Bank", "EUR"),
        ]
    );
    // the paddings are booked
    assert_eq!(balance(&ledger, "Assets:Any", "EUR"), BigDecimal::from(100));
    assert_eq!(balance(&ledger, "Equity:Opening", "EUR"), BigDecimal::from(-100));
    assert_eq!(balance(&ledger, "Assets:Bank", "EUR"), BigDecimal::from(50));
}

#[test]
fn a_balance_with_pad_reports_each_of_its_accounts_once() {
    // its padding books the asserted commodity on its own account: the assertion reports that account
    let ledger = load(indoc! {r#"
        2024-01-01 balance Assets:Bank 100 EUR with pad Equity:Opening
        2024-01-02 balance Assets:Any 100 EUR with pad Equity:Free
    "#});

    assert_eq!(
        errors(&ledger),
        vec![
            not_allowed("2024-01-01 balance Assets:Bank 100 EUR with pad Equity:Opening", "Assets:Bank", "EUR"),
            not_allowed("2024-01-01 balance Assets:Bank 100 EUR with pad Equity:Opening", "Equity:Opening", "EUR"),
        ]
    );
    assert_eq!(balance(&ledger, "Assets:Bank", "EUR"), BigDecimal::from(100));
    assert_eq!(balance(&ledger, "Equity:Opening", "EUR"), BigDecimal::from(-100));
}
