//! The cost spec forms of #497, checked against Python beancount: zhang books every transaction of
//! `cost_specs/ledger.bean` (a cost with only a date or only a label, components in any order, the
//! compound cost `{P # T USD}` and the merge-cost marker `{*}`) against the same lots as beancount
//! 3.2.3, ends with the same lots, reports the merge-cost marker as beancount does
//! (`cost_specs/oracle.json`, written by `cost_specs/generate.py`), and writes each transaction
//! as it was written, so that it reads back unchanged.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use beancount::Beancount;
use bigdecimal::BigDecimal;
use serde_json::Value;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Spanned, Transaction};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType as ColumnType, ParamTypes, Params, Query};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cost_specs")
}

fn load() -> Ledger {
    let data_source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    Ledger::load_with_data_source(dir(), "ledger.bean".to_owned(), data_source).expect("the ledger loads")
}

fn oracle() -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap()
}

fn number(value: &Value) -> BigDecimal {
    BigDecimal::from_str(value.as_str().expect("a number is a string")).unwrap()
}

fn text(value: &Value) -> String {
    value.as_str().unwrap().to_owned()
}

/// (number, currency)
type Units = (BigDecimal, String);
/// (number, currency, acquisition date, label)
type Cost = (BigDecimal, String, String, Option<String>);
/// (account, units, cost of the lot)
type BookedPosting = (String, Units, Option<Cost>);

fn oracle_units(value: &Value) -> Units {
    (number(&value["number"]), text(&value["currency"]))
}

fn oracle_cost(value: &Value) -> Option<Cost> {
    value.as_object().map(|cost| {
        (
            number(&cost["number"]),
            text(&cost["currency"]),
            text(&cost["date"]),
            cost["label"].as_str().map(str::to_owned),
        )
    })
}

/// the booked transactions of the ledger, in date order
fn transactions(ledger: &Ledger) -> Vec<Transaction> {
    ledger
        .directives
        .iter()
        .filter_map(|directive| match &directive.data {
            Directive::Transaction(txn) => Some(txn.clone()),
            _ => None,
        })
        .collect()
}

fn booked_postings(txn: &Transaction) -> Vec<BookedPosting> {
    txn.postings
        .iter()
        .map(|posting| {
            let units = posting.units.as_ref().expect("a booked posting has units");
            let cost = posting.cost.as_ref().map(|cost| {
                let base = cost.base.as_ref().expect("a booked cost has a number");
                let date = cost.date.as_ref().expect("a booked cost has a date");
                (base.number.clone(), base.commodity.clone(), date.naive_date().to_string(), cost.label.clone())
            });
            (posting.account.name().to_owned(), (units.number.clone(), units.commodity.clone()), cost)
        })
        .collect()
}

#[test]
fn every_cost_spec_form_books_as_beancount_books_it() {
    let ledger = load();
    let oracle = oracle();

    let expected: Vec<(String, String, Vec<BookedPosting>)> = oracle["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|txn| {
            let postings = txn["postings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|posting| (text(&posting["account"]), oracle_units(&posting["units"]), oracle_cost(&posting["cost"])))
                .collect();
            (text(&txn["date"]), text(&txn["narration"]), postings)
        })
        .collect();
    let booked: Vec<(String, String, Vec<BookedPosting>)> = transactions(&ledger)
        .iter()
        .map(|txn| {
            (
                txn.date.naive_date().to_string(),
                txn.narration.clone().map(|it| it.to_plain_string()).unwrap_or_default(),
                booked_postings(txn),
            )
        })
        .collect();
    assert_eq!(booked, expected);
}

#[test]
fn the_lots_at_the_end_are_beancounts() {
    let ledger = load();
    let oracle = oracle();

    let expected: BTreeMap<String, Vec<(Units, Option<Cost>)>> = oracle["positions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(account, positions)| {
            let positions = positions
                .as_array()
                .unwrap()
                .iter()
                .map(|position| (oracle_units(&position["units"]), oracle_cost(&position["cost"])))
                .collect();
            (account.clone(), positions)
        })
        .collect();
    // the lots the query engine lists (`commodities.lots`): the booked postings by commodity and lot, those with units
    // left, in the order they were opened
    let query = Query::compile_with_params(
        "SELECT sum(number), currency, cost_number, cost_currency, cost_date, cost_label WHERE account = :account \
         GROUP BY currency, cost_number, cost_currency, cost_date, cost_label HAVING sum(number) != 0 \
         ORDER BY first(seq), first(posting_index)",
        &ParamTypes::new().bind("account", ColumnType::Str),
    )
    .unwrap();
    let lots: BTreeMap<String, Vec<(Units, Option<Cost>)>> = expected
        .keys()
        .map(|account| {
            let result = query.execute(&ledger, &Params::new().bind("account", account.as_str())).unwrap();
            let lots = result
                .rows
                .iter()
                .map(|lot| {
                    let cost = lot[2].as_decimal().map(|number| {
                        (
                            number,
                            lot[3].to_string(),
                            lot[4].as_date().expect("a lot at cost has a date").to_string(),
                            lot[5].as_str().map(str::to_owned),
                        )
                    });
                    ((lot[0].as_decimal().unwrap(), lot[1].to_string()), cost)
                })
                .collect();
            (account.clone(), lots)
        })
        .collect();
    assert_eq!(lots, expected);
}

/// beancount reports "Cost merging is not supported yet" on the transaction written with `{*}`,
/// and nothing else; zhang reports `CostMergingNotSupported` on that transaction, once, and nothing else
#[test]
fn the_merge_cost_marker_is_reported_once_as_in_beancount() {
    let ledger = load();
    let oracle = oracle();

    let errors = oracle["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(text(&errors[0]["message"]), "Cost merging is not supported yet");
    let line = errors[0]["lineno"].as_u64().unwrap();
    // the transaction the error is on: the last one starting at or before the error's line
    let narration = oracle["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|txn| txn["lineno"].as_u64().unwrap() <= line)
        .map(|txn| text(&txn["narration"]))
        .unwrap();

    let reported: Vec<(ErrorKind, String)> = ledger
        .errors
        .iter()
        .map(|error| {
            let span = error.span.as_ref().and_then(|span| span.content.lines().next()).unwrap_or_default();
            (error.error_type.clone(), span.to_owned())
        })
        .collect();
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].0, ErrorKind::CostMergingNotSupported);
    assert!(reported[0].1.contains(&format!("\"{narration}\"")), "{reported:?}");
}

/// every transaction, booked, is written as the user wrote it (the cost spec as written, the
/// implicit posting without units) and reads back as that written form
#[test]
fn every_transaction_round_trips_as_written() {
    let ledger = load();
    for txn in transactions(&ledger) {
        let written = Transaction {
            postings: txn.written_postings(),
            ..txn.clone()
        };
        let exported = Beancount::default().export(Spanned::new(Directive::Transaction(written.clone()), SpanInfo::default()));
        let reparsed: Vec<Transaction> = Beancount::default()
            .transform(exported.clone(), None)
            .unwrap_or_else(|err| panic!("cannot parse the export of {written:?}: {err}\n{exported}"))
            .into_iter()
            .filter_map(|directive| match directive.data {
                Directive::Transaction(txn) => Some(txn),
                _ => None,
            })
            .collect();
        assert_eq!(reparsed, vec![written], "exported as:\n{exported}");
    }
}
