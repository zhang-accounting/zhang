//! The ledger of #496 through the error box's query (`journals.errors`): `open Assets:Bank USD` restricts the account to USD, as in beancount,
//! so a posting in EUR is reported as `CommodityNotAllowed` on its transaction, line 7, where bean-check reports
//! `Invalid currency EUR for account 'Assets:Bank'`. Both formats report it the same way, and the transaction is
//! still booked.

use axum::extract::{Path as UrlPath, State};
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::{BuiltinQueryRunRequest, QueryRequest};
use zhang_server::routes::query::{run_builtin_query, run_query};
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{body, shared};
use zhang_testkit::ledger::Scratch;

/// The ledger of #496, the same in both formats.
const LEDGER: &str = "option \"operating_currency\" \"USD\"\n\
                      1970-01-01 commodity USD\n\
                      1970-01-01 commodity EUR\n\
                      2024-01-01 open Assets:Bank USD\n\
                      2024-01-01 open Equity:Opening\n\
                      \n\
                      2024-01-10 * \"Deposit in the wrong currency\"\n  Assets:Bank 100 EUR\n  Equity:Opening -100 EUR\n";

/// The ledger of `scratch`, loaded the way the server loads one, in the format of its main file.
fn load(scratch: &Scratch) -> SharedLedger {
    shared(scratch.ledger().unwrap_or_else(|error| panic!("{} should load: {error}", scratch.main())))
}

/// The first page of the error box: the rows of `journals.errors`, as objects keyed by column name.
async fn errors(ledger: &SharedLedger) -> Vec<Value> {
    let request = BuiltinQueryRunRequest {
        params: serde_json::from_value(json!({ "size": 100, "offset": 0 })).unwrap(),
        count_total: None,
    };
    let result = body(run_builtin_query(State(ledger.clone()), UrlPath(("journals.errors".to_owned(),)), Json(request)).await).await;
    let columns = result["data"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    result["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| Value::Object(columns.iter().cloned().zip(row.as_array().unwrap().iter().cloned()).collect()))
        .collect()
}

/// The metas of a row, by key.
fn metas(row: &Value) -> Value {
    Value::Object(
        row["metas"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| (it["key"].as_str().unwrap().to_owned(), it["value"].clone()))
            .collect(),
    )
}

/// The rows of `POST /api/query`.
async fn query(ledger: &SharedLedger, sql: &str) -> Value {
    let request = QueryRequest {
        query: sql.to_owned(),
        count_total: None,
    };
    body(run_query(State(ledger.clone()), Json(request)).await).await["data"]["rows"].clone()
}

async fn the_posting_in_eur_is_reported_on_its_transaction(main: &str) {
    let dir = Scratch::with_main(main, LEDGER);
    let ledger = load(&dir);

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{main}: {errors:?}");
    assert_eq!(errors[0]["kind"], "CommodityNotAllowed", "{main}");
    assert_eq!(metas(&errors[0]), json!({"account_name": "Assets:Bank", "commodity": "EUR"}), "{main}");
    let span = &errors[0];
    assert_eq!(span["file"], main);
    assert_eq!(span["line"].as_u64(), Some(7), "{main}: {span}");

    // the table the error box reads, with the message the UI shows
    assert_eq!(
        query(&ledger, "SELECT line, kind, account, meta('commodity'), message FROM #errors").await,
        json!([[
            7,
            "CommodityNotAllowed",
            "Assets:Bank",
            "EUR",
            "Commodity EUR is not allowed in Assets:Bank: its open lists other commodities"
        ]]),
        "{main}"
    );

    // the transaction is still booked
    assert_eq!(
        query(&ledger, "SELECT account, currency WHERE account = 'Assets:Bank'").await,
        json!([["Assets:Bank", "EUR"]]),
        "{main}"
    );
}

#[tokio::test]
async fn a_zhang_ledger_reports_a_posting_in_a_commodity_its_open_does_not_list() {
    the_posting_in_eur_is_reported_on_its_transaction("main.zhang").await;
}

#[tokio::test]
async fn a_beancount_ledger_reports_it_where_bean_check_does() {
    the_posting_in_eur_is_reported_on_its_transaction("main.bean").await;
}
