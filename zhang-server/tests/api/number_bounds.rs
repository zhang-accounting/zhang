//! The numbers of a transaction request are bounded as the ledger's parser bounds a number: written out in plain
//! notation, as every write writes one, with at most as many digits and decimal places as it reads back. A request
//! can give an amount with any exponent: `{"number": "1e300000000"}` was written out with three hundred million digits,
//! which took a preview a minute and hundreds of megabytes with the ledger held, and `1e1000000` was written into a
//! file the ledger then refused to load. The create, the update and both previews refuse such a number first, as
//! a field of the request, and nothing is written.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::CreateTransactionRequest;
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
        // a literal the parser reads, which written out has a million digits
        ("unit", json!("1e999999 STK"), "unit", "1e999999"),
        ("cost", json!("{1e999999 USD}"), "cost", "1e999999"),
        ("price", json!("@ 1e999999 USD"), "price", "1e999999"),
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
