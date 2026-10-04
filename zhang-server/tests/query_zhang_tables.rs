//! The `#budgets` and `#errors` query tables against the APIs the UI reads, for the same
//! ledgers: every row of `#budgets` has the amounts `GET /api/budgets?year=&month=` returns for
//! its budget and month, and its accounts are the related accounts of `GET /api/budgets/{name}`;
//! `#errors` has one row per error of `GET /api/errors`, with its type, file, directive, id and
//! span. `#documents` has the documents of `GET /api/documents` and `#budget_events` the events
//! of `GET /api/budgets/{name}/interval/{year}/{month}`.
//!
//! The budget API computes its figures with built-in queries over `#budgets` (#479), so the two
//! agree on every figure, also where the API used to be wrong: it added the numbers of amounts in
//! different commodities and reported a budget's final `closed` for every month.
//!
//! `GET /api/errors` and `GET /api/documents` now read these tables (#479): their tables are
//! compared with the hand-written endpoints they replace, kept until they are removed.

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
use zhang_server::request::{BudgetIntervalDetailRequest, BudgetListRequest, QueryRequest};
use zhang_server::routes::budget::{get_budget_info, get_budget_interval_detail, get_budget_list};
use zhang_server::routes::query::{get_query_schema, run_query};
use zhang_server::state::SharedLedger;

fn fixture_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests").join(name)
}

/// The fixture, loaded the way the server loads a ledger.
async fn load(name: &str) -> SharedLedger {
    load_dir(&fixture_dir(name)).await
}

/// The ledger of `dir`, loaded the way the server loads a ledger.
async fn load_dir(dir: &Path) -> SharedLedger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::async_load(dir.to_path_buf(), "main.zhang".to_owned(), source)
        .await
        .unwrap_or_else(|error| panic!("{} should load: {error}", dir.display()));
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// A temporary ledger directory, removed when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    /// A directory with `files` (relative path, content).
    fn with(files: &[(&str, &str)]) -> ScratchDir {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-query-tables-{}", uuid::Uuid::new_v4())));
        for (name, content) in files {
            let path = dir.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn body(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The rows of `POST /api/query`, as objects keyed by column name.
async fn query(ledger: &SharedLedger, sql: &str) -> Vec<serde_json::Map<String, Value>> {
    let response = body(
        run_query(
            State(ledger.clone()),
            Json(QueryRequest {
                query: sql.to_owned(),
                count_total: None,
            }),
        )
        .await,
    )
    .await;
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

fn budget_name_of(row: &serde_json::Map<String, Value>) -> &str {
    row["name"].as_str().unwrap()
}

/// Checks every row of `#budgets` of the fixture against the budget API, and returns the number
/// of rows through the month of the ledger's last entry.
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
    // the last month of each budget's series, and the ledger's last month with a transaction
    let mut ends = std::collections::BTreeMap::new();
    for row in &rows {
        let month = (row["year"].as_u64().unwrap() as u32, row["month"].as_u64().unwrap() as u32);
        let end = ends.entry(row["name"].as_str().unwrap().to_owned()).or_insert(month);
        *end = (*end).max(month);
    }
    let last_transaction = query(&ledger, "SELECT max(date) AS last FROM #transactions").await[0]["last"]
        .as_str()
        .map(|date| (date[..4].parse::<u32>().unwrap(), date[5..7].parse::<u32>().unwrap()))
        .unwrap();
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
        // a month of the table lists the budgets the budget page lists for it, except for the
        // budgets whose series ended before it, after the last transaction
        let listed_names = listed.iter().map(|it| it["name"].as_str().unwrap()).collect::<BTreeSet<_>>();
        let row_names = in_month.iter().map(|it| it["name"].as_str().unwrap()).collect::<BTreeSet<_>>();
        assert!(row_names.is_subset(&listed_names), "{name} {year}-{month}: {row_names:?} {listed_names:?}");
        for ended in listed_names.difference(&row_names) {
            assert!((year, month) > last_transaction, "{name} {ended} {year}-{month}");
            assert!(ends[*ended] < (year, month), "{name} {ended} {year}-{month}");
        }
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
            // whether the budget was closed in or before the month
            assert_eq!(row["closed"], info["data"]["closed"], "{at}");
            assert_eq!(row["closed"], api["closed"], "{at}");
        }
    }
    // the months through the ledger's last entry; the later ones, through the current month,
    // carry the last one over
    let last_entry = query(&ledger, "SELECT max(date) AS last FROM #entries").await[0]["last"]
        .as_str()
        .map(|date| (date[..4].parse::<u32>().unwrap(), date[5..7].parse::<u32>().unwrap()))
        .unwrap();
    rows.iter()
        .filter(|row| (row["year"].as_u64().unwrap() as u32, row["month"].as_u64().unwrap() as u32) <= last_entry)
        .count()
}

#[tokio::test]
async fn budgets_are_the_amounts_of_the_budget_api() {
    assert_eq!(check_budgets("budget-sytem-syntax-and-category").await, 10);
    assert_eq!(check_budgets("budget-sytem-syntax-and-category-multiple-file").await, 10);
    // `fun`, closed in April, runs on through the current month, so May and June count too
    assert_eq!(check_budgets("query-zhang-tables").await, 12);
}

async fn check_errors(name: &str) -> usize {
    let ledger = load(name).await;
    let rows = query(&ledger, "SELECT kind, file, source, account, date, id, span_start, span_end FROM #errors").await;
    rows.len()
}

#[tokio::test]
async fn the_errors_table_has_the_errors_of_each_ledger() {
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
    assert_eq!(errors.len(), 12);
    for (column, ty) in [
        ("kind", "str"),
        ("message", "str"),
        ("file", "str"),
        ("line", "int"),
        ("column", "int"),
        ("date", "date"),
        ("account", "str"),
        ("id", "str"),
        ("span_start", "int"),
        ("span_end", "int"),
        ("metas", "metas"),
    ] {
        assert!(errors.contains(&(column.to_owned(), ty.to_owned())), "errors.{column}: {errors:?}");
    }
    let events = columns("budget_events");
    assert_eq!(
        events,
        [
            ("name", "str"),
            ("date", "date"),
            ("time", "str"),
            ("timestamp", "int"),
            ("type", "str"),
            ("amount", "amount")
        ]
        .map(|(name, ty)| (name.to_owned(), ty.to_owned()))
    );
    for (table, column, ty) in [
        ("balances", "actual", "amount"),
        ("balances", "passed", "bool"),
        ("documents", "source", "str"),
        ("documents", "path", "str"),
        ("documents", "transaction_id", "str"),
    ] {
        assert!(
            columns(table).contains(&(column.to_owned(), ty.to_owned())),
            "{table}.{column}: {:?}",
            columns(table)
        );
    }
}

/// A budget in CNY spent in CNY and USD, then closed: the ledger the survey of #479 found the
/// budget API wrong on. In April it spent 1630 + 1500 CNY, 300 USD (2100 CNY at 7.0, the price
/// as of April 2nd) and 60 CNY, 5290 CNY in all; the API used to add the 300 USD as 300.
const MULTI_CURRENCY_BUDGET: &str = r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Travel
  budget: food
1970-01-01 open Equity:Opening

2025-03-01 budget food CNY
2025-03-01 budget-add food 1000 CNY

2025-03-31 price USD 7.0 CNY
2025-04-30 price USD 7.3 CNY

2025-03-31 "Opening" "opening"
  Assets:Bank 10000 CNY
  Assets:USBank 1000 USD
  Equity:Opening -10000 CNY
  Equity:Opening -1000 USD

2025-03-31 23:30:00 "Late" "late march dinner"
  Expenses:Food 40 CNY
  Assets:Bank

2025-04-01 "Landlord" "big rent on the first"
  Expenses:Food 1630 CNY
  Assets:Bank

2025-04-01 "Garage" "car on the first"
  Expenses:Travel 1500 CNY
  Assets:Bank

2025-04-02 "Airline" "US trip"
  Expenses:Travel 300 USD
  Assets:USBank

2025-04-30 23:30:00 "Late" "late april dinner"
  Expenses:Food 60 CNY
  Assets:Bank

2025-05-01 "Landlord" "may rent"
  Expenses:Food 1630 CNY
  Assets:Bank

2025-05-02 budget-close food
"#;

/// Every figure of every row of `#budgets` that differs from what `GET /api/budgets` returns
/// for the budget and month: `(budget, year, month, column, table, api)`.
async fn budget_differences(ledger: &SharedLedger) -> Vec<(String, u32, u32, &'static str, Value, Value)> {
    let rows = query(ledger, "SELECT name, year, month, assigned, activity, available, closed FROM #budgets").await;
    let mut differences = vec![];
    for row in rows {
        let (year, month) = (row["year"].as_u64().unwrap() as u32, row["month"].as_u64().unwrap() as u32);
        let request = BudgetListRequest {
            year: Some(year),
            month: Some(month),
        };
        let listed = body(get_budget_list(State(ledger.clone()), UrlQuery(request)).await).await;
        let api = listed["data"].as_array().unwrap().iter().find(|it| it["name"] == row["name"]).unwrap().clone();
        for (column, api_field) in [
            ("assigned", "assigned_amount"),
            ("activity", "activity_amount"),
            ("available", "available_amount"),
        ] {
            if amount(&row[column]) != amount(&api[api_field]) {
                let number = |value: &Value| json!(zhang_query::decimal::to_plain_string(&amount(value).0.normalized()));
                differences.push((
                    budget_name_of(&row).to_owned(),
                    year,
                    month,
                    column,
                    number(&row[column]),
                    number(&api[api_field]),
                ));
            }
        }
        if row["closed"] != api["closed"] {
            differences.push((
                budget_name_of(&row).to_owned(),
                year,
                month,
                "closed",
                row["closed"].clone(),
                api["closed"].clone(),
            ));
        }
    }
    differences
}

#[tokio::test]
async fn the_budget_api_reports_the_figures_of_the_table() {
    let dir = ScratchDir::with(&[("main.zhang", MULTI_CURRENCY_BUDGET)]);
    let ledger = load_dir(&dir.0).await;
    assert_eq!(budget_differences(&ledger).await, vec![]);
    let request = BudgetListRequest {
        year: Some(2025),
        month: Some(4),
    };
    let april = body(get_budget_list(State(ledger.clone()), UrlQuery(request)).await).await;
    assert_eq!(
        amount(&april["data"][0]["activity_amount"]),
        amount(&json!({"number": "5290", "commodity": "CNY"}))
    );
    assert_eq!(april["data"][0]["closed"], json!(false));
    for name in ["budget-sytem-syntax-and-category", "query-zhang-tables"] {
        let ledger = load(name).await;
        assert_eq!(budget_differences(&ledger).await, vec![], "{name}");
    }
}

/// `#budget_events` has the events of the budget API (which has no close events), with the
/// same timestamps and amounts.
#[tokio::test]
async fn budget_events_are_the_events_of_the_budget_api() {
    let ledger = load("query-zhang-tables").await;
    let rows = query(
        &ledger,
        "SELECT name, year(date) AS y, month(date) AS m, timestamp, type, amount FROM #budget_events",
    )
    .await;
    assert_eq!(rows.len(), 7);
    let months = rows
        .iter()
        .map(|row| {
            (
                budget_name_of(row).to_owned(),
                row["y"].as_u64().unwrap() as u32,
                row["m"].as_u64().unwrap() as u32,
            )
        })
        .collect::<BTreeSet<_>>();
    for (name, year, month) in months {
        let request = BudgetIntervalDetailRequest {
            budget_name: name.clone(),
            year,
            month,
        };
        let detail = body(get_budget_interval_detail(State(ledger.clone()), UrlPath(request)).await).await;
        let mut api = detail["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["type"] == "BudgetEvent")
            .map(|it| {
                let kind = it["event_type"].as_str().unwrap().to_owned();
                (it["timestamp"].as_i64().unwrap(), amount(&it["amount"]), kind)
            })
            .collect::<Vec<_>>();
        let mut table = rows
            .iter()
            .filter(|row| budget_name_of(row) == name && row["y"] == json!(year) && row["m"] == json!(month) && row["type"] != "close")
            .map(|row| {
                let kind = match row["type"].as_str().unwrap() {
                    "assign" => "AddAssignedAmount",
                    _ => "Transfer",
                };
                (row["timestamp"].as_i64().unwrap(), amount(&row["amount"]), kind.to_owned())
            })
            .collect::<Vec<_>>();
        api.sort();
        table.sort();
        assert_eq!(table, api, "{name} {year}-{month}");
    }
    // the close of `fun` is an event of the table only
    let closes = query(&ledger, "SELECT name, date FROM #budget_events WHERE type = 'close'").await;
    assert_eq!(closes, vec![json!({"name": "fun", "date": "2024-04-01"}).as_object().unwrap().clone()]);
}

const DOCUMENTS_MAIN: &str = r#"
option "operating_currency" "CNY"
include "sub/more.zhang"

1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food

2024-01-02 document Assets:Bank "statements/jan.pdf"

2024-01-03 * "Shop" "documents of the transaction and of a posting"
  document: "receipts/a.pdf"
  document: "receipts/b.pdf"
  Expenses:Food 10 CNY
    document: "receipts/c.pdf"
  Assets:Bank

2024-01-04 * "Shop" "rejected: two implicit postings"
  document: "receipts/rejected.pdf"
  Expenses:Food
  Assets:Bank
"#;

const DOCUMENTS_MORE: &str = r#"
2024-01-01 document Assets:Bank "w.pdf"
"#;

/// `#documents` has every document of the ledger, and a document of a posting belongs to the
/// posting's account.
#[tokio::test]
async fn the_documents_table_has_every_document_and_a_postings_account() {
    let dir = ScratchDir::with(&[("main.zhang", DOCUMENTS_MAIN), ("sub/more.zhang", DOCUMENTS_MORE)]);
    let ledger = load_dir(&dir.0).await;
    let rows = query(&ledger, "SELECT date, account, path, transaction_id, source FROM #documents").await;
    assert_eq!(rows.len(), 5);
    // the posting's document belongs to its account
    assert_eq!(
        query(&ledger, "SELECT account, path FROM #documents WHERE source = 'posting'").await,
        vec![json!({"account": "Expenses:Food", "path": "receipts/c.pdf"}).as_object().unwrap().clone()]
    );
}
