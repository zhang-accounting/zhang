//! Balance assertions through the server, written against the API contract:
//!
//! - an assertion moves no balance: accounts show the sum of their postings, and a pad is sized
//!   from it;
//! - `GET /api/journals` lists every assertion as a `BalanceCheck` item, in its place among the
//!   transactions, with the balance (`account_before`), the asserted amount (`account_after`),
//!   its tolerance, and `passed`, decided by the server within the tolerance;
//! - `GET /api/accounts/:account/journals` lists every assertion of the account as a row that adds
//!   nothing, with the true running balance, the asserted amount and `passed`;
//! - a transaction flagged `C` is an ordinary transaction.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::JournalRequest;
use zhang_server::routes::account::{get_account_info, get_account_journals};
use zhang_server::routes::transaction::get_journals;
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;

const LEDGER: &str = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Open
1970-01-01 open Income:Salary
1970-01-01 open Expenses:Food

2024-01-01 * "Employer" "salary"
  Assets:Bank 165 CNY
  Income:Salary

2024-01-02 balance Assets:Bank 200 CNY

2024-01-03 * "Shop" "lunch" #food
  Assets:Bank -10.004 CNY
  Expenses:Food

2024-01-04 balance Assets:Bank 155 ~ 0.01 CNY

2024-01-05 balance Assets:Bank 500 CNY with pad Equity:Open

2024-01-06 balance Assets:Bank 500 CNY
"#;

/// A ledger directory under the system temp dir, removed on drop.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(content: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("zhang-balance-assertions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zhang"), content).unwrap();
        Scratch { dir }
    }

    async fn state(&self) -> State<SharedLedger> {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(self.dir.clone(), "main.zhang".to_owned(), source)
            .await
            .expect("the ledger loads");
        State(SharedLedger(Arc::new(RwLock::new(ledger))))
    }

    /// The body of `GET /api/journals` with these parameters.
    async fn journals(&self, page: Option<u32>, size: Option<u32>, keyword: Option<&str>, tags: Option<&[&str]>) -> Value {
        let request = JournalRequest {
            page,
            size,
            keyword: keyword.map(str::to_owned),
            tags: tags.map(|tags| tags.iter().map(|it| (*it).to_owned()).collect()),
            links: None,
        };
        let (status, body) = respond(get_journals(self.state().await, UrlQuery(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"].clone()
    }

    async fn account_journals(&self, account: &str) -> Vec<Value> {
        let (status, body) = respond(get_account_journals(self.state().await, UrlPath((account.to_owned(),))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"].as_array().unwrap().clone()
    }

    async fn balance(&self, account: &str) -> Value {
        let (status, body) = respond(get_account_info(self.state().await, UrlPath((account.to_owned(),))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"]["amount"]["detail"].clone()
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
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// a decimal string of the response as a number, so `35` and `35.000` compare equal
fn number(value: &Value) -> BigDecimal {
    value.as_str().unwrap_or_else(|| panic!("{value} is a decimal string")).parse().unwrap()
}

fn numbers(values: &[&Value]) -> Vec<BigDecimal> {
    values.iter().map(|it| number(it)).collect()
}

fn decimal(number: &str) -> BigDecimal {
    number.parse().unwrap()
}

#[tokio::test]
async fn balances_are_the_sums_of_the_postings_and_the_pad_starts_from_them() {
    let scratch = Scratch::new(LEDGER);
    // 165 - 10.004, then padded to 500: the failing assertion of 200 moved nothing
    assert_eq!(scratch.balance("Assets:Bank").await, json!({"CNY": "500.000"}));
    assert_eq!(scratch.balance("Equity:Open").await, json!({"CNY": "-345.004"}));
}

#[tokio::test]
async fn the_journal_lists_each_assertion_with_its_balance_and_whether_it_passed() {
    let scratch = Scratch::new(LEDGER);
    let journals = scratch.journals(None, None, None, None).await;
    let records = journals["records"].as_array().unwrap();
    let kinds = records.iter().map(|it| it["type"].as_str().unwrap()).collect::<Vec<_>>();
    // newest first; the assertions keep their place among the transactions
    assert_eq!(
        kinds,
        vec!["BalanceCheck", "BalancePad", "BalanceCheck", "Transaction", "BalanceCheck", "Transaction"]
    );
    assert_eq!(journals["total_count"], 6);

    let checks = records.iter().filter(|it| it["type"] == "BalanceCheck").collect::<Vec<_>>();
    for check in &checks {
        assert_eq!(check["payee"], "Balance Check");
        assert_eq!(check["narration"], "Assets:Bank");
        assert_eq!(check["type_"], "C");
        assert_eq!(check["postings"].as_array().unwrap().len(), 1);
    }
    let posting = |check: &Value, field: &str| check["postings"][0][field]["number"].clone();
    // (balance, asserted, asserted - balance, passed), newest first
    let described = checks
        .iter()
        .map(|check| {
            (
                number(&posting(check, "account_before")),
                number(&posting(check, "account_after")),
                number(&posting(check, "inferred_unit")),
                check["passed"].as_bool().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        described,
        vec![
            (decimal("500"), decimal("500"), decimal("0"), true),
            // within its tolerance: passes, though the balance is not the asserted amount
            (decimal("154.996"), decimal("155"), decimal("0.004"), true),
            (decimal("165"), decimal("200"), decimal("35"), false),
        ]
    );
    let tolerances = checks.iter().map(|check| check["tolerance"].clone()).collect::<Vec<_>>();
    assert_eq!(tolerances, vec![Value::Null, json!("0.01"), Value::Null]);

    // the pad is sized from the true balance, 154.996
    let pad = records.iter().find(|it| it["type"] == "BalancePad").unwrap();
    assert_eq!(
        numbers(&[&pad["postings"][0]["inferred_unit"]["number"], &pad["postings"][0]["account_after"]["number"]]),
        vec![decimal("345.004"), decimal("500")]
    );
}

#[tokio::test]
async fn assertions_take_part_in_paging_and_search_like_transactions() {
    let scratch = Scratch::new(LEDGER);
    let page = scratch.journals(Some(2), Some(4), None, None).await;
    assert_eq!(page["total_count"], 6);
    let kinds = page["records"].as_array().unwrap().iter().map(|it| it["type"].clone()).collect::<Vec<_>>();
    assert_eq!(kinds, vec![json!("BalanceCheck"), json!("Transaction")]);

    // listed under the payee `Balance Check` and their account
    let found = scratch.journals(None, None, Some("balance check"), None).await;
    assert_eq!(found["total_count"], 3);
    let found = scratch.journals(None, None, Some("assets:bank"), None).await;
    assert_eq!(found["total_count"], 6);
    // an assertion has no tags
    let tagged = scratch.journals(None, None, None, Some(&["food"])).await;
    let kinds = tagged["records"].as_array().unwrap().iter().map(|it| it["type"].clone()).collect::<Vec<_>>();
    assert_eq!(kinds, vec![json!("Transaction")]);
}

#[tokio::test]
async fn the_account_journal_shows_assertions_with_the_true_running_balance() {
    let scratch = Scratch::new(LEDGER);
    let rows = scratch.account_journals("Assets:Bank").await;
    // (payee, change, balance after, asserted, passed), newest first
    let described = rows
        .iter()
        .map(|row| {
            (
                row["payee"].as_str().unwrap().to_owned(),
                number(&row["inferred_unit"]["number"]),
                number(&row["account_after"]["number"]),
                row["asserted"].get("number").map(number),
                row["passed"].as_bool(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        described,
        vec![
            ("Balance Check".to_owned(), decimal("0"), decimal("500"), Some(decimal("500")), Some(true)),
            ("Balance Pad".to_owned(), decimal("345.004"), decimal("500"), None, None),
            ("Balance Check".to_owned(), decimal("0"), decimal("154.996"), Some(decimal("155")), Some(true)),
            ("Shop".to_owned(), decimal("-10.004"), decimal("154.996"), None, None),
            // fails, and changes nothing
            ("Balance Check".to_owned(), decimal("0"), decimal("165"), Some(decimal("200")), Some(false)),
            ("Employer".to_owned(), decimal("165"), decimal("165"), None, None),
        ]
    );
}

#[tokio::test]
async fn an_assertion_on_a_parent_account_is_checked_against_its_sub_accounts_too() {
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Equity:Open
2024-01-01 * "Self" "opening"
  Assets:Bank:Checking 60 CNY
  Assets:Bank:Savings 40 CNY
  Equity:Open
2024-01-02 balance Assets:Bank 100 CNY
2024-01-03 balance Assets:Bank 150 CNY with pad Equity:Open
"#,
    );
    let journals = scratch.journals(None, None, None, None).await;
    let records = journals["records"].as_array().unwrap();
    let check = records.iter().find(|it| it["type"] == "BalanceCheck").unwrap();
    assert_eq!(check["passed"], true);
    assert_eq!(number(&check["postings"][0]["account_before"]["number"]), decimal("100"));
    // the pad books what the sub-accounts lack to the parent account itself
    let pad = records.iter().find(|it| it["type"] == "BalancePad").unwrap();
    assert_eq!(pad["postings"][0]["account"], "Assets:Bank");
    assert_eq!(number(&pad["postings"][0]["inferred_unit"]["number"]), decimal("50"));

    // the assertion row shows the balance it was checked against; the posting rows show the
    // parent account's own postings
    let rows = scratch.account_journals("Assets:Bank").await;
    let described = rows
        .iter()
        .map(|row| {
            (
                row["payee"].as_str().unwrap().to_owned(),
                number(&row["account_after"]["number"]),
                row["passed"].as_bool(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        described,
        vec![
            ("Balance Pad".to_owned(), decimal("50"), None),
            ("Balance Check".to_owned(), decimal("100"), Some(true)),
        ]
    );
}

#[tokio::test]
async fn a_c_flagged_transaction_is_an_ordinary_transaction() {
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Conversions
2024-01-01 C "Conversion" ""
  Assets:Bank 10 CNY
  Equity:Conversions
"#,
    );
    let journals = scratch.journals(None, None, None, None).await;
    let record = &journals["records"][0];
    assert_eq!(record["type"], "Transaction");
    assert_eq!(record["flag"], "C");
    assert_eq!(record["is_balanced"], true);
    assert_eq!(scratch.balance("Assets:Bank").await, json!({"CNY": "10"}));
}
