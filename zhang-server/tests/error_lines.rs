//! `GET /api/errors` locates an error by the lines of its directive (#493): the span of an error has the 1-based
//! `line` and `column` where its directive starts, next to `start` and `end`, which stay the byte offsets writers
//! replace the directive by. The `#errors` table the endpoint reads has the same `line` and `column`, for both formats.

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

/// A temporary ledger directory, removed when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn with(main: &str, content: &str) -> ScratchDir {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-error-lines-{}", uuid::Uuid::new_v4())));
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

#[tokio::test]
async fn an_error_is_located_by_the_lines_of_its_directive() {
    let dir = ScratchDir::with("main.zhang", ZHANG);
    let ledger = load(&dir.0, "main.zhang").await;

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["error_type"], "UnbalancedTransaction");
    let span = &errors[0]["span"];
    assert_eq!(span["filename"], "main.zhang");
    assert_eq!((span["line"].as_u64(), span["column"].as_u64()), (Some(5), Some(1)), "{span}");
    // the byte offsets stay: a writer replaces the directive by them
    assert_eq!((span["start"].as_u64(), span["end"].as_u64()), (Some(98), Some(157)), "{span}");
    assert_eq!(span["content"], "2024-01-10 \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY");

    // the table the endpoint reads
    assert_eq!(
        query(&ledger, "SELECT line, column, span_start, span_end FROM #errors").await,
        json!([[5, 1, 98, 157]])
    );
}

#[tokio::test]
async fn a_beancount_error_is_located_the_same_way() {
    let dir = ScratchDir::with("main.bean", BEANCOUNT);
    let ledger = load(&dir.0, "main.bean").await;

    let errors = errors(&ledger).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    let span = &errors[0]["span"];
    assert_eq!(
        (span["line"].as_u64(), span["column"].as_u64(), span["start"].as_u64()),
        (Some(5), Some(1), Some(98)),
        "{span}"
    );
    assert_eq!(query(&ledger, "SELECT line, column FROM #errors").await, json!([[5, 1]]));
}
