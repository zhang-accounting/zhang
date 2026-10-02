use std::time::Duration;

use axum::extract::State;
use axum::Json;
use gotcha::api;
use zhang_query::{ExecuteOptions, Params, Query};

use crate::request::QueryRequest;
use crate::response::{QueryApiResult, QueryResultEntity, QuerySchemaEntity, ResponseWrapper, SavedQueryEntity};
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// How long one query may run before it is stopped with a 400.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Run a BQL-compatible query over the ledger.
///
/// Query errors are answered with HTTP 400 and `{"message", "line", "column"}`.
#[api(group = "query")]
pub async fn run_query(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> QueryApiResult<QueryResultEntity> {
    QueryApiResult(execute(ledger.0 .0.clone(), payload.query).await)
}

async fn execute(ledger: std::sync::Arc<tokio::sync::RwLock<zhang_core::ledger::Ledger>>, text: String) -> ServerResult<ResponseWrapper<QueryResultEntity>> {
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
    ResponseWrapper::json(result.into())
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
mod test {
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
