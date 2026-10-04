//! Regression tests for #501: the commodity that `operating_currency` defines takes the ledger's
//! final `default_commodity_precision` and `default_rounding`, whatever the order the options are
//! written in. The deprecated `default_balance_tolerance_precision` only stands in for
//! `default_commodity_precision` when that option is not written, and never gives a balance
//! assertion a tolerance.

use std::sync::Arc;

use indoc::indoc;
use zhang_core::ast::error::ErrorKind;
use zhang_core::ast::Rounding;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::domains::schemas::CommodityDomain;
use zhang_core::ledger::Ledger;

/// load `content` as a single-file ledger
fn load(content: &str) -> Ledger {
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("main.zhang"), content).unwrap();
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

fn commodity(ledger: &Ledger, name: &str) -> CommodityDomain {
    ledger
        .operations()
        .commodity(name)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} should be defined"))
}

/// the precision of `CNY` once `options` (in the order given) are read
fn cny_precision(options: &[&str]) -> i32 {
    commodity(&load(&options.join("\n")), "CNY").precision
}

const OPERATING_CURRENCY: &str = r#"option "operating_currency" "CNY""#;
const COMMODITY_PRECISION_4: &str = r#"option "default_commodity_precision" "4""#;
const BALANCE_TOLERANCE_PRECISION_4: &str = r#"option "default_balance_tolerance_precision" "4""#;

#[test]
fn default_commodity_precision_applies_to_the_operating_currency_in_either_order() {
    assert_eq!(cny_precision(&[COMMODITY_PRECISION_4, OPERATING_CURRENCY]), 4);
    assert_eq!(cny_precision(&[OPERATING_CURRENCY, COMMODITY_PRECISION_4]), 4);
}

#[test]
fn default_commodity_precision_applies_to_the_default_operating_currency() {
    // no `operating_currency` option: CNY is still the operating currency, and still a commodity like any other
    assert_eq!(cny_precision(&[COMMODITY_PRECISION_4]), 4);
}

#[test]
fn deprecated_balance_tolerance_precision_stands_in_for_default_commodity_precision_in_either_order() {
    assert_eq!(cny_precision(&[BALANCE_TOLERANCE_PRECISION_4, OPERATING_CURRENCY]), 4);
    assert_eq!(cny_precision(&[OPERATING_CURRENCY, BALANCE_TOLERANCE_PRECISION_4]), 4);
    assert_eq!(cny_precision(&[BALANCE_TOLERANCE_PRECISION_4]), 4);
}

#[test]
fn a_written_default_commodity_precision_wins_over_the_deprecated_option() {
    let commodity_precision_3 = r#"option "default_commodity_precision" "3""#;
    for options in [
        [commodity_precision_3, BALANCE_TOLERANCE_PRECISION_4, OPERATING_CURRENCY],
        [BALANCE_TOLERANCE_PRECISION_4, commodity_precision_3, OPERATING_CURRENCY],
        [OPERATING_CURRENCY, BALANCE_TOLERANCE_PRECISION_4, commodity_precision_3],
    ] {
        assert_eq!(cny_precision(&options), 3, "options: {options:?}");
    }
    // even when it is written with the built-in default value
    let commodity_precision_2 = r#"option "default_commodity_precision" "2""#;
    assert_eq!(cny_precision(&[commodity_precision_2, BALANCE_TOLERANCE_PRECISION_4, OPERATING_CURRENCY]), 2);
    assert_eq!(cny_precision(&[BALANCE_TOLERANCE_PRECISION_4, commodity_precision_2, OPERATING_CURRENCY]), 2);
}

#[test]
fn default_rounding_applies_to_the_operating_currency_in_either_order() {
    let round_up = r#"option "default_rounding" "RoundUp""#;
    assert_eq!(commodity(&load(&[round_up, OPERATING_CURRENCY].join("\n")), "CNY").rounding, Rounding::RoundUp);
    assert_eq!(commodity(&load(&[OPERATING_CURRENCY, round_up].join("\n")), "CNY").rounding, Rounding::RoundUp);
}

#[test]
fn the_operating_currency_keeps_the_built_in_defaults_without_options() {
    let cny = commodity(&load("1970-01-01 open Assets:Bank\n"), "CNY");
    assert_eq!(cny.precision, 2);
    assert_eq!(cny.rounding, Rounding::RoundDown);
}

#[test]
fn a_commodity_directive_still_replaces_the_operating_currency_definition() {
    let ledger = load(indoc! {r#"
        option "operating_currency" "CNY"
        option "default_commodity_precision" "4"
        1970-01-01 commodity CNY
          precision: "6"
          prefix: "¥"
    "#});
    let cny = commodity(&ledger, "CNY");
    assert_eq!(cny.precision, 6);
    assert_eq!(cny.prefix.as_deref(), Some("¥"));
}

#[test]
fn the_deprecated_option_leaves_other_commodities_at_default_commodity_precision() {
    let ledger = load(indoc! {r#"
        option "default_balance_tolerance_precision" "4"
        1970-01-01 commodity USD
    "#});
    assert_eq!(commodity(&ledger, "CNY").precision, 4);
    assert_eq!(commodity(&ledger, "USD").precision, 2);
}

#[test]
fn the_deprecated_option_gives_balance_assertions_no_tolerance() {
    // 0.00001 is below the precision the option sets; the assertion still fails, as assertions are exact unless
    // they write a tolerance with `~`
    let ledger = load(indoc! {r#"
        option "default_balance_tolerance_precision" "4"
        1970-01-01 open Assets:Bank
        1970-01-01 open Equity:Opening
        2024-01-01 * "deposit"
          Assets:Bank 100.00001 CNY
          Equity:Opening -100.00001 CNY
        2024-01-02 balance Assets:Bank 100.00 CNY
    "#});
    let errors = ledger.operations().errors().unwrap();
    assert!(
        errors.iter().any(|it| it.error_type == ErrorKind::AccountBalanceCheckError),
        "the assertion should fail, got {:?}",
        errors.iter().map(|it| it.error_type.clone()).collect::<Vec<_>>()
    );
}
