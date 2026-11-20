//! The numbers a request writes have at most a thousand digits written out in full, in plain notation as every write
//! writes one, before and after the point together (`validate::MAX_WRITTEN_DIGITS`). A request could give an amount
//! with any exponent: `{"number": "1e300000000"}` was written out with three hundred million digits, which took a
//! preview a minute and hundreds of megabytes with the ledger held; `1e1000000` was written into a file the ledger then
//! refused to load; and `1e-999999` and `1e999998`, both within the parser's bound, booked a residual of two million
//! digits, which every reload computed again for seconds. The create, the update and both previews refuse such a
//! number first, as a field of the request, and nothing is written; so do the balances, for the difference a padding
//! books too.

use std::str::FromStr;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use zhang_ast::amount::Amount;
use zhang_server::request::{AccountBalanceRequest, CreateTransactionRequest};
use zhang_server::routes::account::create_account_balance;
use zhang_server::routes::transaction::{create_new_transaction, preview_new_transaction, preview_transaction_update, update_single_transaction};
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{reload, respond, shared};
use zhang_testkit::ledger::Scratch;

const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 commodity USD\n1970-01-01 commodity STK\n\
    1970-01-01 open Assets:Cash\n1970-01-01 open Assets:Stock\n1970-01-01 open Expenses:Food\n\n\
    2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n";

/// A request on 2024-01-15 with `postings`.
fn request(postings: Value) -> CreateTransactionRequest {
    serde_json::from_value(json!({
        "datetime": "2024-01-15T12:00:00",
        "payee": "Shop",
        "narration": "",
        "postings": postings,
        "metas": [],
        "tags": [],
        "links": [],
    }))
    .unwrap()
}

/// The postings of a request whose first posting has `field` set to `value`: units as an amount or as text, a cost
/// or a price.
fn postings(field: &str, value: Value) -> Value {
    let mut first = json!({"account": "Assets:Stock", "unit": "1 STK"});
    first[field] = value;
    json!([first, {"account": "Assets:Cash", "unit": null}])
}

/// the values a request gives that are out of range written out, each with the field it is given in and the value
/// the error names
fn out_of_range() -> Vec<(&'static str, Value, &'static str, &'static str)> {
    vec![
        ("unit", json!({"number": "1e300000000", "commodity": "STK"}), "unit", "1e300000000"),
        ("unit", json!({"number": "-1e1000000", "commodity": "STK"}), "unit", "-1e1000000"),
        ("unit", json!({"number": "1e-1000000", "commodity": "STK"}), "unit", "1e-1000000"),
        // one digit too many
        ("unit", json!({"number": "1e1000", "commodity": "STK"}), "unit", "1e1000"),
        ("unit", json!({"number": "1e-1000", "commodity": "STK"}), "unit", "1e-1000"),
        // a literal the parser reads, which written out has a million digits
        ("unit", json!("1e999999 STK"), "unit", "1e999999"),
        ("cost", json!("{1e1000 USD}"), "cost", "1e1000"),
        ("price", json!("@ 1e1000 USD"), "price", "1e1000"),
    ]
}

/// a scratch ledger, served
fn served() -> (Scratch, SharedLedger) {
    let scratch = Scratch::zhang(LEDGER);
    let ledger = shared(scratch.ledger().expect("the ledger loads"));
    (scratch, ledger)
}

#[tokio::test]
async fn a_create_or_an_update_with_a_number_out_of_range_writes_nothing() {
    for (field, value, _, named) in out_of_range() {
        let (scratch, ledger) = served();
        let (status, body) = respond(create_new_transaction(State(ledger.clone()), reload(), Json(request(postings(field, value.clone())))).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}: {body}");
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("{named:?}")) && message.contains("number out of range"),
            "{value}: {message}"
        );
        assert!(!scratch.dir().join("data").exists(), "{value}: nothing is written");

        let id = ledger.read().await.transactions()[0].0.to_string();
        let update = update_single_transaction(State(ledger.clone()), reload(), UrlPath((id,)), Json(request(postings(field, value.clone()))));
        let (status, body) = respond(update.await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}: {body}");
        assert_eq!(scratch.read("main.zhang"), LEDGER, "{value}: nothing is written");
    }
}

#[tokio::test]
async fn both_previews_name_a_number_out_of_range_as_a_field() {
    for (field, value, error_field, named) in out_of_range() {
        let (scratch, ledger) = served();
        let id = ledger.read().await.transactions()[0].0.to_string();
        let new = respond(preview_new_transaction(State(ledger.clone()), Json(request(postings(field, value.clone())))).await).await;
        let update = respond(preview_transaction_update(State(ledger.clone()), UrlPath((id,)), Json(request(postings(field, value.clone())))).await).await;
        for (status, body) in [new, update] {
            assert_eq!(status, StatusCode::OK, "{value}: {body}");
            let preview = &body["data"];
            assert_eq!(preview["text"], Value::Null, "{value}: nothing is written out");
            let error = &preview["field_errors"][0];
            assert_eq!(
                (&error["posting"], &error["field"], &error["kind"], &error["value"]),
                (&json!(0), &json!(error_field), &json!("invalid_amount"), &json!(named)),
                "{value}: {preview}"
            );
        }
        assert_eq!(scratch.read("main.zhang"), LEDGER);
    }
}

/// A large number within the bound is written, and the ledger reads it back.
#[tokio::test]
async fn a_large_number_within_the_bound_is_written_and_read_back() {
    let (scratch, ledger) = served();
    let value = json!({"number": "1e40", "commodity": "STK"});
    let (status, body) = respond(create_new_transaction(State(ledger.clone()), reload(), Json(request(postings("unit", value)))).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reloaded = scratch.ledger().expect("the ledger reads it back");
    assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
    assert!(scratch
        .read("data/2024/01.zhang")
        .contains("Assets:Stock 10000000000000000000000000000000000000000 STK"));
}

/// Numbers far apart within the bound are written, and booked cheaply: their residual has at most about two thousand
/// digits. Those of the parser's bound are refused at once.
#[tokio::test]
async fn numbers_far_apart_within_the_bound_are_cheap_and_those_beyond_refused() {
    let (scratch, ledger) = served();
    let far_apart = |small: &str, large: &str| {
        json!([
            {"account": "Assets:Stock", "unit": format!("{small} STK")},
            {"account": "Assets:Cash", "unit": format!("{large} STK")},
            {"account": "Expenses:Food", "unit": null},
        ])
    };
    let (status, body) = respond(create_new_transaction(State(ledger.clone()), reload(), Json(request(far_apart("1e-999999", "1e999998")))).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!scratch.dir().join("data").exists(), "nothing is written");

    let started = std::time::Instant::now();
    let (status, body) = respond(create_new_transaction(State(ledger.clone()), reload(), Json(request(far_apart("1e-999", "1e999")))).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reloaded = scratch.ledger().expect("the ledger reads them back");
    assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

/// A padding books the difference between the balance asked for and the one held: computed, it is bounded as a number
/// of the request is. With `-9.5 CNY` held, a balance of `1e999` (a thousand digits) pads by `1e999 + 9.5`, which has
/// a thousand and one: refused, and nothing is written.
#[tokio::test]
async fn a_padding_whose_difference_has_too_many_digits_is_refused() {
    let main = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Equity:Open\n\
                1970-01-01 open Expenses:Food\n\n2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -9.5 CNY\n  Expenses:Food\n";
    let scratch = Scratch::beancount(main);
    let ledger = shared(scratch.ledger().expect("the ledger loads"));
    let pad = |number: &str| AccountBalanceRequest::Pad {
        amount: Amount::new(BigDecimal::from_str(number).unwrap(), "CNY"),
        pad: "Equity:Open".to_owned(),
    };
    let (status, body) = respond(create_account_balance(State(ledger.clone()), reload(), UrlPath(("Assets:Cash".to_owned(),)), Json(pad("1e999"))).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("it has 1001 digits"), "{body}");
    assert_eq!(scratch.read("main.bean"), main, "nothing is written");
    assert!(!scratch.dir().join("data").exists(), "nothing is written");

    // within the bound, written and read back
    let (status, body) = respond(create_account_balance(State(ledger.clone()), reload(), UrlPath(("Assets:Cash".to_owned(),)), Json(pad("1e998"))).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reloaded = scratch.ledger().expect("the ledger reads it back");
    assert!(reloaded.errors.is_empty(), "{:?}", reloaded.errors);
}
