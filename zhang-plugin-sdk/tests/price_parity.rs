//! The SDK's [`PriceMap`] against zhang's own: the rates `zhang-query` uses for `convert`, `value` and
//! `getprice`. Each ledger is loaded by zhang-core once; the SDK builds its map from the processed stream, as a
//! plugin would, and zhang-query from the store's prices. Every pair of the ledger's commodities must have the
//! same rate, to the digit, on every day around its prices.
#![cfg(not(target_arch = "wasm32"))]

use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::{Days, NaiveDate};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_plugin_sdk::ast::amount::Amount;
use zhang_plugin_sdk::prices::PriceMap;

fn day(text: &str) -> NaiveDate {
    NaiveDate::from_str(text).unwrap()
}

fn number(text: &str) -> BigDecimal {
    BigDecimal::from_str(text).unwrap()
}

fn load(content: &str) -> Ledger {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

/// both maps of `content`, after checking they agree on every pair of `commodities` from `from` to `to`
fn parity(content: &str, commodities: &[&str], from: &str, to: &str) -> PriceMap {
    let ledger = load(content);
    let sdk = PriceMap::from_stream(&ledger.directives);
    let zhang = zhang_query::PriceMap::from_prices(&ledger.store.read().unwrap().prices);
    let mut date = day(from);
    let mut compared = 0;
    while date <= day(to) {
        for base in commodities {
            for quote in commodities {
                let expected = zhang.rate(base, quote, Some(date));
                let actual = sdk.rate(base, quote, date);
                assert_eq!(
                    actual.as_ref().map(ToString::to_string),
                    expected.as_ref().map(ToString::to_string),
                    "rate {base} -> {quote} on {date}"
                );
                // and a conversion as zhang's `convert()` computes it
                let amount = Amount::new(number("1234.56"), *base);
                let converted = expected.map(|rate| zhang_query::decimal::mul_in_context(&amount.number, &rate).to_string());
                assert_eq!(
                    sdk.convert(&amount, quote, date).map(|it| it.number.to_string()),
                    converted,
                    "convert {base} -> {quote} on {date}"
                );
                compared += 1;
            }
        }
        date = date + Days::new(1);
    }
    assert!(compared > 0);
    sdk
}

#[test]
fn latest_price_on_or_before_the_date_and_nothing_before_the_first() {
    let prices = parity(
        r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity EUR
2024-01-10 price USD 7.10 CNY
2024-02-01 price USD 7.20 CNY
2024-03-15 price USD 7.05 CNY
"#,
        &["CNY", "USD", "EUR"],
        "2024-01-01",
        "2024-04-01",
    );
    assert_eq!(prices.rate("USD", "CNY", day("2024-01-09")), None, "before any price");
    assert_eq!(prices.rate("USD", "CNY", day("2024-01-10")), Some(number("7.10")));
    assert_eq!(prices.rate("USD", "CNY", day("2024-03-14")), Some(number("7.20")));
    assert_eq!(prices.rate("USD", "CNY", day("2024-04-01")), Some(number("7.05")));
    assert_eq!(prices.rate("EUR", "CNY", day("2024-04-01")), None, "no price at all");
    assert_eq!(prices.rate("EUR", "EUR", day("2024-04-01")), Some(number("1")));
}

#[test]
fn inverse_rates_exact_and_rounded() {
    let prices = parity(
        r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity JPY
1970-01-01 commodity EUR
2024-01-01 price USD 8 CNY
2024-01-01 price JPY 0.050 CNY
2024-01-01 price EUR 7.77 CNY
2024-02-01 price USD 7 CNY
"#,
        &["CNY", "USD", "JPY", "EUR"],
        "2023-12-31",
        "2024-02-02",
    );
    assert_eq!(prices.rate("CNY", "USD", day("2024-01-15")).unwrap().to_string(), "0.125");
    // `1 / 0.050` keeps Python's ideal exponent, as beancount and zhang do: 2E+1, which is 20
    assert_eq!(prices.rate("CNY", "JPY", day("2024-01-15")), Some(number("20")));
    assert_eq!(
        prices.rate("CNY", "EUR", day("2024-01-15")).unwrap().to_string(),
        "0.1287001287001287001287001287"
    );
    assert_eq!(
        prices.rate("CNY", "USD", day("2024-02-01")).unwrap().to_string(),
        "0.1428571428571428571428571429"
    );
}

#[test]
fn the_last_price_of_a_day_wins() {
    let prices = parity(
        r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
2024-02-01 price USD 7.1 CNY
2024-02-01 price USD 7.2 CNY
2024-03-01 09:00:00 price USD 7.3 CNY
2024-03-01 price USD 7.0 CNY
2024-03-02 price USD 7.4 CNY
2024-03-02 price USD 0 CNY
"#,
        &["CNY", "USD"],
        "2024-01-31",
        "2024-03-03",
    );
    assert_eq!(prices.rate("USD", "CNY", day("2024-02-01")), Some(number("7.2")));
    // a price at 09:00 sorts after the one at midnight of the same day
    assert_eq!(prices.rate("USD", "CNY", day("2024-03-01")), Some(number("7.3")));
    // a zero price is a price, but it has no inverse: the inverse falls back to the previous day
    assert_eq!(prices.rate("USD", "CNY", day("2024-03-02")), Some(number("0")));
    assert_eq!(prices.rate("CNY", "USD", day("2024-03-02")), Some(number("0.1369863013698630136986301370")));
}

#[test]
fn a_pair_quoted_in_both_directions_merges_into_one_history() {
    // USD -> CNY has more quotes, so the CNY -> USD quotes are inverted into its history
    let prices = parity(
        r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
2024-01-01 price USD 7 CNY
2024-02-01 price CNY 0.125 USD
2024-03-01 price USD 7.5 CNY
2024-03-01 price CNY 0.2 USD
2024-04-01 price USD 6 CNY
"#,
        &["CNY", "USD"],
        "2023-12-31",
        "2024-04-02",
    );
    assert_eq!(
        prices.rate("USD", "CNY", day("2024-02-15")),
        Some(number("8")),
        "the latest quote in either direction"
    );
    assert_eq!(
        prices.rate("USD", "CNY", day("2024-03-01")),
        Some(number("5")),
        "the fewer-quoted direction wins its day"
    );
    assert_eq!(prices.rate("CNY", "USD", day("2024-03-01")), Some(number("0.2")));
    assert_eq!(
        prices.rate("CNY", "USD", day("2024-04-02")).unwrap().to_string(),
        "0.1666666666666666666666666667"
    );

    // on a tie the direction quoted first is kept
    let prices = parity(
        r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
2024-01-01 price USD 7 CNY
2024-01-01 price CNY 0.125 USD
"#,
        &["CNY", "USD"],
        "2023-12-31",
        "2024-01-02",
    );
    assert_eq!(prices.rate("USD", "CNY", day("2024-01-01")), Some(number("8")));
    assert_eq!(prices.rate("CNY", "USD", day("2024-01-01")), Some(number("0.125")));
}
