use gotcha::State;
use gotcha::Json;
use zhang_sql::ExecutionResult;
use crate::error::ServerError;
use crate::request::SqlExecutionRequest;
use crate::response::ResponseWrapper;
use crate::state::SharedLedger;
use crate::ApiResult;
use zhang_sql::AsExecutor;
use gotcha::api;

#[api(group = "sql")]
pub async fn execute_sql(ledger: State<SharedLedger>, sql: Json<SqlExecutionRequest>) -> ApiResult<ExecutionResult> {
    let ledger = ledger.read().await;
    let executor = ledger.as_executor();
    let ret = executor.execute(&sql.sql).map_err(|e| ServerError::SqlError(e.to_string()))?;
    ResponseWrapper::json(ret)
}
