//! Requests take the ledger's wall-clock time, as responses give it (audit finding D4, PLAN DECISION 12). The ledger is in
//! Asia/Shanghai: a request writing `2024-01-02T07:00:00` means 07:00 there, whatever the timezone of the browser that
//! sends it, so an edit that leaves the time as the journal showed it keeps it. An instant with `Z`, what requests took
//! before, still reads as the wall-clock time it is in the ledger's timezone.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::{BuiltinParamValue, BuiltinQueryRunRequest, CreateTransactionRequest, NewTransactionInfoRequest};
use zhang_server::routes::query::run_builtin_query;
use zhang_server::routes::transaction::{get_info_for_new_transactions, get_journals, preview_new_transaction, update_single_transaction};
use zhang_server::routes::Query;
use zhang_server::state::{SharedLedger, SharedReloadSender};
use zhang_server::ReloadSender;

const LEDGER: &str = r#"option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
  budget: Food
1970-01-01 open Assets:Old
2024-01-01 budget Food CNY
2024-01-01 07:30:00 budget-add Food 100 CNY
2024-01-02 07:00:00 * "Shop" "breakfast"
  Assets:Cash -5 CNY
  Expenses:Food 5 CNY
2024-01-02 06:00:00 close Assets:Old
"#;

/// 2024-01-02 07:00 in Asia/Shanghai
fn breakfast_time() -> Clock {
    Clock::Fixed(DateTime::parse_from_rfc3339("2024-01-01T23:00:00Z").unwrap().with_timezone(&Utc))
}

async fn ledger(dir: &Path) -> SharedLedger {
    std::fs::write(dir.join("main.zhang"), LEDGER).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::load_with_clock(dir.to_path_buf(), "main.zhang".to_owned(), source, breakfast_time()).unwrap();
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

async fn data(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    assert!(status.is_success(), "{status}: {body}");
    body["data"].clone()
}

/// the breakfast as the journal shows it, edited to 6 CNY with its date and time sent as `datetime`
fn edit(datetime: &str) -> CreateTransactionRequest {
    serde_json::from_value(json!({
        "datetime": datetime,
        "payee": "Shop",
        "narration": "breakfast",
        "postings": [{"account": "Assets:Cash", "unit": "-6 CNY"}, {"account": "Expenses:Food", "unit": "6 CNY"}],
        "metas": [],
        "tags": [],
        "links": [],
    }))
    .unwrap()
}

async fn breakfast(ledger: &SharedLedger) -> Value {
    let journal = data(get_journals(State(ledger.clone()), Query(serde_json::from_value(json!({})).unwrap())).await).await;
    journal["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["narration"] == "breakfast")
        .unwrap()
        .clone()
}

async fn save(ledger: &SharedLedger, dir: &Path, datetime: &str) -> String {
    let id = breakfast(ledger).await["id"].as_str().unwrap().to_owned();
    let (sender, _receiver) = mpsc::channel(1);
    let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
    data(update_single_transaction(State(ledger.clone()), reload, UrlPath((id,)), Json(edit(datetime))).await).await;
    std::fs::read_to_string(dir.join("main.zhang")).unwrap()
}

#[tokio::test]
async fn an_edit_that_sends_the_time_the_journal_shows_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path()).await;
    // the journal shows the ledger's wall-clock time, and the form sends it back as it is
    let shown = breakfast(&ledger).await["datetime"].as_str().unwrap().to_owned();
    assert_eq!(shown, "2024-01-02T07:00:00");
    let written = save(&ledger, dir.path(), &shown).await;
    assert!(
        written.contains("2024-01-02 07:00:00 * \"Shop\" \"breakfast\"\n  Assets:Cash -6 CNY"),
        "{written}"
    );
}

#[tokio::test]
async fn an_instant_still_reads_as_its_time_in_the_ledgers_timezone() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path()).await;
    // 23:00 UTC is 07:00 the next day in Asia/Shanghai
    let written = save(&ledger, dir.path(), "2024-01-01T23:00:00Z").await;
    assert!(written.contains("2024-01-02 07:00:00 * \"Shop\" \"breakfast\""), "{written}");
}

#[tokio::test]
async fn the_preview_writes_the_wall_clock_time_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path()).await;
    let preview = data(preview_new_transaction(State(ledger.clone()), Json(edit("2024-01-02T07:00:00"))).await).await;
    assert!(preview["text"].as_str().unwrap().starts_with("2024-01-02 07:00:00 * \"Shop\""), "{preview}");
}

#[tokio::test]
async fn the_new_transaction_form_gets_the_ledgers_time_and_its_open_accounts_at_a_wall_clock_time() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path()).await;
    let info = |datetime: Option<&str>| {
        let ledger = ledger.clone();
        let datetime = datetime.map(|it| it.parse().unwrap());
        async move { data(get_info_for_new_transactions(State(ledger), Query(NewTransactionInfoRequest { datetime })).await).await }
    };
    let now = info(None).await;
    // now by the ledger's clock, in its timezone
    assert_eq!(now["now"], "2024-01-02T07:00:00");
    let accounts = |info: &Value| info["account_name"].as_array().unwrap().iter().any(|it| it == "Assets:Old");
    // Assets:Old is closed at 06:00 on 2024-01-02 in Asia/Shanghai: open at 05:00 there, closed at 07:00
    assert!(accounts(&info(Some("2024-01-02T05:00:00")).await));
    assert!(!accounts(&info(Some("2024-01-02T07:00:00")).await));
}

#[tokio::test]
async fn a_budget_event_has_its_wall_clock_time() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path()).await;
    // the budget page lists the month's events (`budgets.events`) and postings (`budgets.postings`), newest first,
    // each with its `date` and `time`: the ledger's wall-clock time in its timezone
    let params: HashMap<String, Option<BuiltinParamValue>> = serde_json::from_value(json!({ "name": "Food", "month": "2024-01-01" })).unwrap();
    let run = |name: &str| {
        let ledger = ledger.clone();
        let request = BuiltinQueryRunRequest {
            params: params.clone(),
            count_total: None,
        };
        let name = name.to_owned();
        async move { data(run_builtin_query(State(ledger), UrlPath((name,)), Json(request)).await).await }
    };
    let day_time = |result: &Value, row: usize| {
        let column = |name: &str| result["columns"].as_array().unwrap().iter().position(|it| it["name"] == name).unwrap();
        (result["rows"][row][column("date")].clone(), result["rows"][row][column("time")].clone())
    };
    let events = run("budgets.events").await;
    assert_eq!(day_time(&events, 0), (json!("2024-01-01"), json!("07:30:00")));
    let postings = run("budgets.postings").await;
    assert_eq!(day_time(&postings, 0), (json!("2024-01-02"), json!("07:00:00")));
}
