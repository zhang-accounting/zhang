//! The ledger of #496 through `GET /api/errors`: `open Assets:Bank USD` restricts the account to USD, as in beancount,
//! so a posting in EUR is reported as `CommodityNotAllowed` on its transaction, line 7, where bean-check reports
//! `Invalid currency EUR for account 'Assets:Bank'`. Both formats report it the same way, and the transaction is
//! still booked.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::{JournalRequest, QueryRequest};
use zhang_server::routes::common::get_errors;
use zhang_server::routes::query::run_query;
use zhang_server::state::SharedLedger;

/// The ledger of #496, the same in both formats.
const LEDGER: &str = "option \"operating_currency\" \"USD\"\n\
                      1970-01-01 commodity USD\n\
                      1970-01-01 commodity EUR\n\
                      2024-01-01 open Assets:Bank USD\n\
                      2024-01-01 open Equity:Opening\n\
                      \n\
                      2024-01-10 * \"Deposit in the wrong currency\"\n  Assets:Bank 100 EUR\n  Equity:Opening -100 EUR\n";

/// A temporary ledger directory, removed when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn with(main: &str, content: &str) -> ScratchDir {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-open-commodities-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join(main), content).unwrap();
        dir
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The ledger of `dir`, loaded the way the server loads one, in the format of its `main` file.
async fn load(dir: &Path, main: &str) -> SharedLedger {
    let source: Arc<dyn zhang_core::data_source::DataSource> = if main.ends_with(".bean") {
        Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
    } else {
        Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
    };
    let ledger = Ledger::async_load(dir.to_path_buf(), main.to_owned(), source)
        .await
        .unwrap_or_else(|error| panic!("{main} should load: {error}"));
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

async fn body(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The first page of `GET /api/errors`.
async fn errors(ledger: &SharedLedger) -> Vec<Value> {
    let request = JournalRequest {
        page: None,
        size: None,
        keyword: None,
        tags: None,
        links: None,
    };
    body(get_errors(State(ledger.clone()), Query(request)).await).await["data"]["records"]
        .as_array()
        .unwrap()
        .clone()
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
    let dir = ScratchDir::with(main, LEDGER);
    let ledger = load(&dir.0, main).await;

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{main}: {errors:?}");
    assert_eq!(errors[0]["error_type"], "CommodityNotAllowed", "{main}");
    assert_eq!(errors[0]["metas"], json!({"account_name": "Assets:Bank", "commodity": "EUR"}), "{main}");
    let span = &errors[0]["span"];
    assert_eq!(span["filename"], main);
    assert_eq!(span["line"].as_u64(), Some(7), "{main}: {span}");

    // the table the endpoint reads, with the message the UI shows
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
