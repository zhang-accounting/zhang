use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use gotcha::api;
use log::{info, warn};
use zhang_core::ledger::Ledger;
use zhang_query::{ExecuteOptions, Params, Query, QueryResult, DEFAULT_MAX_RESULT_VALUES};

use crate::builtin::{self, BUILTINS};
use crate::error::ServerError;
use crate::request::{BuiltinQueryBatchItem, BuiltinQueryRunRequest, BuiltinQueryTextRequest, QueryRequest};
use crate::response::{
    BuiltinQueryEntity, BuiltinQueryTextEntity, QueryApiResult, QueryCsvResult, QueryResultEntity, QuerySchemaEntity, ResponseWrapper, SavedQueryEntity,
};
use crate::state::SharedLedger;
use crate::{ApiResult, LedgerState, ServerResult};

/// How long one query may run before it is stopped with a 400.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// The environment variable that sets how large a query result may grow, in values (see
/// [`zhang_query::ExecuteOptions::max_result_values`]), before `/api/query` and
/// `/api/query/csv` stop the query with a 400. It also bounds the size of the JSON and CSV
/// encodings, so instances with little memory can lower it.
pub const MAX_RESULT_VALUES_ENV: &str = "ZHANG_QUERY_MAX_RESULT_VALUES";

/// The result size limit of the query API: [`MAX_RESULT_VALUES_ENV`] when it is a positive
/// integer, else the engine's [`DEFAULT_MAX_RESULT_VALUES`]. The variable is read once, at
/// startup ([`crate::start_server`] calls this first).
pub fn max_result_values() -> u64 {
    static LIMIT: OnceLock<u64> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let limit = match parse_max_result_values(std::env::var(MAX_RESULT_VALUES_ENV).ok().as_deref()) {
            Ok(limit) => limit,
            Err(message) => {
                warn!("{}", message);
                DEFAULT_MAX_RESULT_VALUES
            }
        };
        info!("query results are limited to {} values", limit);
        limit
    })
}

/// The limit a value of [`MAX_RESULT_VALUES_ENV`] sets; the default when it is unset, and an
/// error to warn about when it is not a positive integer.
fn parse_max_result_values(value: Option<&str>) -> Result<u64, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_MAX_RESULT_VALUES);
    };
    match value.trim().parse::<u64>() {
        Ok(limit) if limit > 0 => Ok(limit),
        _ => Err(format!(
            "{} must be a positive integer, got {:?}; using the default of {} values",
            MAX_RESULT_VALUES_ENV, value, DEFAULT_MAX_RESULT_VALUES
        )),
    }
}

/// Run a BQL-compatible query over the ledger.
///
/// With `count_total` the result also has the number of rows before `LIMIT` and `OFFSET`,
/// `total`. Query errors are answered with HTTP 400 and `{"message", "line", "column"}`.
#[api(group = "query")]
pub async fn run_query(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> QueryApiResult<QueryResultEntity> {
    let count_total = payload.count_total.unwrap_or(false);
    QueryApiResult(run(&ledger, payload.query, max_result_values(), count_total).await)
}

async fn run(ledger: &LedgerState, text: String, max_result_values: u64, count_total: bool) -> ServerResult<ResponseWrapper<QueryResultEntity>> {
    let result = execute(ledger, text, max_result_values, count_total).await?;
    // converting the cells is CPU-bound too, so it stays off the async workers
    let entity = tokio::task::spawn_blocking(move || QueryResultEntity::from(result)).await?;
    ResponseWrapper::json(entity)
}

/// Run a BQL-compatible query and download the result as CSV (`query.csv`).
///
/// Amount, position and inventory columns are split into one numeric column per currency,
/// named like `balance (USD)`, as beanquery's numberify does. Query errors are answered
/// with HTTP 400 and `{"message", "line", "column"}`, as in `POST /api/query`.
#[api(group = "query")]
pub async fn run_query_csv(ledger: State<SharedLedger>, Json(payload): Json<QueryRequest>) -> QueryCsvResult {
    QueryCsvResult(export_csv(&ledger, payload.query).await)
}

async fn export_csv(ledger: &LedgerState, text: String) -> ServerResult<String> {
    let result = execute(ledger, text, max_result_values(), false).await?;
    // the read lock is released by now; rendering is CPU-bound, so it stays off the async workers
    Ok(tokio::task::spawn_blocking(move || zhang_query::export::to_csv(&result)).await?)
}

/// The limits of every query the server runs: [`QUERY_TIMEOUT`] and a result of at most
/// `max_result_values` values. Router plugins' `zhang_query` calls get them too.
pub(crate) fn execute_options(max_result_values: u64) -> ExecuteOptions {
    ExecuteOptions {
        today: None,
        timeout: Some(QUERY_TIMEOUT),
        max_result_values: Some(max_result_values),
        count_total: false,
    }
}

/// Run `work` on the ledger off the async workers, under its read lock: the path of every
/// query the server runs, `POST /api/query` and the built-in queries alike. Queries in `work`
/// run with [`execute_options`], whose time limit bounds how long the lock is held.
pub async fn with_ledger<T: Send + 'static>(ledger: &LedgerState, work: impl FnOnce(&Ledger) -> ServerResult<T> + Send + 'static) -> ServerResult<T> {
    // an owned guard moves into the blocking task
    let ledger = ledger.clone().read_owned().await;
    tokio::task::spawn_blocking(move || work(&ledger)).await?
}

/// Compile and run a query off the async workers, under the ledger read lock, the time
/// limit and the result size limit. The query length is capped by the parser.
async fn execute(ledger: &LedgerState, text: String, max_result_values: u64, count_total: bool) -> ServerResult<QueryResult> {
    // compiling needs no ledger; run it, and the CPU-bound execution, off the async workers
    let query = tokio::task::spawn_blocking(move || Query::compile(&text)).await??;
    let options = ExecuteOptions {
        count_total,
        ..execute_options(max_result_values)
    };
    with_ledger(ledger, move |ledger| Ok(query.execute_with_options(ledger, &Params::new(), &options)?)).await
}

/// The built-in queries: the named BQL behind the figures the app shows, which the Query page
/// (`/explore`) can open and a user adapt. See the "Built-in queries" page of the docs.
#[api(group = "query")]
pub async fn get_builtin_queries() -> ApiResult<Vec<BuiltinQueryEntity>> {
    ResponseWrapper::json(BUILTINS.iter().map(BuiltinQueryEntity::from).collect())
}

/// A built-in query with its parameters written in as BQL literals, to open it on the Query page
/// (`/explore`), where parameters cannot be bound: it runs there to the same result as in the app.
///
/// `params` gives every parameter of the query by name (see `GET /api/query/builtins` for
/// their types): a boolean for `bool`, an integer for `int`, a number or a string such as
/// `"12.50"` for `decimal`, a string for `str`, a string `YYYY-MM-DD` for `date`, a list of
/// strings for `set`, and `null` for NULL. An unknown query is a 404; a missing, unknown or
/// mistyped parameter a 400.
#[api(group = "query")]
pub async fn get_builtin_query_text(paths: Path<(String,)>, Json(payload): Json<BuiltinQueryTextRequest>) -> ApiResult<BuiltinQueryTextEntity> {
    let builtin = lookup(&paths.0 .0)?;
    let params = builtin::json_params(builtin, payload.params)?;
    ResponseWrapper::json(BuiltinQueryTextEntity {
        query: builtin::text(builtin, &params)?,
    })
}

/// The built-in query a request names, or the 404 that names it.
fn lookup(name: &str) -> ServerResult<&'static builtin::BuiltinQuery> {
    builtin::get(name).ok_or_else(|| ServerError::NoSuchBuiltinQuery(name.to_owned()))
}

/// Run a built-in query with its parameters bound: the rows the app computes its figures from,
/// with the columns `GET /api/query/builtins` lists for the query, as `POST /api/query` returns
/// a result.
///
/// `params` gives every parameter by name, as `POST /api/query/builtins/{name}/text` takes
/// them; with `count_total` the result also has `total`, the number of rows before `LIMIT`
/// and `OFFSET`. An unknown query is a 404; a missing, unknown or mistyped parameter a 400,
/// each naming the query or the parameter.
#[api(group = "query")]
pub async fn run_builtin_query(
    ledger: State<SharedLedger>, paths: Path<(String,)>, Json(payload): Json<BuiltinQueryRunRequest>,
) -> QueryApiResult<QueryResultEntity> {
    QueryApiResult(run_builtin(&ledger, &paths.0 .0, payload).await)
}

async fn run_builtin(ledger: &LedgerState, name: &str, payload: BuiltinQueryRunRequest) -> ServerResult<ResponseWrapper<QueryResultEntity>> {
    let builtin = lookup(name)?;
    let params = builtin::json_params(builtin, payload.params)?;
    let count_total = payload.count_total.unwrap_or(false);
    let result = with_ledger(ledger, move |ledger| builtin::execute(ledger, builtin.name, &params, count_total)).await?;
    ResponseWrapper::json(tokio::task::spawn_blocking(move || QueryResultEntity::from(result)).await?)
}

/// Run several built-in queries, each as `POST /api/query/builtins/{name}` runs it, under one
/// read lock of the ledger, so the figures of one page agree with each other: the results in
/// the order of the requests. The whole batch is refused, with the same 404 or 400, when one
/// of its queries would be.
#[api(group = "query")]
pub async fn run_builtin_queries(ledger: State<SharedLedger>, Json(payload): Json<Vec<BuiltinQueryBatchItem>>) -> QueryApiResult<Vec<QueryResultEntity>> {
    QueryApiResult(run_builtins(&ledger, payload).await)
}

async fn run_builtins(ledger: &LedgerState, payload: Vec<BuiltinQueryBatchItem>) -> ServerResult<ResponseWrapper<Vec<QueryResultEntity>>> {
    // every name and parameter is checked before the lock is taken
    let runs = payload
        .into_iter()
        .map(|item| {
            let builtin = lookup(&item.name)?;
            Ok((builtin.name, builtin::json_params(builtin, item.params)?, item.count_total.unwrap_or(false)))
        })
        .collect::<ServerResult<Vec<_>>>()?;
    let results = with_ledger(ledger, move |ledger| {
        runs.into_iter()
            .map(|(name, params, count_total)| builtin::execute(ledger, name, &params, count_total))
            .collect::<ServerResult<Vec<_>>>()
    })
    .await?;
    // converting the cells is CPU-bound too, so it stays off the async workers, and off the lock
    ResponseWrapper::json(tokio::task::spawn_blocking(move || results.into_iter().map(QueryResultEntity::from).collect()).await?)
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
    let queries = ledger.read().await.queries();
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
    use zhang_core::clock::Clock;
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
            dialect: zhang_core::data_type::Dialect::Zhang,
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            clock: Clock::System,
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
        let ledger = Ledger::load(dir.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        // queries only read the in-memory store
        std::fs::remove_dir_all(dir).ok();
        SharedLedger(Arc::new(RwLock::new(ledger)))
    }

    async fn post_csv(query: &str) -> Response {
        let request = QueryRequest {
            query: query.to_owned(),
            count_total: None,
        };
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
    async fn query_csv_of_a_pivot_splits_the_pivoted_columns_per_currency() {
        let response = post_csv("SELECT account, currency, sum(position) AS total GROUP BY 1, 2 PIVOT BY account, currency").await;
        assert_eq!(response.status(), StatusCode::OK);
        // a pivoted inventory column is split like any other; missing cells stay empty
        assert_eq!(
            text(response).await,
            "account/currency,AAPL (AAPL),CNY (CNY),USD (USD)\r\n\
             Assets:Broker,2,,\r\n\
             Assets:Cash,,-12.50,\r\n\
             Equity:Opening,,,-300.00\r\n\
             Expenses:Food,,12.50,\r\n"
        );
    }

    #[tokio::test]
    async fn query_results_of_a_pivot_have_typed_columns() {
        let ledger = ledger().await;
        let response = super::run(
            &ledger.0,
            "SELECT account, currency, count(*) AS n GROUP BY 1, 2 HAVING count(*) > 0 PIVOT BY currency, account".to_owned(),
            zhang_query::DEFAULT_MAX_RESULT_VALUES,
            false,
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
        assert_eq!(
            body,
            json!({"data": {
                "columns": [
                    {"name": "currency/account", "type": "str"},
                    {"name": "Assets:Broker", "type": "int"},
                    {"name": "Assets:Cash", "type": "int"},
                    {"name": "Equity:Opening", "type": "int"},
                    {"name": "Expenses:Food", "type": "int"}
                ],
                "rows": [["AAPL", 1, null, null, null], ["CNY", null, 1, null, 1], ["USD", null, null, 1, null]]
            }})
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

#[cfg(test)]
mod result_limit_test {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;
    use zhang_query::DEFAULT_MAX_RESULT_VALUES;

    use super::{parse_max_result_values, run};

    async fn ledger() -> Arc<RwLock<Ledger>> {
        let mut content = String::from("1970-01-01 open Assets:Broker\n1970-01-01 open Assets:Cash\n");
        for day in 1..=28 {
            content.push_str(&format!(
                "\n2024-02-{day:02} * \"buy\"\n  Assets:Broker 1 STK {{{day}.00 USD}}\n  Assets:Cash -{day}.00 USD\n"
            ));
        }
        let dir = std::env::temp_dir().join(format!("zhang-query-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(dir.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        std::fs::remove_dir_all(dir).ok();
        Arc::new(RwLock::new(ledger))
    }

    /// A name with thousands of dots is a positioned 400, on a runtime whose threads have a
    /// 2 MiB stack.
    #[test]
    fn long_dotted_names_are_a_query_400() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(2 << 20)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let ledger = ledger().await;
            for dots in [10_000, 32_000] {
                let query = format!("SELECT a{} FROM #accounts", ".a".repeat(dots));
                let response = run(&ledger, query, 100, false).await.into_response();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["line"], 1);
                assert_eq!(body["column"], 8 + 2 * zhang_query::MAX_NAME_PARTS - 1);
            }
        });
    }

    #[tokio::test]
    async fn results_over_the_limit_are_a_query_400() {
        let ledger = ledger().await;
        let response = run(&ledger, "JOURNAL".to_owned(), 100, false).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({
                "message": "the result is too large: it would hold more than 100 values (cells, plus the positions of inventories); \
                            narrow the query with FROM or WHERE, or add a LIMIT",
                "line": null,
                "column": null,
            })
        );

        // the default limit, which this result is far below
        let response = run(&ledger, "JOURNAL".to_owned(), DEFAULT_MAX_RESULT_VALUES, false).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["data"]["rows"].as_array().unwrap().len(), 56);
    }

    #[test]
    fn the_limit_comes_from_a_positive_integer() {
        assert_eq!(parse_max_result_values(None), Ok(DEFAULT_MAX_RESULT_VALUES));
        assert_eq!(parse_max_result_values(Some("250000")), Ok(250_000));
        assert_eq!(parse_max_result_values(Some(" 5000\n")), Ok(5_000));
        for invalid in ["", "0", "-1", "1.5", "1e6", "a lot", "18446744073709551616"] {
            let message = parse_max_result_values(Some(invalid)).unwrap_err();
            assert!(message.starts_with("ZHANG_QUERY_MAX_RESULT_VALUES must be a positive integer"), "{}", message);
            assert!(message.ends_with("using the default of 1000000 values"), "{}", message);
        }
    }
}

#[cfg(test)]
mod schema_test {
    use axum::response::IntoResponse;
    use gotcha::Schematic;

    use super::get_query_schema;
    use crate::response::QuerySchemaEntity;

    #[tokio::test]
    async fn the_schema_lists_every_table_with_its_columns() {
        let response = get_query_schema().await.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let schema = &body["data"];
        assert!(!schema["functions"].as_array().unwrap().is_empty());

        let tables = schema["tables"].as_array().unwrap();
        // postings comes first, with the same columns as the top-level `columns`
        assert_eq!(tables[0]["name"], "postings");
        assert_eq!(tables[0]["columns"], schema["columns"]);
        assert!(tables.iter().any(|table| table["name"] == "prices"));
        for table in tables {
            let name = table["name"].as_str().unwrap();
            assert!(!name.starts_with('#') && !name.is_empty(), "{}", name);
            assert!(!table["description"].as_str().unwrap().is_empty(), "{}", name);
            let columns = table["columns"].as_array().unwrap();
            assert!(!columns.is_empty(), "{}", name);
            for column in columns {
                assert_eq!(column.as_object().unwrap().len(), 3, "{}: {}", name, column);
                for field in ["name", "type", "description"] {
                    assert!(!column[field].as_str().unwrap().is_empty(), "{}: {}", name, column);
                }
            }
        }
    }

    #[test]
    fn the_openapi_schema_declares_the_tables() {
        let schema = serde_json::to_value(QuerySchemaEntity::generate_schema().schema).unwrap();
        assert_eq!(schema["required"], serde_json::json!(["columns", "functions", "tables", "keywords"]));
        let table = &schema["properties"]["tables"]["items"];
        assert_eq!(table["required"], serde_json::json!(["name", "description", "columns"]));
        assert_eq!(table["properties"]["columns"]["type"], "array");
    }
}

#[cfg(test)]
mod builtin_test {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use chrono::NaiveDate;
    use serde_json::json;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;
    use zhang_query::{Params, Value};

    use super::{get_builtin_queries, get_builtin_query_text, run_builtin_queries, run_builtin_query, run_query};
    use crate::builtin::{self, BUILTINS};
    use crate::request::{BuiltinParamValue, BuiltinQueryBatchItem, BuiltinQueryRunRequest, BuiltinQueryTextRequest, QueryRequest};
    use crate::response::QueryResultEntity;
    use crate::state::SharedLedger;

    const LEDGER: &str = r#"
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel

2024-01-31 * "O'Brien" "Lunch" #food
  Expenses:Food 12.50 USD
  Assets:Bank -12.50 USD

2024-02-01 * "it's \"quoted\"" "both kinds of quotes" #trip #it's
  Expenses:Travel 100 USD
  Assets:Bank -100 USD

2024-02-29 * "C:\\temp\\" "a backslash" #trip
  Expenses:Travel 7 USD
  Assets:Bank -7 USD
"#;

    async fn ledger() -> SharedLedger {
        let dir = std::env::temp_dir().join(format!("zhang-builtin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zhang"), LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(dir.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        std::fs::remove_dir_all(dir).ok();
        SharedLedger(Arc::new(RwLock::new(ledger)))
    }

    async fn body(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn text_of(name: &str, params: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let params: HashMap<String, Option<BuiltinParamValue>> = serde_json::from_value(params).unwrap();
        body(
            get_builtin_query_text(Path((name.to_owned(),)), Json(BuiltinQueryTextRequest { params }))
                .await
                .into_response(),
        )
        .await
    }

    async fn query(ledger: &SharedLedger, query: &str, count_total: Option<bool>) -> (StatusCode, serde_json::Value) {
        let request = QueryRequest {
            query: query.to_owned(),
            count_total,
        };
        body(run_query(State(ledger.clone()), Json(request)).await.into_response()).await
    }

    /// `POST /api/query/builtins/{name}`
    async fn run_of(ledger: &SharedLedger, name: &str, params: serde_json::Value, count_total: Option<bool>) -> (StatusCode, serde_json::Value) {
        let params: HashMap<String, Option<BuiltinParamValue>> = serde_json::from_value(params).unwrap();
        let request = BuiltinQueryRunRequest { params, count_total };
        body(
            run_builtin_query(State(ledger.clone()), Path((name.to_owned(),)), Json(request))
                .await
                .into_response(),
        )
        .await
    }

    /// `POST /api/query/builtins` with `[{name, params, count_total}]`
    async fn batch_of(ledger: &SharedLedger, items: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let items: Vec<BuiltinQueryBatchItem> = serde_json::from_value(items).unwrap();
        body(run_builtin_queries(State(ledger.clone()), Json(items)).await.into_response()).await
    }

    #[tokio::test]
    async fn builtins_are_listed_with_their_bql_typed_params_and_columns() {
        let (status, body) = body(get_builtin_queries().await.into_response()).await;
        assert_eq!(status, StatusCode::OK);
        let listed = body["data"].as_array().unwrap();
        assert_eq!(listed.len(), BUILTINS.len());
        let between = listed.iter().find(|it| it["name"] == "postings.between").unwrap();
        assert_eq!(
            between,
            &json!({
                "name": "postings.between",
                "description": builtin::get("postings.between").unwrap().description,
                "bql": builtin::get("postings.between").unwrap().bql,
                "params": [{"name": "from", "type": "date"}, {"name": "to", "type": "date"}],
                "columns": [
                    {"name": "date", "type": "date"},
                    {"name": "flag", "type": "str"},
                    {"name": "payee", "type": "str"},
                    {"name": "narration", "type": "str"},
                    {"name": "account", "type": "str"},
                    {"name": "position", "type": "position"},
                ],
            })
        );
        let matching = listed.iter().find(|it| it["name"] == "postings.matching").unwrap();
        assert_eq!(matching["params"], json!([{"name": "payee", "type": "str"}, {"name": "tags", "type": "set"}]));
        // every query has columns, and they are those of its result
        let ledger = ledger().await;
        for builtin in listed {
            let name = builtin["name"].as_str().unwrap();
            assert!(!builtin["columns"].as_array().unwrap().is_empty(), "{}", name);
            let result = crate::builtin::compiled(name).unwrap().columns();
            let expected = result.iter().map(|c| json!({"name": c.name, "type": c.ty.to_string()})).collect::<Vec<_>>();
            assert_eq!(builtin["columns"], json!(expected), "{}", name);
        }
        // the listed columns are those a run returns
        let (status, run) = run_of(&ledger, "postings.between", json!({"from": "2024-01-01", "to": "2024-12-31"}), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(run["data"]["columns"], between["columns"]);
    }

    /// `POST /api/query/builtins/{name}` returns what the app gets by binding the parameters:
    /// the result of `POST /api/query` for the query's text, and its `total` on request.
    #[tokio::test]
    async fn a_builtin_runs_by_name_to_the_bound_result() {
        let ledger = ledger().await;
        let cases = [
            ("postings.between", json!({"from": "2024-02-01", "to": "2024-02-29"}), 4),
            ("postings.matching", json!({"payee": "it's \"quoted\"", "tags": ["it's", "trip"]}), 2),
            ("postings.matching", json!({"payee": null, "tags": []}), 0),
            ("postings.matching", json!({"payee": null, "tags": null}), 6),
        ];
        for (name, params, rows) in cases {
            let (status, run) = run_of(&ledger, name, params.clone(), None).await;
            assert_eq!(status, StatusCode::OK, "{} {}: {}", name, params, run);
            assert_eq!(run["data"]["rows"].as_array().unwrap().len(), rows, "{} {}", name, params);
            assert_eq!(run["data"].as_object().unwrap().keys().collect::<Vec<_>>(), ["columns", "rows"]);

            let (_, written) = text_of(name, params.clone()).await;
            let (_, inlined) = query(&ledger, written["data"]["query"].as_str().unwrap(), None).await;
            assert_eq!(run["data"], inlined["data"], "{} {}", name, params);

            let (status, counted) = run_of(&ledger, name, params.clone(), Some(true)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(counted["data"]["total"], rows, "{} {}", name, params);
            assert_eq!(counted["data"]["rows"], run["data"]["rows"]);
        }
    }

    /// `POST /api/query/builtins` runs each query as the single endpoint does, in the order of
    /// the requests, each with its own `count_total`.
    #[tokio::test]
    async fn a_batch_of_builtins_runs_each_in_order() {
        let ledger = ledger().await;
        let (status, batch) = batch_of(
            &ledger,
            json!([
                {"name": "postings.between", "params": {"from": "2024-02-01", "to": "2024-02-29"}},
                {"name": "postings.matching", "params": {"payee": null, "tags": ["trip"]}, "count_total": true},
                {"name": "journals.payees", "params": {}, "count_total": false},
            ]),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", batch);
        let results = batch["data"].as_array().unwrap();
        assert_eq!(results.len(), 3);

        let (_, between) = run_of(&ledger, "postings.between", json!({"from": "2024-02-01", "to": "2024-02-29"}), None).await;
        assert_eq!(results[0], between["data"]);
        let (_, matching) = run_of(&ledger, "postings.matching", json!({"payee": null, "tags": ["trip"]}), Some(true)).await;
        assert_eq!(results[1], matching["data"]);
        assert_eq!(results[1]["total"], 4);
        let (_, payees) = run_of(&ledger, "journals.payees", json!({}), None).await;
        assert_eq!(results[2], payees["data"]);
        assert_eq!(results[2]["rows"], json!([["C:\\temp\\"], ["O'Brien"], ["it's \"quoted\""]]));

        let (status, empty) = batch_of(&ledger, json!([])).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(empty, json!({"data": []}));
    }

    /// An unknown query is a 404 and a bad parameter a 400, each naming the problem; a batch
    /// with one such query is refused as a whole.
    #[tokio::test]
    async fn running_an_unknown_builtin_or_bad_params_is_a_named_error() {
        let ledger = ledger().await;
        let (status, body) = run_of(&ledger, "no.such.query", json!({}), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["message"], "there is no built-in query no.such.query");
        let bad_params = [
            (json!({"from": "2024-02-01"}), "parameter :to of postings.between is missing"),
            (
                json!({"from": "2024-02-01", "to": "2024-02-29", "account": "x"}),
                "postings.between has no parameter :account",
            ),
            (
                json!({"from": "2024-02-01", "to": 20240229}),
                "parameter :to must be a date string YYYY-MM-DD or null",
            ),
        ];
        for (params, message) in &bad_params {
            let (status, body) = run_of(&ledger, "postings.between", params.clone(), None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["message"], *message);
        }

        let good = json!({"name": "postings.matching", "params": {"payee": null, "tags": null}});
        let (status, body) = batch_of(&ledger, json!([good, {"name": "no.such.query", "params": {}}])).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["message"], "there is no built-in query no.such.query");
        for (params, message) in &bad_params {
            let (status, body) = batch_of(&ledger, json!([good, {"name": "postings.between", "params": params}])).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["message"], *message);
        }
    }

    /// The text of a built-in query runs over `POST /api/query` to the result the app gets
    /// by binding the parameters.
    #[tokio::test]
    async fn builtin_text_runs_to_the_bound_result() {
        let ledger = ledger().await;
        let cases = [
            (
                "postings.between",
                json!({"from": "2024-02-01", "to": "2024-02-29"}),
                Params::new()
                    .bind("from", NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
                    .bind("to", NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()),
                4,
            ),
            (
                "postings.matching",
                json!({"payee": "it's \"quoted\"", "tags": ["it's", "trip"]}),
                Params::new()
                    .bind("payee", "it's \"quoted\"")
                    .bind("tags", Value::Set(["it's".to_owned(), "trip".to_owned()].into())),
                2,
            ),
            (
                "postings.matching",
                json!({"payee": "C:\\temp\\", "tags": null}),
                Params::new().bind("payee", "C:\\temp\\").bind("tags", Value::Null),
                2,
            ),
            (
                "postings.matching",
                json!({"payee": null, "tags": []}),
                Params::new().bind("payee", Value::Null).bind("tags", Value::Set(Default::default())),
                0,
            ),
            (
                "postings.matching",
                json!({"payee": null, "tags": null}),
                Params::new().bind("payee", Value::Null).bind("tags", Value::Null),
                6,
            ),
        ];
        for (name, json_params, params, rows) in cases {
            let (status, written) = text_of(name, json_params.clone()).await;
            assert_eq!(status, StatusCode::OK, "{}: {}", name, written);
            let text = written["data"]["query"].as_str().unwrap();
            for param in builtin::get(name).unwrap().params {
                assert!(!text.contains(&format!(":{}", param.0)), "{}", text);
            }

            let bound = builtin::run(&ledger, name, params).await.unwrap();
            assert_eq!(bound.rows.len(), rows, "{} {}", name, json_params);
            let bound = serde_json::to_value(QueryResultEntity::from(bound)).unwrap();
            let (status, inlined) = query(&ledger, text, None).await;
            assert_eq!(status, StatusCode::OK, "{}: {}", text, inlined);
            assert_eq!(inlined["data"], bound, "{}", text);
        }
    }

    #[tokio::test]
    async fn builtin_text_of_an_unknown_query_or_bad_params_is_an_error() {
        let (status, body) = text_of("no.such.query", json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["message"], "there is no built-in query no.such.query");
        for (params, message) in [
            (json!({"from": "2024-02-01"}), "parameter :to of postings.between is missing"),
            (
                json!({"from": "2024-02-01", "to": "2024-02-29", "account": "x"}),
                "postings.between has no parameter :account",
            ),
            (
                json!({"from": "2024-02-01", "to": 20240229}),
                "parameter :to must be a date string YYYY-MM-DD or null",
            ),
        ] {
            let (status, body) = text_of("postings.between", params).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["message"], message);
        }
    }

    #[tokio::test]
    async fn builtins_run_like_api_queries() {
        let ledger = ledger().await;
        let params = || Params::new().bind("payee", Value::Null).bind("tags", Value::Set(["trip".to_owned()].into()));
        let result = builtin::run(&ledger, "postings.matching", params()).await.unwrap();
        assert_eq!((result.rows.len(), result.total), (4, None));
        let result = builtin::run_with_total(&ledger, "postings.matching", params()).await.unwrap();
        assert_eq!((result.rows.len(), result.total), (4, Some(4)));

        // several under one read lock
        let (first, second) = super::with_ledger(&ledger, move |ledger| {
            let first = builtin::execute(ledger, "postings.matching", &params(), false)?;
            let second = builtin::execute(ledger, "postings.matching", &params(), true)?;
            Ok((first, second))
        })
        .await
        .unwrap();
        assert_eq!((first.rows, first.total), (second.rows, None));

        // a parameter of another type, or a query that does not exist, is an error
        let error = builtin::run(&ledger, "postings.matching", Params::new().bind("payee", 1i64).bind("tags", Value::Null))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "query error: parameter :payee was compiled as str but is bound to a int (line 1, column 63)"
        );
        let error = builtin::run(&ledger, "no.such.query", Params::new()).await.unwrap_err();
        assert_eq!(error.into_response().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn api_queries_count_their_total_on_request() {
        let ledger = ledger().await;
        let (status, body) = query(&ledger, "SELECT date, account ORDER BY seq LIMIT 2 OFFSET 1", Some(true)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["rows"].as_array().unwrap().len(), 2);
        assert_eq!(body["data"]["total"], 6);
        // without it, as before, there is no total
        for count_total in [None, Some(false)] {
            let (_, body) = query(&ledger, "SELECT date, account ORDER BY seq LIMIT 2 OFFSET 1", count_total).await;
            assert_eq!(body["data"].as_object().unwrap().keys().collect::<Vec<_>>(), ["columns", "rows"]);
        }
    }
}
