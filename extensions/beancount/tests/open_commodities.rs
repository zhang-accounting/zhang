//! The commodities an `open` lists, checked against Python beancount (#496): for every ledger in
//! `open_commodities/`, zhang reports `CommodityNotAllowed` on the same lines, for the same accounts and
//! commodities, as beancount 3.2.3 reports `Invalid currency` (`open_commodities/oracle.json`, written by
//! `open_commodities/generate.py`): a posting by its units only, a balance assertion, and the padding of a `pad` on
//! the `pad`. A ledger the oracle marks with an `accepted_deviation` is checked against zhang's own rule instead.

use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use serde_json::Value;
use zhang_ast::error::ErrorKind;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::ledger::Ledger;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/open_commodities")
}

fn oracle_cases() -> serde_json::Map<String, Value> {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap();
    oracle.as_object().unwrap().clone()
}

/// (line, account, commodity) of an error
type Located = (usize, String, String);

/// the errors beancount reports, sorted: it reports the balance assertions before the postings
fn beancount(case: &Value) -> Vec<Located> {
    assert_eq!(case["other_errors"], Value::Array(vec![]), "the oracle ledgers have no other errors");
    let mut errors = case["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            (
                it["line"].as_u64().unwrap() as usize,
                it["account"].as_str().unwrap().to_owned(),
                it["currency"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    errors.sort();
    errors
}

/// the `CommodityNotAllowed` errors zhang reports, sorted; it reports no other error
fn zhang(case: &str) -> Vec<Located> {
    let data_source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    let ledger = Ledger::load_with_data_source(dir(), format!("{case}.bean"), data_source).expect("the ledger loads");
    let other_errors = ledger
        .errors
        .iter()
        .filter(|error| error.error_type != ErrorKind::CommodityNotAllowed)
        .map(|error| error.error_type.clone())
        .collect::<Vec<_>>();
    assert!(other_errors.is_empty(), "{case}: {other_errors:?}");
    let mut errors = ledger
        .errors
        .iter()
        .map(|error| {
            (
                error
                    .span
                    .as_ref()
                    .and_then(|span| span.line)
                    .expect("the error is on a directive of the ledger"),
                error.metas["account_name"].clone(),
                error.metas["commodity"].clone(),
            )
        })
        .collect::<Vec<_>>();
    errors.sort();
    errors
}

const DEVIATIONS: &[&str] = &["split_reduction"];

#[test]
fn zhang_reports_where_beancount_does_on_every_ledger_without_an_accepted_deviation() {
    let mut differing = vec![];
    for (case, expected) in oracle_cases() {
        if !expected["accepted_deviation"].is_null() {
            assert!(
                DEVIATIONS.contains(&case.as_str()),
                "{case}: an accepted deviation needs a test of zhang's own result"
            );
            continue;
        }
        let expected = beancount(&expected);
        let actual = zhang(&case);
        if actual != expected {
            differing.push(format!("{case}:\n  zhang:     {actual:?}\n  beancount: {expected:?}"));
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}

#[test]
fn a_reduction_booked_against_several_lots_is_reported_once_as_written() {
    let expected = &oracle_cases()["split_reduction"];
    assert!(expected["accepted_deviation"].as_str().unwrap().contains("once as written"));
    let located = |line: usize| (line, "Assets:Bank".to_owned(), "AAPL".to_owned());

    // beancount reports each of the two lots the sale on line 12 is booked against
    assert_eq!(beancount(expected), vec![located(7), located(7), located(12), located(12)]);
    // zhang reports the two postings of line 7, and the one posting of line 12 once
    assert_eq!(zhang("split_reduction"), vec![located(7), located(7), located(12)]);
}
