//! The journal, new-transaction, documents and errors endpoints on the query engine (#479), with
//! hand-verified values. They pin what changed from the hand-written endpoints, each for a reason
//! `journals_golden.rs` names:
//!
//! - a page size of 0 and a page beyond what an offset can count are 400s, and a page past the end
//!   is empty instead of wrapping around;
//! - a repeated metadata key keeps every value (decision 7);
//! - a cost is per unit: a `{{total}}` cost is divided by the units, and `{}` shows the lot's cost;
//! - balance assertions are checked on the true balance and keep #485's `passed` and `tolerance`;
//! - a keyword is plain text, never a regular expression;
//! - payees exclude padding transactions and are sorted, as are the open accounts (decision 4);
//! - documents come from `#documents`, errors from `#errors` (by file, then position).

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::JournalRequest;
use zhang_server::routes::common::get_errors;
use zhang_server::routes::document::get_documents;
use zhang_server::routes::transaction::{get_info_for_new_transactions, get_journals};
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;

const LEDGER: &str = r#"option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

include "more.zhang"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity AAPL

1970-01-01 open Assets:Cash
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Old
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
1970-01-01 open Income:Gains

2024-01-01 10:30:00 * "Cafe" "lunch" #food ^receipt-1
  invoice: "a.pdf"
  invoice: "b.pdf"
  Assets:Cash -10.00 CNY
  Expenses:Food
    note: "x"

2024-01-02 * "Broker" "buy"
  Assets:Broker 10 AAPL {{1000 USD}}
  Assets:Cash -1000 USD

2024-01-03 * "Broker" "sell"
  Assets:Broker -10 AAPL {} @ 110 USD
  Assets:Cash 1100 USD
  Income:Gains

2024-01-04 balance Assets:Cash 100 CNY with pad Equity:Open

2024-01-05 balance Assets:Cash 100.004 ~ 0.01 CNY

2024-01-06 balance Assets:Cash 50 CNY

2024-01-07 * "zz-shop (a)" "a.b"
  Assets:Cash -1 CNY
  Expenses:Food

2024-01-08 close Assets:Old
"#;

const MORE: &str = r#"2024-01-09 * "Unbalanced" "in another file"
  Assets:Cash -1 CNY
  Expenses:Food 2 CNY
"#;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(files: &[(&str, &str)]) -> Scratch {
        let dir = std::env::temp_dir().join(format!("zhang-journals-engine-{}", uuid::Uuid::new_v4()));
        for (name, content) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Scratch {
            dir: dir.canonicalize().unwrap(),
        }
    }

    async fn ledger(&self) -> SharedLedger {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(self.dir.clone(), "main.zhang".to_owned(), source)
            .await
            .expect("the ledger loads");
        SharedLedger(Arc::new(RwLock::new(ledger)))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

async fn respond(response: impl IntoResponse) -> (StatusCode, Value) {
    let response = response.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn request(page: Option<u32>, size: Option<u32>, keyword: Option<&str>, tags: Option<&[&str]>) -> JournalRequest {
    JournalRequest {
        page,
        size,
        keyword: keyword.map(str::to_owned),
        tags: tags.map(|tags| tags.iter().map(|it| (*it).to_owned()).collect()),
        links: None,
    }
}

async fn journals(ledger: &SharedLedger, request: JournalRequest) -> (StatusCode, Value) {
    respond(get_journals(State(ledger.clone()), UrlQuery(request)).await).await
}

async fn page(ledger: &SharedLedger, keyword: Option<&str>, tags: Option<&[&str]>) -> Value {
    let (status, body) = journals(ledger, request(None, None, keyword, tags)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

/// The records of a page as `type payee narration`.
fn summary(page: &Value) -> Vec<String> {
    page["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            format!(
                "{} {} {}",
                it["type"].as_str().unwrap(),
                it["payee"].as_str().unwrap(),
                it["narration"].as_str().unwrap_or("null")
            )
        })
        .collect()
}

fn record<'a>(page: &'a Value, narration: &str) -> &'a Value {
    page["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["narration"] == narration)
        .unwrap_or_else(|| panic!("no record {narration}"))
}

fn amount(number: &str, commodity: &str) -> Value {
    json!({ "number": number, "commodity": commodity })
}

#[tokio::test]
async fn the_journal_lists_transactions_and_assertions_newest_first_in_the_order_of_entries() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let page = page(&ledger, None, None).await;
    assert_eq!(page["total_count"], 9);
    assert_eq!(
        summary(&page),
        vec![
            "Transaction Unbalanced in another file",
            "Transaction zz-shop (a) a.b",
            "BalanceCheck Balance Check Assets:Cash",
            "BalanceCheck Balance Check Assets:Cash",
            // within a day, #entries lists the assertions first: the padding transaction is newer than its check
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open",
            "BalanceCheck Balance Check Assets:Cash",
            "Transaction Broker sell",
            "Transaction Broker buy",
            "Transaction Cafe lunch",
        ]
    );
    // `sequence` is the position in #entries, newest first
    let sequences = page["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert!(sequences.windows(2).all(|it| it[0] > it[1]), "{sequences:?}");
    assert_eq!(record(&page, "in another file")["is_balanced"], false);
    assert_eq!(record(&page, "a.b")["is_balanced"], true);
}

#[tokio::test]
async fn a_transaction_keeps_every_value_of_a_repeated_key_and_its_postings_their_balances() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let page = page(&ledger, None, None).await;
    let lunch = record(&page, "lunch");
    assert_eq!(lunch["datetime"], "2024-01-01T10:30:00");
    assert_eq!(lunch["flag"], "*");
    assert_eq!(lunch["tags"], json!(["food"]));
    assert_eq!(lunch["links"], json!(["receipt-1"]));
    // decision 7: the old journal kept only `b.pdf`
    assert_eq!(
        lunch["metas"],
        json!([{"key": "invoice", "value": "a.pdf"}, {"key": "invoice", "value": "b.pdf"}])
    );
    assert_eq!(
        lunch["postings"],
        json!([
            {
                "account": "Assets:Cash",
                "unit": amount("-10.00", "CNY"),
                "cost": null,
                "inferred_unit": amount("-10.00", "CNY"),
                "account_before": amount("0.00", "CNY"),
                "account_after": amount("-10.00", "CNY"),
                "metas": [],
            },
            {
                // written without an amount: `unit` is null, `inferred_unit` what balances the transaction
                "account": "Expenses:Food",
                "unit": null,
                "cost": null,
                "inferred_unit": amount("10.00", "CNY"),
                "account_before": amount("0.00", "CNY"),
                "account_after": amount("10.00", "CNY"),
                "metas": [{"key": "note", "value": "x"}],
            },
        ])
    );
}

#[tokio::test]
async fn a_cost_is_the_per_unit_cost_of_the_lot() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let page = page(&ledger, None, None).await;
    // `{{1000 USD}}` for 10 AAPL: the old journal reported 1000 USD where a per-unit cost is expected
    let buy = &record(&page, "buy")["postings"][0];
    assert_eq!(buy["cost"], amount("100", "USD"));
    assert_eq!(buy["unit"], amount("10", "AAPL"));
    assert_eq!(buy["account_after"], amount("10", "AAPL"));
    // `{}` reduces the lot bought at 100 USD; the old journal had no cost, as none is written
    let sell = &record(&page, "sell")["postings"][0];
    assert_eq!(sell["cost"], amount("100", "USD"));
    assert_eq!(sell["unit"], amount("-10", "AAPL"));
    assert_eq!(sell["account_before"], amount("10", "AAPL"));
    assert_eq!(sell["account_after"], amount("0", "AAPL"));
}

#[tokio::test]
async fn balance_assertions_and_pads_keep_their_shape() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let page = page(&ledger, None, None).await;
    let records = page["records"].as_array().unwrap();
    let checks = records.iter().filter(|it| it["type"] == "BalanceCheck").collect::<Vec<_>>();
    // newest first: 50 CNY fails, 100.004 ~ 0.01 passes, the pad brings the cash to 100
    let check = |idx: usize, actual: &str, difference: &str, asserted: &str, tolerance: Value, passed: bool| {
        let item = checks[idx];
        assert_eq!(item["payee"], "Balance Check");
        assert_eq!(item["narration"], "Assets:Cash");
        assert_eq!(item["type_"], "C");
        assert_eq!(item["tolerance"], tolerance);
        assert_eq!(item["passed"], passed);
        assert_eq!(
            item["postings"],
            json!([{
                "account": "Assets:Cash",
                "unit": amount(difference, "CNY"),
                "cost": null,
                "inferred_unit": amount(difference, "CNY"),
                "account_before": amount(actual, "CNY"),
                "account_after": amount(asserted, "CNY"),
                "metas": [],
            }])
        );
    };
    check(0, "100.00", "-50.00", "50.00", Value::Null, false);
    check(1, "100.00", "0.004", "100.004", json!("0.01"), true);
    check(2, "100.00", "0.00", "100.00", Value::Null, true);
    assert_eq!(checks[0]["datetime"], "2024-01-06T00:00:00");

    let pad = records.iter().find(|it| it["type"] == "BalancePad").unwrap();
    assert_eq!(pad["type_"], "P");
    assert_eq!(pad["payee"], "Balance Pad");
    assert_eq!(pad["datetime"], "2024-01-04T00:00:00");
    // -10.00 CNY of the lunch and -1 CNY of the shop come later: the pad brings -10.00 to 100
    assert_eq!(pad["postings"][0]["unit"], amount("110.00", "CNY"));
    assert_eq!(pad["postings"][0]["account_after"], amount("100.00", "CNY"));
    assert_eq!(pad["postings"][1]["account"], "Equity:Open");
    assert_eq!(pad["postings"][1]["unit"], Value::Null);
    assert_eq!(pad["postings"][1]["inferred_unit"], amount("-110.00", "CNY"));
}

#[tokio::test]
async fn a_keyword_is_plain_text_that_matches_ignoring_case() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    // payees (`Unbalanced` too), and the `Balance Check` of the assertions
    assert_eq!(
        summary(&page(&ledger, Some("BALANCE"), None).await),
        vec![
            "Transaction Unbalanced in another file",
            "BalanceCheck Balance Check Assets:Cash",
            "BalanceCheck Balance Check Assets:Cash",
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open",
            "BalanceCheck Balance Check Assets:Cash",
        ]
    );
    // accounts: the padding transaction posts to Equity:Open, and the balance ... with pad names it
    assert_eq!(
        summary(&page(&ledger, Some("equity:open"), None).await),
        vec![
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open",
            "BalanceCheck Balance Check Assets:Cash"
        ]
    );
    // a regular expression is text: `.` and `(` are themselves
    assert_eq!(summary(&page(&ledger, Some("a.b"), None).await), vec!["Transaction zz-shop (a) a.b"]);
    assert_eq!(summary(&page(&ledger, Some("p (a"), None).await), vec!["Transaction zz-shop (a) a.b"]);
    assert_eq!(page(&ledger, Some(".*"), None).await["total_count"], 0);
    // tags and links, of transactions only
    assert_eq!(summary(&page(&ledger, Some("receipt"), None).await), vec!["Transaction Cafe lunch"]);
    assert_eq!(summary(&page(&ledger, None, Some(&["food", "other"])).await), vec!["Transaction Cafe lunch"]);
    assert_eq!(page(&ledger, Some("balance"), Some(&["food"])).await["total_count"], 0);
    // an empty keyword searches nothing
    assert_eq!(page(&ledger, Some(""), None).await["total_count"], 9);
}

#[tokio::test]
async fn bad_pages_are_bad_requests_and_a_page_past_the_end_is_empty() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    // the old journal divided by the size and panicked
    let (status, _) = journals(&ledger, request(Some(1), Some(0), None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = journals(&ledger, request(Some(u32::MAX), Some(u32::MAX), None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // the old journal computed the offset in 32 bits: 42949674 × 100 wrapped around to 4, a page of rows
    let (status, body) = journals(&ledger, request(Some(42949674), Some(100), None, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"],
        json!({"total_count": 9, "total_page": 1, "page_size": 100, "current_page": 42949674, "records": []})
    );
    let (status, body) = journals(&ledger, request(Some(3), Some(4), None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary(&body["data"]), vec!["Transaction Cafe lunch"]);
    assert_eq!(body["data"]["total_page"], 3);

    let (status, _) = respond(get_errors(State(ledger.clone()), axum::extract::Query(request(Some(1), Some(0), None, None))).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_new_transaction_form_suggests_sorted_payees_without_pads_and_open_accounts() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let (status, body) = respond(get_info_for_new_transactions(State(ledger.clone())).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"],
        json!({
            "payee": ["Broker", "Cafe", "Unbalanced", "zz-shop (a)"],
            "account_name": ["Assets:Broker", "Assets:Cash", "Equity:Open", "Expenses:Food", "Income:Gains"],
        })
    );
}

#[tokio::test]
async fn documents_come_from_the_documents_table_newest_first() {
    let ledger_text = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food

2024-01-01 document Assets:Cash "statements/jan.pdf"

2024-01-02 09:00:00 * "Shop" "receipts"
  document: "receipts/a.pdf"
  document: "receipts/b.png"
  Assets:Cash -1 CNY
  Expenses:Food
    document: "receipts/c.pdf"
"#;
    let scratch = Scratch::new(&[("main.zhang", ledger_text)]);
    let ledger = scratch.ledger().await;
    let id = {
        let guard = ledger.read().await;
        let store = guard.store.read().unwrap();
        store.transactions.values().next().unwrap().id.to_string()
    };
    let (status, body) = respond(get_documents(State(ledger.clone())).await).await;
    assert_eq!(status, StatusCode::OK);
    let document = |datetime: &str, path: &str, extension: &str, account: Value, trx_id: Value| {
        json!({
            "datetime": datetime,
            "filename": path.rsplit('/').next().unwrap(),
            "path": path,
            "extension": extension,
            "account": account,
            "trx_id": trx_id,
        })
    };
    assert_eq!(
        body["data"],
        json!([
            // the documents of a transaction in written order: its own, then its postings'
            document("2024-01-02T09:00:00", "receipts/a.pdf", "application/pdf", Value::Null, json!(id)),
            document("2024-01-02T09:00:00", "receipts/b.png", "image/png", Value::Null, json!(id)),
            // a posting's document belongs to the posting's account too
            document("2024-01-02T09:00:00", "receipts/c.pdf", "application/pdf", json!("Expenses:Food"), json!(id)),
            document(
                "2024-01-01T00:00:00",
                "statements/jan.pdf",
                "application/pdf",
                json!("Assets:Cash"),
                Value::Null
            ),
        ])
    );
}

#[tokio::test]
async fn errors_are_listed_by_file_then_position_one_page_at_a_time() {
    let scratch = Scratch::new(&[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = scratch.ledger().await;
    let (status, body) = respond(get_errors(State(ledger.clone()), axum::extract::Query(request(Some(1), Some(10), None, None))).await).await;
    assert_eq!(status, StatusCode::OK);
    let data = &body["data"];
    assert_eq!(data["total_count"], 2);
    let records = data["records"].as_array().unwrap();
    // main.zhang before more.zhang, whatever the order zhang found them in
    assert_eq!(records[0]["error_type"], "AccountBalanceCheckError");
    assert_eq!(records[0]["span"]["filename"], "main.zhang");
    assert_eq!(records[0]["span"]["content"], "2024-01-06 balance Assets:Cash 50 CNY");
    assert_eq!(records[0]["metas"], json!({"account_name": "Assets:Cash"}));
    let start = LEDGER.find("2024-01-06 balance").unwrap();
    assert_eq!(records[0]["span"]["start"], start);
    assert_eq!(records[1]["error_type"], "UnbalancedTransaction");
    assert_eq!(records[1]["span"]["filename"], "more.zhang");
    assert_eq!(records[1]["span"]["start"], 0);
    assert!(records[1]["metas"]["txn_id"].is_string());

    let (_, body) = respond(get_errors(State(ledger.clone()), axum::extract::Query(request(Some(2), Some(1), None, None))).await).await;
    assert_eq!(body["data"]["total_page"], 2);
    assert_eq!(body["data"]["records"][0]["error_type"], "UnbalancedTransaction");
}
