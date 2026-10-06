//! Posting flags (#474) through `POST /api/query`, on the `integration-tests` posting flag ledgers:
//! the `posting_flag` column has the flag written before each posting, and is NULL for a posting
//! without one, as in beanquery. An indented line starting with `*` is a posting flagged `*` in both
//! formats. One starting with `#` is a comment in a zhang file, so `posting-flags-star-hash-zhang`
//! books none of those lines, while `posting-flags-star-hash-beancount` books them as postings
//! flagged `#`, as beancount does.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::QueryRequest;
use zhang_server::routes::query::run_query;
use zhang_server::state::SharedLedger;

/// The fixture `name` with its `main` file, loaded the way the server loads a ledger.
async fn load(name: &str, main: &str) -> SharedLedger {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests").join(name);
    let source = if main.ends_with(".bean") {
        Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
    } else {
        Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
    };
    let ledger = Ledger::load(dir, main.to_owned(), source).unwrap_or_else(|error| panic!("{name}/{main} should load: {error}"));
    assert!(ledger.errors.is_empty(), "{name}/{main} has errors");
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// The rows `query` returns over the fixture.
async fn rows(name: &str, main: &str, query: &str) -> Value {
    let request = QueryRequest {
        query: query.to_owned(),
        count_total: None,
    };
    let response = run_query(State(load(name, main).await), Json(request)).await.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["data"]["rows"].clone()
}

const POSTINGS: &str = "SELECT date, flag, posting_flag, account, number";

#[tokio::test]
async fn both_formats_read_the_flags_they_share() {
    let expected = json!([
        ["2020-01-02", "*", null, "Assets:Cash", "100"],
        ["2020-01-02", "*", null, "Assets:Bank", "1000"],
        ["2020-01-02", "*", null, "Equity:Opening", "-1100"],
        ["2020-01-10", "*", "!", "Assets:Cash", "-10"],
        ["2020-01-10", "*", null, "Expenses:Food", "10"],
        ["2020-01-11", "!", "!", "Assets:Cash", "-20"],
        ["2020-01-11", "!", "X", "Expenses:Food", "15"],
        ["2020-01-11", "!", "?", "Expenses:Drinks", "5"],
        ["2020-01-12", "*", "&", "Assets:Bank", "-1"],
        ["2020-01-12", "*", "?", "Assets:Bank", "-2"],
        ["2020-01-12", "*", "%", "Assets:Bank", "-3"],
        ["2020-01-12", "*", "P", "Assets:Bank", "-4"],
        ["2020-01-12", "*", "C", "Expenses:Drinks", "10"],
    ]);
    for main in ["main.zhang", "main.bean"] {
        assert_eq!(rows("posting-flags", main, POSTINGS).await, expected, "{main}");
    }
}

#[tokio::test]
async fn star_lines_are_postings_in_both_formats_and_hash_lines_only_in_beancount() {
    // in a zhang file the `*` lines are postings flagged `*` and the `#` lines are comments: the
    // elided Lunch posting is inferred without the `#` line, and the Dinner amounts balance without it
    let zhang = rows("posting-flags-star-hash-zhang", "main.zhang", POSTINGS).await;
    assert_eq!(
        zhang,
        json!([
            ["2020-01-02", "*", null, "Assets:Cash", "100"],
            ["2020-01-02", "*", null, "Equity:Opening", "-100"],
            ["2020-01-10", "*", null, "Assets:Cash", "-10"],
            ["2020-01-10", "*", null, "Expenses:Food", "15"],
            ["2020-01-10", "*", "*", "Assets:Cash", "-5"],
            ["2020-01-11", "*", null, "Assets:Cash", "-27"],
            ["2020-01-11", "*", null, "Expenses:Food", "20"],
            ["2020-01-11", "*", "*", "Expenses:Drinks", "7"],
        ])
    );

    // in a beancount file the `*` and `#` lines are all postings, flagged `*` and `#`, as beancount reads them
    let beancount = rows("posting-flags-star-hash-beancount", "main.bean", POSTINGS).await;
    assert_eq!(
        beancount,
        json!([
            ["2020-01-02", "*", null, "Assets:Cash", "100"],
            ["2020-01-02", "*", null, "Equity:Opening", "-100"],
            ["2020-01-10", "*", null, "Assets:Cash", "-10"],
            ["2020-01-10", "*", null, "Expenses:Food", "10"],
            ["2020-01-10", "*", "*", "Assets:Cash", "-5"],
            ["2020-01-10", "*", "#", "Expenses:Food", "5"],
            ["2020-01-11", "*", null, "Assets:Cash", "-20"],
            ["2020-01-11", "*", null, "Expenses:Food", "20"],
            ["2020-01-11", "*", "#", "Assets:Cash", "-7"],
            ["2020-01-11", "*", "*", "Expenses:Drinks", "7"],
        ])
    );
}
