//! When an account is active, checked against Python beancount: for every ledger in `active_accounts/`, zhang reports
//! `AccountDoesNotExist` or `AccountClosed` on the same lines, for the same accounts, as beancount 3.2.3 reports
//! `Invalid reference to inactive account` or `Invalid reference to unknown account` (`active_accounts/oracle.json`,
//! written by `active_accounts/generate.py`). Both keep an account active through the day of its `close`. A ledger the
//! oracle marks with an `accepted_deviation` is checked against zhang's own rule instead.

use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use serde_json::Value;
use zhang_ast::error::ErrorKind;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::ledger::Ledger;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/active_accounts")
}

fn oracle_cases() -> serde_json::Map<String, Value> {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap();
    oracle.as_object().unwrap().clone()
}

/// (line, account) of an error
type Located = (usize, String);

/// the errors beancount reports, sorted
fn beancount(case: &Value) -> Vec<Located> {
    assert_eq!(case["other_errors"], Value::Array(vec![]), "the oracle ledgers have no other errors");
    let mut errors = case["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (it["line"].as_u64().unwrap() as usize, it["account"].as_str().unwrap().to_owned()))
        .collect::<Vec<_>>();
    errors.sort();
    errors
}

/// the `AccountDoesNotExist` and `AccountClosed` errors zhang reports, sorted; it reports no other error
fn zhang(case: &str) -> Vec<Located> {
    let data_source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    let ledger = Ledger::load_with_data_source(dir(), format!("{case}.bean"), data_source).expect("the ledger loads");
    let store = ledger.store.read().unwrap();
    let inactive = |kind: &ErrorKind| matches!(kind, ErrorKind::AccountDoesNotExist | ErrorKind::AccountClosed);
    let other_errors = store
        .errors
        .iter()
        .filter(|error| !inactive(&error.error_type))
        .map(|error| error.error_type.clone())
        .collect::<Vec<_>>();
    assert!(other_errors.is_empty(), "{case}: {other_errors:?}");
    let mut errors = store
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
            )
        })
        .collect::<Vec<_>>();
    errors.sort();
    errors
}

/// the kinds of the errors zhang reports, in its order
fn kinds(case: &str) -> Vec<ErrorKind> {
    let data_source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    let ledger = Ledger::load_with_data_source(dir(), format!("{case}.bean"), data_source).expect("the ledger loads");
    let store = ledger.store.read().unwrap();
    store.errors.iter().map(|error| error.error_type.clone()).collect()
}

const DEVIATIONS: &[&str] = &["after_close", "close_time"];

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
fn an_account_is_active_through_the_day_of_its_close() {
    // everything on the close day passes, as in beancount, and the transaction of the day after is reported
    assert_eq!(zhang("close_day"), vec![(26, "Assets:Old".to_owned())]);
    assert_eq!(kinds("close_day"), vec![ErrorKind::AccountClosed]);
    assert_eq!(kinds("before_open"), vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountDoesNotExist]);
}

#[test]
fn a_balance_and_a_document_after_the_close_are_reported() {
    let expected = &oracle_cases()["after_close"];
    assert!(expected["accepted_deviation"].as_str().unwrap().contains("balance and a document"));
    assert_eq!(beancount(expected), vec![]);
    assert_eq!(zhang("after_close"), vec![(6, "Assets:Old".to_owned()), (7, "Assets:Old".to_owned())]);
    assert_eq!(kinds("after_close"), vec![ErrorKind::AccountClosed, ErrorKind::AccountClosed]);
}

#[test]
fn a_close_with_a_time_closes_the_account_at_that_time() {
    let expected = &oracle_cases()["close_time"];
    assert!(expected["accepted_deviation"].as_str().unwrap().contains("time metadata of a close"));
    assert_eq!(beancount(expected), vec![]);
    assert_eq!(zhang("close_time"), vec![(8, "Assets:A".to_owned())]);
}
