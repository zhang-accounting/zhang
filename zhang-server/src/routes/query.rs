use axum::extract::State;
use axum::Json;
use gotcha::api;

use crate::request::QueryRequest;
use crate::response::{QueryResultEntity, QuerySchemaEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::ApiResult;

/// Run a BQL-compatible query over the ledger.
///
/// Query errors are answered with HTTP 400 and `{"message", "line", "column"}`.
#[api(group = "query")]
pub async fn run_query(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> ApiResult<QueryResultEntity> {
    let ledger = ledger.read().await;
    let result = zhang_query::execute(&ledger, &payload.query)?;
    ResponseWrapper::json(result.into())
}

/// The columns and functions available to queries.
#[api(group = "query")]
pub async fn get_query_schema() -> ApiResult<QuerySchemaEntity> {
    ResponseWrapper::json(zhang_query::schema().into())
}
