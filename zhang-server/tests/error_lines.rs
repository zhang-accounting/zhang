//! The error box locates an error by the lines of its directive (#493): a row of `journals.errors` has the 1-based
//! `line` and `column` where its directive starts, next to `span_start` and `span_end`, which stay the byte offsets
//! writers replace the directive by. The `#errors` table it reads has the same `line` and `column`, for both formats.

use axum::extract::{Path as UrlPath, State};
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::{BuiltinQueryRunRequest, QueryRequest};
use zhang_server::routes::query::{run_builtin_query, run_query};
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{body, shared};
use zhang_testkit::ledger::Scratch;

/// The ledger of #493: an unbalanced transaction on lines 5 to 7, at bytes 98 to 157.
const ZHANG: &str = "option \"operating_currency\" \"CNY\"\n\
                     1970-01-01 open Assets:A CNY\n\
                     1970-01-01 open Expenses:Food CNY\n\
                     \n\
                     2024-01-10 \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY\n";

/// The same ledger as beancount: the transaction starts at the same line and byte.
const BEANCOUNT: &str = "option \"operating_currency\" \"CNY\"\n\
                         1970-01-01 open Assets:A CNY\n\
                         1970-01-01 open Expenses:Food CNY\n\
                         \n\
                         2024-01-10 * \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY\n";

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

/// The rows of `POST /api/query`.
async fn query(ledger: &SharedLedger, sql: &str) -> Value {
    let request = QueryRequest {
        query: sql.to_owned(),
        count_total: None,
    };
    body(run_query(State(ledger.clone()), Json(request)).await).await["data"]["rows"].clone()
}

#[tokio::test]
async fn an_error_is_located_by_the_lines_of_its_directive() {
    let dir = Scratch::zhang(ZHANG);
    let ledger = load(&dir);

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["kind"], "UnbalancedTransaction");
    let span = &errors[0];
    assert_eq!(span["file"], "main.zhang");
    assert_eq!((span["line"].as_u64(), span["column"].as_u64()), (Some(5), Some(1)), "{span}");
    // the byte offsets stay: a writer replaces the directive by them
    assert_eq!((span["span_start"].as_u64(), span["span_end"].as_u64()), (Some(98), Some(157)), "{span}");
    assert_eq!(span["source"], "2024-01-10 \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY");

    // the table the error box reads
    assert_eq!(
        query(&ledger, "SELECT line, column, span_start, span_end FROM #errors").await,
        json!([[5, 1, 98, 157]])
    );
}

#[tokio::test]
async fn a_beancount_error_is_located_the_same_way() {
    let dir = Scratch::beancount(BEANCOUNT);
    let ledger = load(&dir);

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    let span = &errors[0];
    assert_eq!(
        (span["line"].as_u64(), span["column"].as_u64(), span["span_start"].as_u64()),
        (Some(5), Some(1), Some(98)),
        "{span}"
    );
    assert_eq!(query(&ledger, "SELECT line, column FROM #errors").await, json!([[5, 1]]));
}
