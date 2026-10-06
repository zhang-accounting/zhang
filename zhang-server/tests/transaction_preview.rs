//! `POST /api/transactions/preview` and `POST /api/transactions/{transaction_id}/preview`: what a create or an update
//! would write, found by the server's own parser, exporter and booking, without writing it.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::CreateTransactionRequest;
use zhang_server::routes::transaction::{create_new_transaction, preview_new_transaction, preview_transaction_update};
use zhang_server::state::{SharedLedger, SharedReloadSender};
use zhang_server::ReloadSender;

const LEDGER: &str = r#"option "operating_currency" "CNY"
option "timezone" "UTC"
1970-01-01 commodity CNY
  precision: 2
1970-01-01 commodity USD
1970-01-01 commodity AAPL
  precision: 0
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
2023-06-30 close Expenses:Food
"#;

async fn ledger(dir: &Path, main: &str) -> SharedLedger {
    std::fs::write(dir.join("main.zhang"), main).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::load(dir.to_path_buf(), "main.zhang".to_owned(), source).unwrap_or_else(|error| panic!("ledger should load: {error}"));
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// A create or update request on 2024-01-15 12:00 UTC with `postings`.
fn request(postings: Value) -> CreateTransactionRequest {
    serde_json::from_value(json!({
        "datetime": "2024-01-15T12:00:00Z",
        "payee": "Broker",
        "narration": "trade",
        "postings": postings,
        "metas": [],
        "tags": [],
        "links": [],
    }))
    .unwrap()
}

async fn body(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn preview(ledger: &SharedLedger, postings: Value) -> Value {
    let (status, body) = body(preview_new_transaction(State(ledger.clone()), Json(request(postings))).await.into_response()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

#[tokio::test]
async fn a_stock_purchase_balances_by_its_cost() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    let preview = preview(
        &ledger,
        json!([
            {"account": "Assets:Broker", "unit": "10 AAPL", "cost": "{150USD}"},
            {"account": "Assets:Cash", "unit": "-1,500 USD"},
        ]),
    )
    .await;
    assert_eq!(
        preview,
        json!({
            "text": "2024-01-15 12:00:00 * \"Broker\" \"trade\"\n  Assets:Broker 10 AAPL { 150 USD }\n  Assets:Cash -1500 USD",
            "field_errors": [],
            "unbalanced": [],
            "errors": [],
        })
    );
}

#[tokio::test]
async fn units_weigh_by_their_price_and_round_at_their_precision() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    // `@6CNY` is a price the ledger reads, written back as `@ 6 CNY`
    let priced = preview(
        &ledger,
        json!([
            {"account": "Assets:Cash", "unit": "-10 USD", "price": "@6CNY"},
            {"account": "Assets:Broker", "unit": "60 CNY"},
        ]),
    )
    .await;
    assert_eq!(priced["unbalanced"], json!([]));
    assert_eq!(
        priced["text"],
        "2024-01-15 12:00:00 * \"Broker\" \"trade\"\n  Assets:Cash -10 USD @ 6 CNY\n  Assets:Broker 60 CNY"
    );
    // CNY has 2 decimals: 0.005 CNY is no imbalance, 0.5 CNY is
    let dust = preview(
        &ledger,
        json!([{"account": "Assets:Cash", "unit": "10.005 CNY"}, {"account": "Assets:Broker", "unit": "-10"}]),
    )
    .await;
    assert_eq!(dust["unbalanced"], json!([]));
    assert_eq!(dust["errors"], json!([]));
    let off = preview(
        &ledger,
        json!([{"account": "Assets:Cash", "unit": "10.5 CNY"}, {"account": "Assets:Broker", "unit": "-10 CNY"}]),
    )
    .await;
    assert_eq!(off["unbalanced"], json!([{"number": "0.50", "commodity": "CNY"}]));
    assert_eq!(off["errors"], json!([{"error_type": "UnbalancedTransaction", "metas": {}}]));
}

#[tokio::test]
async fn units_are_read_with_the_ledger_grammar() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    // grouped digits, an expression, a trailing dot and a number alone, in the operating currency
    let preview = preview(
        &ledger,
        json!([
            {"account": "Assets:Cash", "unit": "1,000 CNY"},
            {"account": "Assets:Cash", "unit": "(10 + 2) / 4 CNY"},
            {"account": "Assets:Cash", "unit": "5."},
            {"account": "Assets:Broker", "unit": null},
        ]),
    )
    .await;
    assert_eq!(
        preview["text"],
        "2024-01-15 12:00:00 * \"Broker\" \"trade\"\n  Assets:Cash 1000 CNY\n  Assets:Cash 3 CNY\n  Assets:Cash 5 CNY\n  Assets:Broker"
    );
    assert_eq!(preview["unbalanced"], json!([]));
}

#[tokio::test]
async fn every_invalid_field_is_named_and_nothing_else_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    let postings = json!([
        {"account": "Assets:Broker", "unit": "10 AAPL {150 USD}"},
        {"account": "Assets:Cash", "unit": "-1500 USD", "cost": "150 USD", "price": "6 USD"},
    ]);
    let preview = preview(&ledger, postings.clone()).await;
    assert_eq!(preview["text"], Value::Null);
    assert_eq!(preview["unbalanced"], Value::Null);
    assert_eq!(preview["errors"], json!([]));
    // each with the kind and the value a client tells it with in its own words
    let fields = preview["field_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| (error["posting"].clone(), error["field"].clone(), error["kind"].clone(), error["value"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            (json!(0), json!("unit"), json!("invalid_amount"), json!("10 AAPL {150 USD}")),
            (json!(1), json!("cost"), json!("invalid_cost"), json!("150 USD")),
            (json!(1), json!("price"), json!("invalid_price"), json!("6 USD")),
        ]
    );
    // the create answers with the first one
    let (sender, _receiver) = mpsc::channel(1);
    let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
    let (status, body) = body(
        create_new_transaction(State(ledger.clone()), reload, Json(request(postings)))
            .await
            .into_response(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], preview["field_errors"][0]["message"]);
}

#[tokio::test]
async fn the_accounts_are_checked_at_the_transactions_date() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    let preview = preview(
        &ledger,
        json!([{"account": "Assets:Cash", "unit": "-5 CNY"}, {"account": "Expenses:Food", "unit": "5 CNY"}]),
    )
    .await;
    assert_eq!(
        preview["errors"],
        json!([{"error_type": "AccountClosed", "metas": {"account_name": "Expenses:Food"}}])
    );
    assert_eq!(preview["unbalanced"], json!([]));
}

#[tokio::test]
async fn an_update_preview_keeps_flags_and_bare_metadata_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let main = format!(
        "{LEDGER}{}",
        "2024-01-15 12:00:00 * \"Broker\" \"trade\"\n  rate: 1.5\n  ! Assets:Cash -5 CNY\n  Assets:Broker 5 CNY\n"
    );
    let ledger = ledger(dir.path(), &main).await;
    let id = ledger.read().await.transactions()[0].0.to_string();
    let mut update = request(json!([{"account": "Assets:Cash", "unit": "-6 CNY"}, {"account": "Assets:Broker", "unit": "6 CNY"}]));
    update.metas = serde_json::from_value(json!([{"key": "rate", "value": "1.5"}])).unwrap();
    let (status, preview) = body(
        preview_transaction_update(State(ledger.clone()), UrlPath((id,)), Json(update))
            .await
            .into_response(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        preview["data"]["text"],
        "2024-01-15 12:00:00 * \"Broker\" \"trade\"\n  rate: 1.5\n  ! Assets:Cash -6 CNY\n  Assets:Broker 6 CNY"
    );
    assert_eq!(preview["data"]["unbalanced"], json!([]));
    assert_eq!(std::fs::read_to_string(dir.path().join("main.zhang")).unwrap(), main);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "nothing was written");
}

#[tokio::test]
async fn an_update_preview_of_no_transaction_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    let response = preview_transaction_update(State(ledger), UrlPath((uuid::Uuid::new_v4().to_string(),)), Json(request(json!([]))))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_name_beancount_rejects_has_its_own_kind() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.bean"), "1970-01-01 open Assets:Cash\n1970-01-01 commodity CNY\n").unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
    let ledger = Ledger::load(dir.path().to_path_buf(), "main.bean".to_owned(), source).unwrap();
    let ledger = SharedLedger(Arc::new(RwLock::new(ledger)));
    let preview = preview(
        &ledger,
        json!([
            {"account": "Assets:Cash", "unit": "-5 cny"},
            {"account": "Assets:bank", "unit": "5 CNY"},
        ]),
    )
    .await;
    let kinds = preview["field_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| (error["kind"].clone(), error["value"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [(json!("beancount_commodity"), json!("cny")), (json!("beancount_account"), json!("Assets:bank"))]
    );
}

/// Units, a cost or a price that divide by zero made the parser panic, which the preview answered with a 500. Each is
/// a field error, as any that does not read back, and a create answers with the first one, a 400.
#[tokio::test]
async fn a_division_by_zero_is_a_field_error() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = ledger(dir.path(), LEDGER).await;
    let postings = json!([
        {"account": "Assets:Broker", "unit": "1/0 AAPL", "cost": "{1/0 USD}", "price": "@ 1/0 USD"},
        {"account": "Assets:Cash", "unit": "10 / (2 - 2)"},
    ]);
    let preview = preview(&ledger, postings.clone()).await;
    assert_eq!(preview["text"], Value::Null);
    let fields = preview["field_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| (error["posting"].clone(), error["field"].clone(), error["kind"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            (json!(0), json!("unit"), json!("invalid_amount")),
            (json!(0), json!("cost"), json!("invalid_cost")),
            (json!(0), json!("price"), json!("invalid_price")),
            (json!(1), json!("unit"), json!("invalid_amount")),
        ]
    );
    let (sender, _receiver) = mpsc::channel(1);
    let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
    let (status, body) = body(
        create_new_transaction(State(ledger.clone()), reload, Json(request(postings)))
            .await
            .into_response(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], preview["field_errors"][0]["message"]);
}
