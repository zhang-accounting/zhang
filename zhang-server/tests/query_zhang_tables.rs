//! The `#budgets` and `#errors` query tables against the APIs the UI reads, for the same
//! ledgers: every row of `#budgets` has the amounts `GET /api/budgets?year=&month=` returns for
//! its budget and month, and its accounts are the related accounts of `GET /api/budgets/{name}`;
//! `#errors` has one row per error of `GET /api/errors`, with its type, file and directive.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, Query as UrlQuery, State};
use axum::response::IntoResponse;
use axum::Json;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::{BudgetListRequest, JournalRequest, QueryRequest};
use zhang_server::routes::budget::{get_budget_info, get_budget_list};
use zhang_server::routes::common::get_errors;
use zhang_server::routes::query::{get_query_schema, run_query};
use zhang_server::state::SharedLedger;

fn fixture_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests").join(name)
}

/// The fixture, loaded the way the server loads a ledger.
async fn load(name: &str) -> SharedLedger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::async_load(fixture_dir(name), "main.zhang".to_owned(), source)
        .await
        .unwrap_or_else(|error| panic!("{name} should load: {error}"));
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

async fn body(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The rows of `POST /api/query`, as objects keyed by column name.
async fn query(ledger: &SharedLedger, sql: &str) -> Vec<serde_json::Map<String, Value>> {
    let response = body(run_query(State(ledger.clone()), Json(QueryRequest { query: sql.to_owned() })).await).await;
    let columns = response["data"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    response["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| columns.iter().cloned().zip(row.as_array().unwrap().iter().cloned()).collect())
        .collect()
}

/// An amount of the query API (`{"number", "currency"}`) or of the budget API
/// (`{"number", "commodity"}`), as a comparable pair.
fn amount(value: &Value) -> (BigDecimal, String) {
    let currency = value.get("currency").or_else(|| value.get("commodity")).and_then(Value::as_str).unwrap();
    (BigDecimal::from_str(value["number"].as_str().unwrap()).unwrap(), currency.to_owned())
}

async fn check_budgets(name: &str) -> usize {
    let ledger = load(name).await;
    let rows = query(
        &ledger,
        "SELECT name, alias, category, year, month, assigned, activity, available, accounts, closed FROM #budgets",
    )
    .await;
    assert!(!rows.is_empty(), "{name}");
    let months = rows
        .iter()
        .map(|row| (row["year"].as_u64().unwrap() as u32, row["month"].as_u64().unwrap() as u32))
        .collect::<BTreeSet<_>>();
    let last_month = *months.last().unwrap();
    for (year, month) in months {
        let request = BudgetListRequest {
            year: Some(year),
            month: Some(month),
        };
        let listed = body(get_budget_list(State(ledger.clone()), UrlQuery(request)).await).await["data"]
            .as_array()
            .unwrap()
            .clone();
        let in_month = rows
            .iter()
            .filter(|row| row["year"] == json!(year) && row["month"] == json!(month))
            .collect::<Vec<_>>();
        // a month of the table lists the budgets the budget page lists for it
        let listed_names = listed.iter().map(|it| it["name"].as_str().unwrap()).collect::<BTreeSet<_>>();
        let row_names = in_month.iter().map(|it| it["name"].as_str().unwrap()).collect::<BTreeSet<_>>();
        assert_eq!(row_names, listed_names, "{name} {year}-{month}");
        for row in in_month {
            let api = listed.iter().find(|it| it["name"] == row["name"]).unwrap();
            let at = format!("{name} {} {year}-{month}", row["name"]);
            assert_eq!(row["alias"], api["alias"], "{at}");
            assert_eq!(row["category"], api["category"], "{at}");
            assert_eq!(amount(&row["assigned"]), amount(&api["assigned_amount"]), "{at}");
            assert_eq!(amount(&row["activity"]), amount(&api["activity_amount"]), "{at}");
            assert_eq!(amount(&row["available"]), amount(&api["available_amount"]), "{at}");

            let request = BudgetListRequest {
                year: Some(year),
                month: Some(month),
            };
            let budget_name = row["name"].as_str().unwrap().to_owned();
            let info = body(get_budget_info(State(ledger.clone()), UrlPath((budget_name,)), UrlQuery(request)).await).await;
            let mut related = info["data"]["related_accounts"].as_array().unwrap().clone();
            related.sort_by_key(|it| it.as_str().unwrap().to_owned());
            assert_eq!(row["accounts"], Value::Array(related), "{at}");
            // the API's flag says whether the budget is closed now; the table's whether it was in
            // that month, so the two agree on the last month
            if (year, month) == last_month || row["closed"] == json!(true) {
                assert_eq!(row["closed"], info["data"]["closed"], "{at}");
            }
        }
    }
    rows.len()
}

#[tokio::test]
async fn budgets_are_the_amounts_of_the_budget_api() {
    assert_eq!(check_budgets("budget-sytem-syntax-and-category").await, 10);
    assert_eq!(check_budgets("budget-sytem-syntax-and-category-multiple-file").await, 10);
    assert_eq!(check_budgets("query-zhang-tables").await, 10);
}

async fn check_errors(name: &str) -> usize {
    let ledger = load(name).await;
    let rows = query(&ledger, "SELECT kind, file, source, account, date FROM #errors").await;
    let request = JournalRequest {
        page: None,
        size: Some(1000),
        keyword: None,
        tags: None,
        links: None,
    };
    let errors = body(get_errors(State(ledger.clone()), UrlQuery(request)).await).await["data"].clone();
    assert_eq!(errors["total_count"], json!(rows.len()), "{name}");

    let root = fixture_dir(name).canonicalize().unwrap();
    let mut api = errors["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| {
            let span = &error["span"];
            let file = span["filename"]
                .as_str()
                .map(|it| Path::new(it).strip_prefix(&root).unwrap().to_string_lossy().into_owned());
            let row = json!({
                "kind": error["error_type"],
                "file": file,
                "source": span["content"].as_str().map(str::trim_end),
                "account": error["metas"]["account_name"],
            });
            (file, span["start"].as_u64(), row)
        })
        .collect::<Vec<_>>();
    // the table comes by file, then position in the file
    api.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    let table = rows
        .iter()
        .map(|row| json!({"kind": row["kind"], "file": row["file"], "source": row["source"], "account": row["account"]}))
        .collect::<Vec<_>>();
    assert_eq!(table, api.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>(), "{name}");
    rows.len()
}

#[tokio::test]
async fn errors_are_the_errors_of_the_error_api() {
    assert_eq!(check_errors("raise-error-if-posting-commodity-is-not-defined").await, 1);
    assert_eq!(check_errors("should_raise_unbalance_error_for_unbalanced_txn").await, 1);
    assert_eq!(check_errors("query-zhang-tables").await, 6);
    assert_eq!(check_errors("budget-sytem-syntax-and-category").await, 0);
}

#[tokio::test]
async fn the_schema_lists_the_zhang_tables() {
    let schema = body(get_query_schema().await).await;
    let columns = |name: &str| {
        let table = schema["data"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|it| it["name"] == name)
            .unwrap_or_else(|| panic!("no table {name}"));
        table["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| (it["name"].as_str().unwrap().to_owned(), it["type"].as_str().unwrap().to_owned()))
            .collect::<Vec<_>>()
    };
    let budgets = columns("budgets");
    assert_eq!(budgets.len(), 13);
    for (column, ty) in [
        ("name", "str"),
        ("date", "date"),
        ("assigned", "amount"),
        ("activity", "amount"),
        ("available", "amount"),
        ("accounts", "set"),
        ("closed", "bool"),
    ] {
        assert!(budgets.contains(&(column.to_owned(), ty.to_owned())), "budgets.{column}: {budgets:?}");
    }
    let errors = columns("errors");
    assert_eq!(errors.len(), 8);
    for (column, ty) in [
        ("kind", "str"),
        ("message", "str"),
        ("file", "str"),
        ("line", "int"),
        ("column", "int"),
        ("date", "date"),
        ("account", "str"),
    ] {
        assert!(errors.contains(&(column.to_owned(), ty.to_owned())), "errors.{column}: {errors:?}");
    }
}
