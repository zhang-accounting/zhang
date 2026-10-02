use std::time::Duration;

use axum::extract::State;
use axum::Json;
use gotcha::api;
use zhang_query::{ExecuteOptions, Params, Query};

use crate::request::QueryRequest;
use crate::response::{QueryApiResult, QueryResultEntity, QuerySchemaEntity, ResponseWrapper};
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
