use std::time::Duration;

use axum::extract::State;
use axum::Json;
use gotcha::api;
use zhang_query::{ExecuteOptions, Params, Query, QueryResult};

use crate::request::QueryRequest;
use crate::response::{QueryApiResult, QueryCsvResult, QueryResultEntity, QuerySchemaEntity, ResponseWrapper, SavedQueryEntity};
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// How long one query may run before it is stopped with a 400.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Run a BQL-compatible query over the ledger.
///
/// Query errors are answered with HTTP 400 and `{"message", "line", "column"}`.
#[api(group = "query")]
pub async fn run_query(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> QueryApiResult<QueryResultEntity> {
    let result = execute(ledger.0 .0.clone(), payload.query).await;
    QueryApiResult(result.and_then(|result| ResponseWrapper::json(result.into())))
}

/// Run a BQL-compatible query and download the result as CSV (`query.csv`).
///
/// Amount, position and inventory columns are split into one numeric column per currency,
/// named like `balance (USD)`, as beanquery's numberify does. Query errors are answered
/// with HTTP 400 and `{"message", "line", "column"}`, as in `POST /api/query`.
#[api(group = "query")]
pub async fn run_query_csv(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> QueryCsvResult {
    QueryCsvResult(export_csv(ledger.0 .0.clone(), payload.query).await)
}

async fn export_csv(ledger: std::sync::Arc<tokio::sync::RwLock<zhang_core::ledger::Ledger>>, text: String) -> ServerResult<String> {
    let result = execute(ledger, text).await?;
    // the read lock is released by now; rendering is CPU-bound, so it stays off the async workers
    Ok(tokio::task::spawn_blocking(move || zhang_query::export::to_csv(&result)).await?)
}

/// Compile and run a query off the async workers, under the ledger read lock and the time
/// limit. The query length is capped by the parser.
async fn execute(ledger: std::sync::Arc<tokio::sync::RwLock<zhang_core::ledger::Ledger>>, text: String) -> ServerResult<QueryResult> {
    // compiling needs no ledger; run it, and the CPU-bound execution, off the async workers
    let query = tokio::task::spawn_blocking(move || Query::compile(&text)).await??;
    // an owned guard moves into the blocking task; the time limit bounds how long it is held
    let ledger = ledger.read_owned().await;
    let result = tokio::task::spawn_blocking(move || {
        let options = ExecuteOptions {
            today: None,
            timeout: Some(QUERY_TIMEOUT),
        };
        query.execute_with_options(&ledger, &Params::new(), &options)
    })
    .await??;
    Ok(result)
}

/// The columns and functions available to queries.
#[api(group = "query")]
pub async fn get_query_schema() -> ApiResult<QuerySchemaEntity> {
    ResponseWrapper::json(zhang_query::schema().into())
}

/// The queries saved in the ledger by `query` directives, in ledger order (by date, then
/// source order). Queries sharing a name are all listed.
///
/// Each query is compiled to report whether it is `valid` with the current engine; an
/// invalid one is still listed, with its `error`.
#[api(group = "query")]
pub async fn get_saved_queries(ledger: State<SharedLedger>) -> ApiResult<Vec<SavedQueryEntity>> {
    let queries = ledger.read().await.operations().queries()?;
    let saved = tokio::task::spawn_blocking(move || queries.into_iter().map(SavedQueryEntity::from).collect::<Vec<_>>()).await?;
    ResponseWrapper::json(saved)
}

#[cfg(test)]
mod saved_query_test {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::{Ledger, LedgerProcessContext};

    use crate::routes::query::get_saved_queries;
    use crate::state::SharedLedger;

    fn ledger(content: &str) -> SharedLedger {
        let directives = ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        let ledger = Ledger::process(LedgerProcessContext {
            directives,
            entry: (PathBuf::from("."), "main.zhang".to_owned()),
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
        })
        .unwrap();
        SharedLedger(Arc::new(tokio::sync::RwLock::new(ledger)))
    }

    #[tokio::test]
    async fn saved_queries_are_listed_in_ledger_order_with_their_validity() {
        let ledger = ledger(
            r#"
2024-03-01 query "cash" "SELECT date, position WHERE account ~ 'Cash'"
2024-01-01 query "by account" "SELECT account, sum(position) GROUP BY account"
  owner: "alice"
2024-01-01 query "broken" "SELECT nosuchcolumn"
2024-02-01 query "cash" "SELECT 1"
"#,
        );

        let response = get_saved_queries(State(ledger)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let error = zhang_query::Query::compile("SELECT nosuchcolumn").err().unwrap().to_string();
        assert_eq!(
            body,
            json!({"data": [
                {"name": "by account", "query": "SELECT account, sum(position) GROUP BY account", "date": "2024-01-01", "valid": true, "error": null},
                {"name": "broken", "query": "SELECT nosuchcolumn", "date": "2024-01-01", "valid": false, "error": error},
                {"name": "cash", "query": "SELECT 1", "date": "2024-02-01", "valid": true, "error": null},
                {"name": "cash", "query": "SELECT date, position WHERE account ~ 'Cash'", "date": "2024-03-01", "valid": true, "error": null},
            ]})
        );
    }

    #[tokio::test]
    async fn a_ledger_without_saved_queries_lists_none() {
        let response = get_saved_queries(State(ledger("2024-01-01 open Assets:Cash\n"))).await.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body, json!({"data": []}));
    }
}

#[cfg(test)]
mod csv_test {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::{header, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use gotcha::Responsible;
    use serde_json::json;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::run_query_csv;
    use crate::request::QueryRequest;
    use crate::response::QueryCsvResult;
    use crate::state::SharedLedger;

    const LEDGER: &str = r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity AAPL

1970-01-01 open Assets:Cash
1970-01-01 open Assets:Broker
1970-01-01 open Expenses:Food
1970-01-01 open Equity:Opening

2024-01-01 "Shop" "Lunch, with friends"
  Assets:Cash -12.50 CNY
  Expenses:Food 12.50 CNY

2024-01-02 "Broker" "Buy"
  Assets:Broker 2 AAPL {150.00 USD}
  Equity:Opening -300.00 USD
"#;

    async fn ledger() -> SharedLedger {
        let dir = std::env::temp_dir().join(format!("zhang-query-csv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zhang"), LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(dir.clone(), "main.zhang".to_owned(), source).await.expect("load ledger");
        // queries only read the in-memory store
        std::fs::remove_dir_all(dir).ok();
        SharedLedger(Arc::new(RwLock::new(ledger)))
    }

    async fn post_csv(query: &str) -> Response {
        let request = QueryRequest { query: query.to_owned() };
        run_query_csv(State(ledger().await), Json(request)).await.into_response()
    }

    async fn text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn query_csv_downloads_a_numberified_csv() {
        let response = post_csv("SELECT account, narration, sum(position) AS balance GROUP BY account, narration ORDER BY account").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/csv; charset=utf-8");
        assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "attachment; filename=\"query.csv\"");
        assert_eq!(
            text(response).await,
            // CNY occurs in two rows; USD and AAPL tie at one and order by name descending
            "account,narration,balance (CNY),balance (USD),balance (AAPL)\r\n\
             Assets:Broker,Buy,,,2\r\n\
             Assets:Cash,\"Lunch, with friends\",-12.50,,\r\n\
             Equity:Opening,Buy,,-300.00,\r\n\
             Expenses:Food,\"Lunch, with friends\",12.50,,\r\n"
        );
    }

    #[tokio::test]
    async fn query_csv_errors_are_the_query_api_400() {
        let response = post_csv("SELECT account WHERE").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
        assert_eq!(body, json!({"message": "expected an expression, found end of query", "line": 1, "column": 21}));

        let response = post_csv("SELECT nope").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
        assert_eq!(body["line"], 1);
        assert_eq!(body["column"], 8);
    }

    #[test]
    fn query_csv_is_declared_as_text_csv_with_a_400() {
        let responses = serde_json::to_value(<QueryCsvResult as Responsible>::response()).unwrap();
        assert_eq!(responses["200"]["content"]["text/csv; charset=utf-8"]["schema"]["type"], "string");
        assert_eq!(responses["200"]["headers"]["Content-Disposition"]["required"], true);
        let error = &responses["400"]["content"]["application/json"]["schema"];
        assert_eq!(error["required"], json!(["message", "line", "column"]));
    }
}
