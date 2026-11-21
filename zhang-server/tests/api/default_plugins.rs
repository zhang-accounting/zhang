//! The official default plugin `zhang:budget` through the server (RFC #698 §6, P1.5): with plugins on (`ZHANG_PLUGINS`
//! `read`, the default, or `write`) the budget pages work as they always have; with `ZHANG_PLUGINS=off`, which stops
//! every plugin, `zhang:budget` included, the budget tables and the budget pages' queries answer that they are
//! unavailable, saying why, the ledger has one `ExtensionUnavailable` error, and everything else reads as before.

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::{PluginLevel, PluginPolicy};
use zhang_server::routes::plugin_router::PluginSettings;
use zhang_testkit::http::{app, call, router_of, serve_config, Settings};
use zhang_testkit::ledger::Scratch;

const LEDGER: &str = "option \"timezone\" \"UTC\"
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: \"Food\"

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 100 CNY
2024-01-05 * \"Cafe\" \"lunch\"
  Expenses:Food 30 CNY
  Assets:Bank -30 CNY
";

/// The router of a server on `LEDGER` with plugins at `level`, loaded as `zhang serve` loads a ledger.
async fn server(scratch: &Scratch, level: PluginLevel) -> Router {
    let policy = PluginPolicy {
        level,
        ..PluginPolicy::default()
    };
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let options = zhang_server::analysis::load_options(Arc::new(policy.clone()));
    let ledger = Ledger::load_with_options(scratch.dir().to_path_buf(), "main.zhang".to_owned(), source.clone(), Clock::System, options)
        .unwrap_or_else(|e| panic!("the ledger loads: {e}"));
    let settings = Settings {
        plugins: PluginSettings {
            policy,
            ..PluginSettings::default()
        },
        ..Settings::default()
    };
    router_of(app(serve_config(scratch.dir(), "main.zhang", source, &settings), Arc::new(RwLock::new(ledger)))).await
}

/// What the budget pages and a client of the budget tables send.
fn budget_requests() -> Vec<(Method, &'static str, Value)> {
    let month = json!({"name": "Food", "month": "2024-01-01"});
    vec![
        (Method::POST, "/api/query/builtins/budgets.month", json!({"params": {"month": "2024-01-01"}})),
        (Method::POST, "/api/query/builtins/budgets.budget", json!({"params": {"name": "Food"}})),
        (Method::POST, "/api/query/builtins/budgets.budget_month", json!({"params": month})),
        (Method::POST, "/api/query/builtins/budgets.events", json!({"params": month})),
        (Method::POST, "/api/query/builtins/budgets.postings", json!({"params": month})),
        (
            Method::POST,
            "/api/query/builtins",
            json!([{"name": "budgets.budget", "params": {"name": "Food"}}, {"name": "budgets.postings", "params": month}]),
        ),
        (
            Method::POST,
            "/api/query",
            json!({"query": "SELECT name, available FROM #budgets WHERE date = 2024-01-01"}),
        ),
        (Method::POST, "/api/query", json!({"query": "SELECT account, budgets FROM #postings"})),
    ]
}

/// What does not read budgets: a built-in query of the journals, the built-in queries' list, BQL over postings.
fn other_requests() -> Vec<(Method, &'static str, Value)> {
    vec![
        (
            Method::POST,
            "/api/query/builtins/postings.between",
            json!({"params": {"from": "2024-01-01", "to": "2024-12-31"}}),
        ),
        (Method::GET, "/api/query/builtins", Value::Null),
        (
            Method::POST,
            "/api/query",
            json!({"query": "SELECT account, sum(number) GROUP BY account ORDER BY account"}),
        ),
    ]
}

async fn answer(router: &Router, (method, uri, body): &(Method, &str, Value)) -> (StatusCode, Value) {
    let body = (!body.is_null()).then_some(body);
    let reply = call(router, method.clone(), uri, &[], body).await;
    (reply.status, reply.body)
}

/// The ledger's errors, as the errors page lists them: kind and metas.
async fn errors(router: &Router) -> Value {
    let request = (
        Method::POST,
        "/api/query/builtins/journals.errors",
        json!({"params": {"size": 100, "offset": 0}}),
    );
    let (status, body) = answer(router, &request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"]["rows"].as_array().unwrap();
    Value::Array(rows.iter().map(|row| json!([row[1], row[3], row[8]])).collect())
}

#[tokio::test]
async fn budgets_answer_alike_whether_plugins_read_or_write() {
    let scratch = Scratch::zhang(LEDGER);
    let read = server(&scratch, PluginLevel::Read).await;
    let write = server(&scratch, PluginLevel::Write).await;
    for request in budget_requests().iter().chain(&other_requests()) {
        let (status, body) = answer(&read, request).await;
        assert_eq!(status, StatusCode::OK, "{} {}: {body}", request.0, request.1);
        assert_eq!(answer(&write, request).await, (status, body), "{} {}", request.0, request.1);
    }
    let (_, month) = answer(&read, &budget_requests()[0]).await;
    assert_eq!(month["data"]["rows"][0][0], json!("Food"));
    assert_eq!(month["data"]["rows"][0][7], json!({"number": "70", "currency": "CNY"}), "{month}");
    assert_eq!(errors(&read).await, json!([]));
}

#[tokio::test]
async fn plugins_off_make_the_budget_tables_and_queries_unavailable_and_nothing_else() {
    let scratch = Scratch::zhang(LEDGER);
    let off = server(&scratch, PluginLevel::Off).await;
    let read = server(&scratch, PluginLevel::Read).await;
    let why = "plugin zhang:budget does not run: plugins are off (ZHANG_PLUGINS=off)";
    let expected = [
        format!("table #budgets is unavailable: {why}"),
        format!("table #budget_definitions is unavailable: {why}"),
        format!("table #budgets is unavailable: {why}"),
        format!("table #budget_events is unavailable: {why}"),
        format!("column postings.budgets is unavailable: {why}"),
        format!("table #budget_definitions is unavailable: {why}"),
        format!("table #budgets is unavailable: {why}"),
        format!("column postings.budgets is unavailable: {why}"),
    ];
    for (request, message) in budget_requests().iter().zip(&expected) {
        let (status, body) = answer(&off, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{} {}: {body}", request.0, request.1);
        assert_eq!(body["message"], json!(message), "{} {}", request.0, request.1);
    }
    // the rest of the ledger reads as with plugins on, the budget pages' queries listed with zhang's own
    for request in other_requests() {
        let (status, body) = answer(&off, &request).await;
        assert_eq!(status, StatusCode::OK, "{} {}: {body}", request.0, request.1);
        assert_eq!(answer(&read, &request).await, (status, body), "{} {}", request.0, request.1);
    }
    // one error says why, on the first directive that uses budgets
    assert_eq!(
        errors(&off).await,
        json!([[
            "ExtensionUnavailable",
            3,
            [{"key": "plugin", "value": "zhang:budget"}, {"key": "reason", "value": "plugins are off (ZHANG_PLUGINS=off)"}]
        ]])
    );
}
