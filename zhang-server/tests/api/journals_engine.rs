//! The journal, new-transaction, documents and errors endpoints on the query engine (#479), with
//! hand-verified values. They pin what changed from the hand-written endpoints, each for a reason
//! `journals_golden.rs` names:
//!
//! - a page size outside 1 to 1000 is a 400, and a page past the end is empty instead of wrapping
//!   around;
//! - tags and links keep their written order, also through a save;
//! - a repeated metadata key keeps every value (decision 7);
//! - a cost is per unit: a `{{total}}` cost is divided by the units, and `{}` shows the cost of the lots it
//!   reduces when they share one;
//! - a `balance ... with pad` stays after the paddings of its time, as zhang checks it (#485), on every page;
//! - balance assertions are checked on the true balance and keep #485's `passed` and `tolerance`;
//! - a keyword is plain text, never a regular expression;
//! - payees exclude padding transactions and are sorted, as are the open accounts (decision 4);
//! - documents come from `#documents`, errors from `#errors` (by file, then position).

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_ast::Account;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::ledger::Ledger;
use zhang_core::pipeline::AccountUse;
use zhang_server::request::{BuiltinQueryRunRequest, JournalRequest};
use zhang_server::routes::document::download_document;
use zhang_server::routes::query::run_builtin_query;
use zhang_server::routes::transaction::{get_journals, update_single_transaction};
use zhang_server::routes::{Base64Path, Query as UrlQuery};
use zhang_server::state::{SharedLedger, SharedReloadSender};
use zhang_server::ReloadSender;
use zhang_testkit::http::{respond, shared};
use zhang_testkit::ledger::Scratch;

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

/// The ledger of `scratch` as the handlers share it.
fn shared_ledger(scratch: &Scratch) -> SharedLedger {
    shared(scratch.ledger().expect("the ledger loads"))
}

/// The first column of the rows of the built-in query `name` with `params`, as strings: the lists of the forms.
async fn names(ledger: &SharedLedger, name: &str, params: Value) -> Vec<String> {
    let (status, rows) = builtin(ledger, name, params).await;
    assert_eq!(status, StatusCode::OK);
    rows.iter()
        .map(|row| row.as_object().unwrap().values().next().unwrap().as_str().unwrap().to_owned())
        .collect()
}

/// The ledger's current date and time (`ledger.now`), the instant the forms ask the accounts at.
async fn ledger_now(ledger: &SharedLedger) -> (String, String) {
    let (status, rows) = builtin(ledger, "ledger.now", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    (rows[0]["date"].as_str().unwrap().to_owned(), rows[0]["time"].as_str().unwrap().to_owned())
}

/// The rows of the built-in query `name` with `params` (JSON values by name), as objects keyed by column name: what the
/// documents lists read through `POST /api/query/builtins/{name}`.
async fn builtin(ledger: &SharedLedger, name: &str, params: Value) -> (StatusCode, Vec<Value>) {
    let request = BuiltinQueryRunRequest {
        params: serde_json::from_value(params).unwrap(),
        count_total: None,
    };
    let (status, body) = respond(run_builtin_query(State(ledger.clone()), UrlPath((name.to_owned(),)), Json(request)).await).await;
    let columns = body["data"]["columns"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let rows = body["data"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|row| Value::Object(columns.iter().cloned().zip(row.as_array().unwrap().iter().cloned()).collect()))
        .collect();
    (status, rows)
}

/// One page of the error box: the rows of `journals.errors` with `size` and `offset`, counted, and the `total`.
async fn errors_page(ledger: &SharedLedger, size: i64, offset: i64) -> (Vec<Value>, u64) {
    let request = BuiltinQueryRunRequest {
        params: serde_json::from_value(json!({ "size": size, "offset": offset })).unwrap(),
        count_total: Some(true),
    };
    let (status, body) = respond(run_builtin_query(State(ledger.clone()), UrlPath(("journals.errors".to_owned(),)), Json(request)).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let columns = body["data"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let rows = body["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| Value::Object(columns.iter().cloned().zip(row.as_array().unwrap().iter().cloned()).collect()))
        .collect();
    (rows, body["data"]["total"].as_u64().unwrap())
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
async fn the_journal_lists_transactions_and_assertions_newest_first_as_zhang_checks_them() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
    let page = page(&ledger, None, None).await;
    assert_eq!(page["total_count"], 9);
    assert_eq!(
        summary(&page),
        vec![
            "Transaction Unbalanced in another file",
            "Transaction zz-shop (a) a.b",
            "BalanceCheck Balance Check Assets:Cash",
            "BalanceCheck Balance Check Assets:Cash",
            // the `balance ... with pad` is checked after its padding (#485)
            "BalanceCheck Balance Check Assets:Cash",
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open",
            "Transaction Broker sell",
            "Transaction Broker buy",
            "Transaction Cafe lunch",
        ]
    );
    // `sequence` is the position in the processing order, newest first
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
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
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
                // the rest of the posting line as written (#473): none here
                "written": {"cost": null, "price": null, "comment": null},
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
                "written": {"cost": null, "price": null, "comment": null},
            },
        ])
    );
}

#[tokio::test]
async fn a_cost_is_the_per_unit_cost_of_the_lot() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
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
async fn a_reduction_across_lots_has_their_cost_only_when_they_share_it() {
    let ledger_text = r#"option "operating_currency" "USD"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Income:Gains

2024-01-01 * "Buy" "first lot"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD

2024-01-02 * "Buy" "second lot, another cost"
  Assets:Broker 5 AAPL {120 USD}
  Assets:Cash -600 USD

2024-01-03 * "Buy" "third lot, the first cost"
  Assets:Broker 5 AAPL {100 USD, 2024-01-03}
  Assets:Cash -500 USD

2024-01-04 * "Sell" "across the lots of 100 USD"
  Assets:Broker -12 AAPL {100 USD}
  Assets:Cash 1200 USD

2024-01-05 * "Sell" "across the lots of 100 and 120 USD"
  Assets:Broker -7 AAPL {}
  Assets:Cash 820 USD
  Income:Gains
"#;
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", ledger_text)]);
    let ledger = shared_ledger(&scratch);
    let page = page(&ledger, None, None).await;
    // 10 + 2 of the two lots bought at 100 USD: one posting, their common cost
    let same = &record(&page, "across the lots of 100 USD")["postings"][0];
    assert_eq!(same["unit"], amount("-12", "AAPL"));
    assert_eq!(same["inferred_unit"], amount("-12", "AAPL"));
    assert_eq!(same["cost"], amount("100", "USD"));
    assert_eq!(same["account_after"], amount("8", "AAPL"));
    // first in, first out: the 5 bought at 120 USD on 01-02, then 2 of the 3 left at 100 USD: no single cost
    let mixed = &record(&page, "across the lots of 100 and 120 USD")["postings"][0];
    assert_eq!(mixed["unit"], amount("-7", "AAPL"));
    assert_eq!(mixed["cost"], Value::Null);
    assert_eq!(mixed["account_before"], amount("8", "AAPL"));
    assert_eq!(mixed["account_after"], amount("1", "AAPL"));
    assert_eq!(record(&page, "across the lots of 100 and 120 USD")["is_balanced"], true);
}

#[tokio::test]
async fn a_balance_with_pad_follows_the_paddings_of_its_time_on_every_page() {
    // a day with a balance, a balance with pad written before it, transactions before and after the pad's time,
    // and a padding of another account at the same time
    let ledger_text = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Assets:B
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food

2024-01-05 08:00:00 * "Early" "before the pads"
  Assets:A -1 CNY
  Expenses:Food

2024-01-05 12:00:00 balance Assets:A 100 CNY with pad Equity:Open
2024-01-05 12:00:00 balance Assets:B 50 CNY with pad Equity:Open
2024-01-05 balance Assets:B 0 CNY

2024-01-05 12:00:00 * "Noon" "at the time of the pads"
  Assets:A -2 CNY
  Expenses:Food

2024-01-05 18:00:00 * "Late" "after the pads"
  Assets:A -3 CNY
  Expenses:Food
"#;
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", ledger_text)]);
    let ledger = shared_ledger(&scratch);
    let all = page(&ledger, None, None).await;
    let order = all["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| format!("{} {}", it["type"].as_str().unwrap(), it["datetime"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            "Transaction 2024-01-05T18:00:00",
            "Transaction 2024-01-05T12:00:00",
            // both checks after both paddings of their time, as zhang checks them
            "BalanceCheck 2024-01-05T12:00:00",
            "BalanceCheck 2024-01-05T12:00:00",
            "BalancePad 2024-01-05T12:00:00",
            "BalancePad 2024-01-05T12:00:00",
            "Transaction 2024-01-05T08:00:00",
            // a balance is checked at the start of its day
            "BalanceCheck 2024-01-05T00:00:00",
        ]
    );
    let checks = all["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|it| it["type"] == "BalanceCheck")
        .collect::<Vec<_>>();
    assert!(checks.iter().all(|it| it["passed"] == true), "{checks:?}");
    // pages of one row are windows of the same order
    for (idx, expected) in all["records"].as_array().unwrap().iter().enumerate() {
        let (status, body) = journals(&ledger, request(Some(idx as u32 + 1), Some(1), None, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body["data"]["records"][0], expected, "page {}", idx + 1);
    }
}

#[tokio::test]
async fn balance_assertions_and_pads_keep_their_shape() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
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
        // the assertion in the fields an account's journal describes it with too
        assert_eq!(item["asserted"], amount(asserted, "CNY"));
        assert_eq!(item["checked_balance"], amount(actual, "CNY"));
        assert_eq!(item["difference"], amount(difference, "CNY"));
        assert_eq!(
            item["postings"],
            json!([{
                "account": "Assets:Cash",
                "unit": null,
                "cost": null,
                // an assertion books nothing: the balance before and after it is the checked balance
                "inferred_unit": amount("0", "CNY"),
                "account_before": amount(actual, "CNY"),
                "account_after": amount(actual, "CNY"),
                "metas": [],
                // a check's entry is no posting line: nothing is written on it
                "written": null,
            }])
        );
    };
    check(0, "100.00", "-50.00", "50", Value::Null, false);
    check(1, "100.00", "0.004", "100.004", json!("0.01"), true);
    check(2, "100.00", "0.00", "100", Value::Null, true);
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

/// The processing order within one time: a plain balance written after a padding comes after it, and a
/// `balance ... with pad` after every balance entry of its time; the check of the parent account includes the
/// padding of its sub-account.
#[tokio::test]
async fn a_balance_written_after_a_padding_comes_after_it() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", include_str!("../fixtures/journals/review-padorder/main.zhang"))]);
    let ledger = shared_ledger(&scratch);
    let page = page(&ledger, None, None).await;
    let order = page["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| format!("{} {} {}", it["type"].as_str().unwrap(), it["narration"].as_str().unwrap(), it["passed"]))
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            "BalanceCheck Assets:Bank:Sub true",
            "BalanceCheck Assets:Cash true",
            // 100 of the salary and the 50 padded into the sub-account
            "BalanceCheck Assets:Bank true",
            "BalancePad pad Assets:Bank:Sub to Equity:Open null",
            "Transaction salary null",
        ]
    );
}

#[tokio::test]
async fn a_keyword_is_plain_text_that_matches_ignoring_case() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
    // payees (`Unbalanced` too), and the `Balance Check` of the assertions
    assert_eq!(
        summary(&page(&ledger, Some("BALANCE"), None).await),
        vec![
            "Transaction Unbalanced in another file",
            "BalanceCheck Balance Check Assets:Cash",
            "BalanceCheck Balance Check Assets:Cash",
            "BalanceCheck Balance Check Assets:Cash",
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open",
        ]
    );
    // accounts: the padding transaction posts to Equity:Open, and the balance ... with pad names it
    assert_eq!(
        summary(&page(&ledger, Some("equity:open"), None).await),
        vec![
            "BalanceCheck Balance Check Assets:Cash",
            "BalancePad Balance Pad pad Assets:Cash to Equity:Open"
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
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
    // the old journal divided by the size and panicked
    let (status, _) = journals(&ledger, request(Some(1), Some(0), None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // a page has at most 1000 rows, with a message that says so
    for size in [1001, u32::MAX] {
        let (status, body) = journals(&ledger, request(Some(u32::MAX), Some(size), None, None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["message"], "size must be between 1 and 1000");
    }
    let (status, body) = journals(&ledger, request(Some(1), Some(1000), None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["total_count"], 9);
    // the last page there can be, past the end
    let (status, body) = journals(&ledger, request(Some(u32::MAX), Some(1000), None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["records"], json!([]));
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

    // the error box reads its pages from `journals.errors`, which takes any size
    let (rows, total) = errors_page(&ledger, 1000, 0).await;
    assert_eq!((rows.len(), total), (2, 2));
}

const TAGGED: &str = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food

2024-03-02 * "Tagged" "with tags" #trip #Food ^z-link ^a-link
  Assets:Cash -4 CNY
  Expenses:Food 4 CNY
"#;

/// Tags and links keep their written order, which the query engine's sets do not, so that the edit form, which
/// sends them back as the journal lists them, does not reorder them when it saves.
#[tokio::test]
async fn tags_and_links_keep_their_written_order_through_a_save() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", TAGGED)]);
    let ledger = shared_ledger(&scratch);
    let item = page(&ledger, None, None).await["records"][0].clone();
    assert_eq!(item["tags"], json!(["trip", "Food"]));
    assert_eq!(item["links"], json!(["z-link", "a-link"]));
    // a search by tag finds it whatever the case of the needle
    assert_eq!(page(&ledger, Some("FOOD"), None).await["total_count"], 1);

    // save it back as the edit form does, with the tags and links as listed
    let posting = |it: &Value| json!({"account": it["account"], "unit": it["unit"], "metas": it["metas"]});
    let body = json!({
        "datetime": "2024-03-02T00:00:00Z",
        "payee": item["payee"],
        "narration": item["narration"],
        "flag": "Okay",
        "postings": item["postings"].as_array().unwrap().iter().map(posting).collect::<Vec<_>>(),
        "metas": item["metas"],
        "tags": item["tags"],
        "links": item["links"],
    });
    let payload = serde_json::from_value(body).unwrap();
    let (sender, _receiver) = tokio::sync::mpsc::channel(1);
    let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
    let id = item["id"].as_str().unwrap().to_owned();
    let (status, body) = respond(update_single_transaction(State(ledger.clone()), reload, UrlPath((id,)), Json(payload)).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let written = std::fs::read_to_string(scratch.main_file()).unwrap();
    assert!(written.contains("#trip #Food ^z-link ^a-link"), "{written}");

    // and the reloaded journal lists them in that order again
    let reloaded = shared_ledger(&scratch);
    let item = page(&reloaded, None, None).await["records"][0].clone();
    assert_eq!(item["tags"], json!(["trip", "Food"]));
    assert_eq!(item["links"], json!(["z-link", "a-link"]));
}

/// The new-transaction form reads `ledger.now` for its default date and time, and `journals.payees` and
/// `journals.accounts` at that instant for its suggestions (`retrieveNewTransactionInfo` in the frontend).
#[tokio::test]
async fn the_new_transaction_form_suggests_sorted_payees_without_pads_and_open_accounts() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
    // the ledger's time now, by the system clock here
    let (date, time) = ledger_now(&ledger).await;
    assert!(chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_ok(), "{date}");
    assert!(chrono::NaiveTime::parse_from_str(&time, "%H:%M:%S").is_ok(), "{time}");
    assert_eq!(
        names(&ledger, "journals.payees", json!({})).await,
        ["Broker", "Cafe", "Unbalanced", "zz-shop (a)"]
    );
    assert_eq!(
        names(&ledger, "journals.accounts", json!({ "date": date, "time": time })).await,
        ["Assets:Broker", "Assets:Cash", "Equity:Open", "Expenses:Food", "Income:Gains"]
    );
}

/// A document is the same row on the documents page (`journals.documents`) and on its account's page
/// (`accounts.documents`): the lists derive its file name, extension and preview from the row's `path`
/// (`documentOf` in the frontend, `utils/documents.test.ts`).
#[tokio::test]
async fn a_document_is_the_same_row_on_both_lists() {
    let ledger_text = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash

2024-01-01 document Assets:Cash "statements/Jan.PDF"
2024-01-02 document Assets:Cash "photos/receipt.webp"
2024-01-03 document Assets:Cash "notes/README"
"#;
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", ledger_text)]);
    let ledger = shared_ledger(&scratch);
    let (status, mut all) = builtin(&ledger, "journals.documents", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, mut of_account) = builtin(&ledger, "accounts.documents", json!({ "account": "Assets:Cash" })).await;
    assert_eq!(status, StatusCode::OK);
    let paths = |documents: &[Value]| {
        let mut paths = documents.iter().map(|it| it["path"].clone()).collect::<Vec<_>>();
        paths.sort_by_key(|it| it.to_string());
        paths
    };
    let expected = vec![json!("notes/README"), json!("photos/receipt.webp"), json!("statements/Jan.PDF")];
    assert_eq!(paths(&all), expected);
    assert_eq!(paths(&of_account), expected);
    // the very same rows
    all.sort_by_key(|it| it["path"].to_string());
    of_account.sort_by_key(|it| it["path"].to_string());
    assert_eq!(all, of_account);
    assert_eq!(
        all[0],
        json!({"date": "2024-01-03", "time": "00:00:00", "path": "notes/README", "account": "Assets:Cash", "transaction_id": null})
    );
}

/// The form offers the accounts open now, by the rule the ledger checks the transaction with: an account closed today
/// with only a date, and one opened again after its close, but not one closed before now or opened later.
#[tokio::test]
async fn the_new_transaction_form_offers_the_accounts_open_today() {
    let today = chrono::Utc::now().date_naive();
    let tomorrow = today.succ_opt().unwrap();
    let scratch = Scratch::with_files(
        "main.zhang",
        &[(
            "main.zhang",
            &format!(
                r#"option "timezone" "UTC"
1970-01-01 open Assets:Again
2000-01-01 close Assets:Again
2001-01-01 open Assets:Again
1970-01-01 open Assets:Gone
2000-01-01 close Assets:Gone
{tomorrow} open Assets:Later
1970-01-01 open Assets:Today
{today} close Assets:Today
1970-01-01 open Assets:Midnight
{today} 00:00:00 close Assets:Midnight
"#
            ),
        )],
    );
    let ledger = shared_ledger(&scratch);
    let (date, time) = ledger_now(&ledger).await;
    assert_eq!(
        names(&ledger, "journals.accounts", json!({ "date": date, "time": time })).await,
        ["Assets:Again", "Assets:Today"]
    );
}

/// The form asks for the accounts open at the transaction's date and time, which it submits as an instant read in the
/// ledger's timezone: an account closed with only a date is offered through its close day, one closed with a time
/// until that time, and an account is offered from its open on.
#[tokio::test]
async fn the_new_transaction_form_offers_the_accounts_open_at_the_date_of_the_transaction() {
    let scratch = Scratch::with_files(
        "main.zhang",
        &[(
            "main.zhang",
            r#"option "timezone" "Asia/Shanghai"
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Day
2024-01-05 close Assets:Day
1970-01-01 open Assets:Timed
2024-01-05 10:00:00 close Assets:Timed
2024-01-05 open Assets:New
"#,
        )],
    );
    let ledger = shared_ledger(&scratch);
    // the form sends the transaction's wall-clock date and time in the ledger's timezone, which the endpoint read from an
    // instant before; here as the ledger's wall-clock time of each instant
    let accounts = |at: &str| {
        let ledger = ledger.clone();
        let at = chrono::DateTime::parse_from_rfc3339(at)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Shanghai)
            .naive_local();
        async move {
            let params = json!({ "date": at.date().to_string(), "time": at.time().format("%H:%M:%S").to_string() });
            json!(names(&ledger, "journals.accounts", params).await)
        }
    };
    // 2024-01-04 23:00 in Shanghai
    assert_eq!(accounts("2024-01-04T15:00:00Z").await, json!(["Assets:Cash", "Assets:Day", "Assets:Timed"]));
    // 2024-01-05 09:30 and 10:00 in Shanghai: before and at the time of the timed close
    assert_eq!(
        accounts("2024-01-05T01:30:00Z").await,
        json!(["Assets:Cash", "Assets:Day", "Assets:New", "Assets:Timed"])
    );
    assert_eq!(
        accounts("2024-01-05T02:00:00Z").await,
        json!(["Assets:Cash", "Assets:Day", "Assets:New", "Assets:Timed"])
    );
    // 2024-01-05 15:00 in Shanghai: after the timed close, still on the close day
    assert_eq!(accounts("2024-01-05T07:00:00Z").await, json!(["Assets:Cash", "Assets:Day", "Assets:New"]));
    // 2024-01-06 00:30 in Shanghai, while it is still 2024-01-05 in UTC: the day after the close
    assert_eq!(accounts("2024-01-05T16:30:00Z").await, json!(["Assets:Cash", "Assets:New"]));
}

/// The document upload offers every account opened by now, closed ones included: a document only records, and may
/// follow the close, as in beancount. Not an account opened only later, one with postings but no `open`, or one closed
/// without ever being opened.
#[tokio::test]
async fn the_document_upload_offers_every_account_opened_by_now() {
    let tomorrow = chrono::Utc::now().date_naive().succ_opt().unwrap();
    let scratch = Scratch::with_files(
        "main.zhang",
        &[(
            "main.zhang",
            &format!(
                r#"option "timezone" "UTC"
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Gone
2000-01-01 close Assets:Gone
1970-01-01 open Assets:Again
2000-01-01 close Assets:Again
2001-01-01 open Assets:Again
2000-01-01 close Assets:NeverOpened
{tomorrow} open Assets:Later
2020-01-01 * "posted without an open"
  Assets:Cash -1 CNY
  Expenses:Ghost 1 CNY
"#
            ),
        )],
    );
    let ledger = shared_ledger(&scratch);
    let (date, time) = ledger_now(&ledger).await;
    let at = json!({ "date": date, "time": time });
    assert_eq!(
        names(&ledger, "accounts.opened", at.clone()).await,
        ["Assets:Again", "Assets:Cash", "Assets:Gone"]
    );
    // the transaction form, which books, offers the open ones only
    assert_eq!(names(&ledger, "journals.accounts", at).await, ["Assets:Again", "Assets:Cash"]);
}

/// `accounts.opened` is the rule the ledger checks a document, a note or a plain balance assertion with
/// (`AccountUse::Records`): an `open` at or before the instant, by date and then by time. Pinned against the core rule
/// on a ledger with timed opens and closes: an account opened later the same day, one closed at a time, one closed by a
/// date, one closed before it is opened, one reopened, one opened only tomorrow and one only ever closed.
#[tokio::test]
async fn accounts_opened_is_the_rule_the_ledger_checks_a_record_with() {
    const ACCOUNTS: [&str; 9] = [
        "Assets:Afternoon",
        "Assets:Cash",
        "Assets:ClosedAtTen",
        "Assets:ClosedBeforeOpen",
        "Assets:ClosedThatDay",
        "Assets:Morning",
        "Assets:NeverOpened",
        "Assets:Reopened",
        "Assets:Tomorrow",
    ];
    let scratch = Scratch::with_files(
        "main.zhang",
        &[(
            "main.zhang",
            r#"option "timezone" "UTC"
1970-01-01 open Assets:Cash
2024-01-05 09:00:00 open Assets:Morning
2024-01-05 15:00:00 open Assets:Afternoon
2024-01-01 open Assets:ClosedAtTen
2024-01-05 10:00:00 close Assets:ClosedAtTen
2024-01-01 open Assets:ClosedThatDay
2024-01-05 close Assets:ClosedThatDay
2024-01-01 close Assets:NeverOpened
2024-01-05 08:00:00 close Assets:ClosedBeforeOpen
2024-01-05 15:00:00 open Assets:ClosedBeforeOpen
2024-01-01 open Assets:Reopened
2024-01-02 close Assets:Reopened
2024-01-06 open Assets:Reopened
2024-01-06 open Assets:Tomorrow
"#,
        )],
    );
    let ledger = shared_ledger(&scratch);
    let instants = [
        "2024-01-04 12:00:00",
        "2024-01-05 08:00:00",
        "2024-01-05 08:30:00",
        "2024-01-05 09:00:00",
        "2024-01-05 10:00:00",
        "2024-01-05 10:00:01",
        "2024-01-05 12:00:00",
        "2024-01-05 15:00:00",
        "2024-01-05 23:59:59",
        "2024-01-06 00:00:00",
        "2024-01-07 00:00:00",
    ]
    .map(|at| chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S").unwrap());
    let by_the_rule = {
        let guard = ledger.read().await;
        instants.map(|at| {
            ACCOUNTS
                .into_iter()
                .filter(|name| {
                    guard
                        .account_reference_error(&name.parse::<Account>().unwrap(), at, AccountUse::Records)
                        .is_none()
                })
                .collect::<Vec<_>>()
        })
    };
    // the rule itself is not trivial on this ledger
    assert_eq!(
        by_the_rule[6],
        ["Assets:Cash", "Assets:ClosedAtTen", "Assets:ClosedThatDay", "Assets:Morning", "Assets:Reopened"]
    );
    for (at, expected) in instants.iter().zip(by_the_rule) {
        let params = json!({ "date": at.date().to_string(), "time": at.time().format("%H:%M:%S").to_string() });
        // every account of the ledger is a row of #accounts, so each one is judged
        assert_eq!(names(&ledger, "accounts.list", params.clone()).await, ACCOUNTS, "at {at}");
        assert_eq!(names(&ledger, "accounts.opened", params).await, expected, "at {at}");
    }
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
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", ledger_text)]);
    let ledger = shared_ledger(&scratch);
    let id = {
        let guard = ledger.read().await;
        guard.transactions()[0].0.to_string()
    };
    let (status, rows) = builtin(&ledger, "journals.documents", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let document = |date: &str, time: &str, path: &str, account: Value, transaction_id: Value| json!({ "date": date, "time": time, "path": path, "account": account, "transaction_id": transaction_id });
    assert_eq!(
        rows,
        vec![
            // the documents of a transaction in written order: its own, then its postings'
            document("2024-01-02", "09:00:00", "receipts/a.pdf", Value::Null, json!(id)),
            document("2024-01-02", "09:00:00", "receipts/b.png", Value::Null, json!(id)),
            // a posting's document belongs to the posting's account too
            document("2024-01-02", "09:00:00", "receipts/c.pdf", json!("Expenses:Food"), json!(id)),
            document("2024-01-01", "00:00:00", "statements/jan.pdf", json!("Assets:Cash"), Value::Null),
        ]
    );
}

/// In a beancount ledger a document directive names its file relative to the file it is in (#508): the documents
/// page lists the path within the ledger that zhang resolved, the one the download opens.
#[tokio::test]
async fn a_beancount_document_is_listed_with_the_path_the_download_opens() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extensions/beancount/tests/balance_assertions");
    let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
    let ledger = Ledger::load(dir, "document_paths.bean".to_owned(), source).unwrap();
    let ledger = SharedLedger(Arc::new(RwLock::new(ledger)));
    let (status, rows) = builtin(&ledger, "journals.documents", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let paths = rows
        .iter()
        .map(|it| (it["date"].clone(), it["time"].clone(), it["path"].clone()))
        .collect::<Vec<_>>();
    // the one in the included data/2024.bean is written "../attachments/statement.txt"
    assert_eq!(
        paths,
        vec![
            (json!("2024-01-03"), json!("00:00:00"), json!("document_paths/attachments/statement.txt")),
            (json!("2024-01-02"), json!("00:00:00"), json!("document_paths/attachments/statement.txt")),
        ]
    );
    let path = rows[0]["path"].as_str().unwrap().to_owned();
    let response = download_document(State(ledger.clone()), Base64Path(path)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let content = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&content).trim(), "the statement of January 2024");
}

/// A sale that booking splits across two lots names the document of its posting once: the documents page lists the
/// posting as written, not each booked leg (which share its metadata).
#[tokio::test]
async fn the_document_of_a_split_sale_is_listed_once() {
    let ledger_text = r#"option "operating_currency" "USD"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Broker
  booking_method: "FIFO"
1970-01-01 open Income:Gains

2024-01-01 * "Broker" "buy"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD

2024-01-02 * "Broker" "buy"
  Assets:Broker 10 AAPL {120 USD}
  Assets:Cash -1200 USD

2024-01-03 * "Broker" "sell"
  Assets:Broker -15 AAPL {} @ 150 USD
    document: "slips/sale.pdf"
  Assets:Cash 2250 USD
  Income:Gains
"#;
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", ledger_text)]);
    let ledger = shared_ledger(&scratch);
    {
        let guard = ledger.read().await;
        assert!(guard.errors.is_empty(), "{:?}", guard.errors);
        // the sale is booked against both lots
        let legs = zhang_query::execute(&guard, "SELECT count(*) FROM postings WHERE narration = 'sell' AND account = 'Assets:Broker'").unwrap();
        assert_eq!(legs.rows, vec![vec![zhang_query::Value::Int(2)]]);
    }
    let (status, rows) = builtin(&ledger, "journals.documents", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let paths = rows.iter().map(|it| (it["path"].clone(), it["account"].clone())).collect::<Vec<_>>();
    assert_eq!(paths, vec![(json!("slips/sale.pdf"), json!("Assets:Broker"))]);
}

#[tokio::test]
async fn errors_are_listed_by_file_then_position_one_page_at_a_time() {
    let scratch = Scratch::with_files("main.zhang", &[("main.zhang", LEDGER), ("more.zhang", MORE)]);
    let ledger = shared_ledger(&scratch);
    let (records, total) = errors_page(&ledger, 10, 0).await;
    assert_eq!(total, 2);
    // main.zhang before more.zhang, whatever the order zhang found them in
    assert_eq!(records[0]["kind"], "AccountBalanceCheckError");
    assert_eq!(records[0]["file"], "main.zhang");
    assert_eq!(records[0]["source"], "2024-01-06 balance Assets:Cash 50 CNY");
    assert_eq!(records[0]["metas"], json!([{"key": "account_name", "value": "Assets:Cash"}]));
    let start = LEDGER.find("2024-01-06 balance").unwrap();
    assert_eq!(records[0]["span_start"], start);
    assert_eq!(records[1]["kind"], "UnbalancedTransaction");
    assert_eq!(records[1]["file"], "more.zhang");
    assert_eq!(records[1]["span_start"], 0);
    let metas = records[1]["metas"].as_array().unwrap();
    assert!(metas.iter().any(|it| it["key"] == "txn_id" && it["value"].is_string()), "{metas:?}");

    // the second page of one: the error box counts 2 pages from the total
    let (records, total) = errors_page(&ledger, 1, 1).await;
    assert_eq!((total, records.len()), (2, 1));
    assert_eq!(records[0]["kind"], "UnbalancedTransaction");
}
