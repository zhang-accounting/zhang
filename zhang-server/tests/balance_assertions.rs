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
use zhang_server::request::{BatchAccountBalanceRequest, JournalRequest};
use zhang_server::routes::account::{create_batch_account_balances, get_account_info, get_account_journals, get_account_list};
use zhang_server::routes::transaction::get_journals;
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::{SharedLedger, SharedReloadSender};
use zhang_server::ReloadSender;

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
    /// the main file: `main.zhang`, or `main.bean` for a beancount ledger
    main: &'static str,
}

impl Scratch {
    fn new(content: &str) -> Scratch {
        Scratch::with_main("main.zhang", content)
    }

    fn beancount(content: &str) -> Scratch {
        Scratch::with_main("main.bean", content)
    }

    fn with_main(main: &'static str, content: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("zhang-balance-assertions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(main), content).unwrap();
        Scratch { dir, main }
    }

    /// the ledger loaded from the files as they are now
    async fn ledger(&self) -> Ledger {
        let source: Arc<dyn zhang_core::data_source::DataSource> = if self.main.ends_with(".bean") {
            Arc::new(LocalFileSystemDataSource::new(beancount::Beancount::default()))
        } else {
            Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
        };
        Ledger::async_load(self.dir.clone(), self.main.to_owned(), source)
            .await
            .expect("the ledger loads")
    }

    async fn state(&self) -> State<SharedLedger> {
        State(SharedLedger(Arc::new(RwLock::new(self.ledger().await))))
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
        let (status, body) = respond(get_account_journals(self.state().await, UrlPath((account.to_owned(),)), UrlQuery(Default::default())).await).await;
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
    // a write answers with no body
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
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
    // newest first; the assertions keep their place among the transactions, the `balance ... with pad` checked
    // after its padding
    assert_eq!(
        kinds,
        vec![
            "BalanceCheck",
            "BalanceCheck",
            "BalancePad",
            "BalanceCheck",
            "Transaction",
            "BalanceCheck",
            "Transaction"
        ]
    );
    assert_eq!(journals["total_count"], 7);

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
            // the `balance ... with pad`, after its padding
            (decimal("500"), decimal("500"), decimal("0"), true),
            // within its tolerance: passes, though the balance is not the asserted amount
            (decimal("154.996"), decimal("155"), decimal("0.004"), true),
            (decimal("165"), decimal("200"), decimal("35"), false),
        ]
    );
    let tolerances = checks.iter().map(|check| check["tolerance"].clone()).collect::<Vec<_>>();
    assert_eq!(tolerances, vec![Value::Null, Value::Null, json!("0.01"), Value::Null]);

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
    assert_eq!(page["total_count"], 7);
    let kinds = page["records"].as_array().unwrap().iter().map(|it| it["type"].clone()).collect::<Vec<_>>();
    assert_eq!(kinds, vec![json!("Transaction"), json!("BalanceCheck"), json!("Transaction")]);

    // listed under the payee `Balance Check` and their account
    let found = scratch.journals(None, None, Some("balance check"), None).await;
    assert_eq!(found["total_count"], 4);
    let found = scratch.journals(None, None, Some("assets:bank"), None).await;
    assert_eq!(found["total_count"], 7);
    // an assertion has no tags
    let tagged = scratch.journals(None, None, None, Some(&["food"])).await;
    let kinds = tagged["records"].as_array().unwrap().iter().map(|it| it["type"].clone()).collect::<Vec<_>>();
    assert_eq!(kinds, vec![json!("Transaction")]);
}

#[tokio::test]
async fn the_account_journal_shows_assertions_with_the_true_running_balance() {
    let scratch = Scratch::new(LEDGER);
    let rows = scratch.account_journals("Assets:Bank").await;
    // (payee, change, balance after, asserted, checked against, passed), newest first
    let described = rows.iter().map(described_row).collect::<Vec<_>>();
    assert_eq!(
        described,
        vec![
            row("Balance Check", "0", "500", Some(("500", "500", true))),
            // the `balance ... with pad`, after its padding
            row("Balance Check", "0", "500", Some(("500", "500", true))),
            row("Balance Pad", "345.004", "500", None),
            row("Balance Check", "0", "154.996", Some(("155", "154.996", true))),
            row("Shop", "-10.004", "154.996", None),
            // fails, and changes nothing
            row("Balance Check", "0", "165", Some(("200", "165", false))),
            row("Employer", "165", "165", None),
        ]
    );
}

/// (payee, change, running balance after, (asserted, checked against, passed) of an assertion row)
type DescribedRow = (String, BigDecimal, BigDecimal, Option<(BigDecimal, BigDecimal, bool)>);

fn described_row(row: &Value) -> DescribedRow {
    let assertion = (!row["asserted"].is_null()).then(|| {
        (
            number(&row["asserted"]["number"]),
            number(&row["checked_balance"]["number"]),
            row["passed"].as_bool().unwrap(),
        )
    });
    if assertion.is_none() {
        assert!(row["checked_balance"].is_null() && row["passed"].is_null(), "{row}");
    }
    (
        row["payee"].as_str().unwrap().to_owned(),
        number(&row["inferred_unit"]["number"]),
        number(&row["account_after"]["number"]),
        assertion,
    )
}

fn row(payee: &str, change: &str, after: &str, assertion: Option<(&str, &str, bool)>) -> DescribedRow {
    (
        payee.to_owned(),
        decimal(change),
        decimal(after),
        assertion.map(|(asserted, checked, passed)| (decimal(asserted), decimal(checked), passed)),
    )
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
    let check = records
        .iter()
        .find(|it| it["type"] == "BalanceCheck" && it["datetime"] == "2024-01-02T00:00:00")
        .unwrap();
    assert_eq!(check["passed"], true);
    assert_eq!(number(&check["postings"][0]["account_before"]["number"]), decimal("100"));
    // the pad books what the sub-accounts lack to the parent account itself
    let pad = records.iter().find(|it| it["type"] == "BalancePad").unwrap();
    assert_eq!(pad["postings"][0]["account"], "Assets:Bank");
    assert_eq!(number(&pad["postings"][0]["inferred_unit"]["number"]), decimal("50"));

    // a parent account's journal is that of its subtree: every row's balance is the running balance with the
    // sub-accounts, which is the balance an assertion on the account is checked against
    let rows = scratch.account_journals("Assets:Bank").await;
    assert_eq!(
        rows.iter().map(described_row).collect::<Vec<_>>(),
        vec![
            // the `balance ... with pad`, after its padding
            row("Balance Check", "0", "150", Some(("150", "150", true))),
            row("Balance Pad", "50", "150", None),
            row("Balance Check", "0", "100", Some(("100", "100", true))),
            row("Self", "40", "100", None),
            row("Self", "60", "60", None),
        ]
    );
    let accounts = rows.iter().map(|row| row["account"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(
        accounts,
        vec!["Assets:Bank", "Assets:Bank", "Assets:Bank", "Assets:Bank:Savings", "Assets:Bank:Checking"]
    );
}

/// a parent account with postings of its own, two sub-accounts and a sibling that is no sub-account
const PARENT: &str = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Assets:Banking
1970-01-01 open Equity:Open
2024-01-01 * "Self" "opening"
  Assets:Bank 5 CNY
  Assets:Bank:Checking 60 CNY
  Assets:Bank:Savings 40 CNY
  Assets:Bank:Savings 7 USD
  Assets:Banking 1000 CNY
  Equity:Open -1105 CNY
  Equity:Open -7 USD
2024-01-02 balance Assets:Bank:Checking 60 CNY
2024-01-02 balance Assets:Banking 1000 CNY
2024-01-03 balance Assets:Bank 105 CNY
"#;

#[tokio::test]
async fn an_account_shows_the_balance_its_assertions_are_checked_against() {
    let scratch = Scratch::new(PARENT);
    let (status, info) = respond(get_account_info(scratch.state().await, UrlPath(("Assets:Bank".to_owned(),))).await).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    // its own balance, and the balance with the sub-accounts, which a `balance` on it checks
    assert_eq!(info["data"]["amount"]["detail"], json!({"CNY": "5"}));
    assert_eq!(info["data"]["balance_with_sub_accounts"], json!({"CNY": "105", "USD": "7"}));
    assert_eq!(info["data"]["has_sub_accounts"], true);

    let (status, list) = respond(get_account_list(scratch.state().await).await).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let listed = |name: &str| list["data"].as_array().unwrap().iter().find(|it| it["name"] == name).unwrap().clone();
    assert_eq!(listed("Assets:Bank")["balance_with_sub_accounts"], json!({"CNY": "105", "USD": "7"}));
    assert_eq!(listed("Assets:Bank")["has_sub_accounts"], true);
    // `Assets:Banking` is no sub-account of `Assets:Bank`, and has none
    assert_eq!(listed("Assets:Banking")["balance_with_sub_accounts"], json!({"CNY": "1000"}));
    assert_eq!(listed("Assets:Banking")["has_sub_accounts"], false);
    assert_eq!(listed("Assets:Bank:Savings")["balance_with_sub_accounts"], json!({"CNY": "40", "USD": "7"}));
}

#[tokio::test]
async fn the_account_journal_lists_the_assertions_on_the_account_itself_only() {
    let scratch = Scratch::new(PARENT);
    // not those on its sub-account or on `Assets:Banking`; the postings are those of its subtree
    let rows = scratch.account_journals("Assets:Bank").await;
    assert_eq!(
        rows.iter().map(described_row).collect::<Vec<_>>(),
        vec![
            row("Balance Check", "0", "105", Some(("105", "105", true))),
            row("Self", "7", "7", None),
            row("Self", "40", "105", None),
            row("Self", "60", "65", None),
            row("Self", "5", "5", None),
        ]
    );
    let rows = scratch.account_journals("Assets:Bank:Checking").await;
    assert_eq!(
        rows.iter().map(described_row).collect::<Vec<_>>(),
        vec![row("Balance Check", "0", "60", Some(("60", "60", true))), row("Self", "60", "60", None),]
    );
}

#[tokio::test]
async fn the_account_journal_keeps_the_order_of_a_day() {
    // a day's balance entries come in ledger order before its transactions: two checks, a pad and its
    // padding, and a check after it, then the day's transaction
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
2024-01-01 * "Self" "opening"
  Assets:Bank 100 CNY
  Equity:Open
2024-01-05 * "Shop" "lunch"
  Assets:Bank -10 CNY
  Expenses:Food
2024-01-05 balance Assets:Bank 100 CNY
2024-01-05 balance Assets:Bank 90 CNY
2024-01-05 balance Assets:Bank 150 CNY with pad Equity:Open
2024-01-05 balance Assets:Bank 150 CNY
"#,
    );
    let rows = scratch.account_journals("Assets:Bank").await;
    assert_eq!(
        rows.iter().map(described_row).collect::<Vec<_>>(),
        vec![
            row("Shop", "-10", "140", None),
            // the `balance ... with pad`, checked after the balance entries of its time
            row("Balance Check", "0", "150", Some(("150", "150", true))),
            row("Balance Check", "0", "150", Some(("150", "150", true))),
            row("Balance Pad", "50", "150", None),
            row("Balance Check", "0", "100", Some(("90", "100", false))),
            row("Balance Check", "0", "100", Some(("100", "100", true))),
            row("Self", "100", "100", None),
        ]
    );
}

#[tokio::test]
async fn the_padding_of_a_pad_is_listed_on_the_date_of_the_pad() {
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
2024-01-01 pad Assets:Bank Equity:Open
2024-01-10 * "Shop" "lunch"
  Assets:Bank -30 CNY
  Expenses:Food
2024-02-01 balance Assets:Bank 70 CNY
"#,
    );
    let journals = scratch.journals(None, None, None, None).await;
    let records = journals["records"].as_array().unwrap();
    let described = records
        .iter()
        .map(|it| (it["type"].as_str().unwrap().to_owned(), it["datetime"].as_str().unwrap().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        described,
        vec![
            ("BalanceCheck".to_owned(), "2024-02-01T00:00:00".to_owned()),
            ("Transaction".to_owned(), "2024-01-10T00:00:00".to_owned()),
            ("BalancePad".to_owned(), "2024-01-01T00:00:00".to_owned()),
        ]
    );
    let pad = &records[2];
    assert_eq!(number(&pad["postings"][0]["inferred_unit"]["number"]), decimal("100"));
    assert_eq!(records[0]["passed"], true);

    // the running balance includes the padding from the date of the pad
    let rows = scratch.account_journals("Assets:Bank").await;
    let balances = rows.iter().map(|row| number(&row["account_after"]["number"])).collect::<Vec<_>>();
    assert_eq!(balances, vec![decimal("70"), decimal("70"), decimal("100")]);
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

#[tokio::test]
async fn a_new_account_gets_a_row_in_the_operating_currency() {
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Empty
1970-01-01 open Liabilities:Card USD
"#,
    );
    let (_, info) = respond(get_account_info(scratch.state().await, UrlPath(("Assets:Empty".to_owned(),))).await).await;
    // like the account's own balance, its balance with sub-accounts holds the operating currency, at zero
    assert_eq!(info["data"]["amount"]["detail"], json!({"CNY": "0"}));
    assert_eq!(info["data"]["balance_with_sub_accounts"], json!({"CNY": "0"}));
    assert_eq!(info["data"]["has_sub_accounts"], false);
    let (_, list) = respond(get_account_list(scratch.state().await).await).await;
    let card = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["name"] == "Liabilities:Card")
        .unwrap()
        .clone();
    assert_eq!(card["balance_with_sub_accounts"], json!({"CNY": "0"}));
}

#[tokio::test]
async fn a_batch_pads_sub_accounts_before_their_parents() {
    let today = chrono::Utc::now().date_naive();
    let data_file = format!("data/{}/{:02}.zhang", chrono::Datelike::year(&today), chrono::Datelike::month(&today));
    let scratch = Scratch::new(&format!(
        r#"option "operating_currency" "CNY"
option "timezone" "UTC"
include "{data_file}"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Equity:Open
2024-01-02 * "Self" "init"
  Assets:Bank 305 CNY
  Assets:Bank:Checking 155 CNY
  Assets:Bank:Savings 40 CNY
  Equity:Open
"#
    ));
    let data = scratch.dir.join(&data_file);
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::write(&data, "").unwrap();

    // the parent first, as a user may fill the form
    let cny = |number: u32| zhang_ast::amount::Amount::new(BigDecimal::from(number), "CNY");
    let batch = vec![
        BatchAccountBalanceRequest::Pad {
            account_name: "Assets:Bank".to_owned(),
            amount: cny(500),
            pad: "Equity:Open".to_owned(),
        },
        BatchAccountBalanceRequest::Pad {
            account_name: "Assets:Bank:Checking".to_owned(),
            amount: cny(200),
            pad: "Equity:Open".to_owned(),
        },
    ];
    let (sender, _) = tokio::sync::mpsc::channel(1);
    let reload = State(SharedReloadSender(Arc::new(ReloadSender::new(sender))));
    let response = create_batch_account_balances(scratch.state().await, reload, axum::Json(batch))
        .await
        .into_response();
    assert!(response.status().is_success(), "{}", response.status());

    // the sub-account is written first, so the parent's assertion covers its padding
    let written = std::fs::read_to_string(&data).unwrap();
    let checking = written.find("Assets:Bank:Checking 200 CNY").expect("the sub-account's balance is written");
    let bank = written.find("Assets:Bank 500 CNY").expect("the parent's balance is written");
    assert!(checking < bank, "{written}");
    let state = scratch.state().await;
    {
        let ledger = state.read().await;
        let store = ledger.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert!(store.balance_assertions.iter().all(|it| it.passed));
    }
    let (_, info) = respond(get_account_info(state, UrlPath(("Assets:Bank".to_owned(),))).await).await;
    assert_eq!(number(&info["data"]["balance_with_sub_accounts"]["CNY"]), decimal("500"));
}

#[tokio::test]
async fn an_opened_sub_account_without_postings_counts_as_a_sub_account() {
    let scratch = Scratch::new(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Equity:Open
2024-01-02 * "Self" "init"
  Assets:Bank 5 CNY
  Equity:Open
"#,
    );
    let (_, info) = respond(get_account_info(scratch.state().await, UrlPath(("Assets:Bank".to_owned(),))).await).await;
    assert_eq!(info["data"]["has_sub_accounts"], true);
    assert_eq!(info["data"]["balance_with_sub_accounts"], json!({"CNY": "5"}));
    let (_, list) = respond(get_account_list(scratch.state().await).await).await;
    let bank = list["data"].as_array().unwrap().iter().find(|it| it["name"] == "Assets:Bank").unwrap().clone();
    assert_eq!(bank["has_sub_accounts"], true);
    let checking = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["name"] == "Assets:Bank:Checking")
        .unwrap()
        .clone();
    assert_eq!(checking["has_sub_accounts"], false);
}

/// A beancount ledger reconciled through the API, day after day, as the UI does it. Beancount knows no times: it checks a
/// `balance` at the start of its date, before the transactions of that day. So the UI writes "my balance now" as a
/// `balance` dated tomorrow, and the difference a pad books as a padding transaction (flag `P`) dated now. It writes no
/// `pad`, which would pad the next balance of every commodity and absorb transactions added later.
mod beancount_pads {
    use axum::extract::{Path as UrlPath, State};
    use axum::http::StatusCode;
    use axum::Json;
    use bigdecimal::BigDecimal;
    use chrono::{Days, NaiveDate};
    use serde_json::{json, Value};
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_server::request::{AccountBalanceRequest, BatchAccountBalanceRequest};
    use zhang_server::routes::account::{create_account_balance, create_batch_account_balances};
    use zhang_server::routes::transaction::update_single_transaction;
    use zhang_server::state::{SharedLedger, SharedReloadSender};
    use zhang_server::ReloadSender;

    use super::{decimal, respond, Scratch};

    fn today() -> NaiveDate {
        // the ledgers below are in UTC
        chrono::Utc::now().date_naive()
    }

    fn days_ago(days: u64) -> NaiveDate {
        today().checked_sub_days(Days::new(days)).unwrap()
    }

    fn tomorrow() -> NaiveDate {
        today().checked_add_days(Days::new(1)).unwrap()
    }

    async fn check(scratch: &Scratch, account: &str, amount: Amount) -> (StatusCode, Value) {
        check_with(scratch.state().await, account, amount).await
    }

    /// a check written to `ledger`, loaded before
    async fn check_with(ledger: State<SharedLedger>, account: &str, amount: Amount) -> (StatusCode, Value) {
        let request = AccountBalanceRequest::Check { amount };
        respond(create_account_balance(ledger, reload(), UrlPath((account.to_owned(),)), Json(request)).await).await
    }

    /// a batch of rows (account, amount, account padded from, or "" for a plain balance)
    async fn rows(scratch: &Scratch, rows: Vec<(&str, Amount, &str)>) -> (StatusCode, Value) {
        let rows = rows
            .into_iter()
            .map(|(account, amount, from)| match from {
                "" => BatchAccountBalanceRequest::Check {
                    account_name: account.to_owned(),
                    amount,
                },
                from => BatchAccountBalanceRequest::Pad {
                    account_name: account.to_owned(),
                    amount,
                    pad: from.to_owned(),
                },
            })
            .collect();
        respond(create_batch_account_balances(scratch.state().await, reload(), Json(rows)).await).await
    }

    /// assert the request was refused with a message holding every one of `parts`, and nothing was written
    fn refused(scratch: &Scratch, before: &str, (status, body): (StatusCode, Value), parts: &[&str]) {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let message = body["message"].as_str().unwrap();
        for part in parts {
            assert!(message.contains(part), "{part:?} in {message}");
        }
        assert_eq!(written(scratch), before, "nothing is written");
    }

    fn reload() -> State<SharedReloadSender> {
        let (sender, _) = tokio::sync::mpsc::channel(1);
        State(SharedReloadSender(std::sync::Arc::new(ReloadSender::new(sender))))
    }

    fn amount(number: u32, commodity: &str) -> Amount {
        Amount::new(BigDecimal::from(number), commodity)
    }

    async fn pad(scratch: &Scratch, account: &str, amount: Amount, from: &str) -> (StatusCode, Value) {
        pad_with(scratch.state().await, account, amount, from).await
    }

    /// a reconcile written to `ledger`, loaded before
    async fn pad_with(ledger: State<SharedLedger>, account: &str, amount: Amount, from: &str) -> (StatusCode, Value) {
        let request = AccountBalanceRequest::Pad { amount, pad: from.to_owned() };
        respond(create_account_balance(ledger, reload(), UrlPath((account.to_owned(),)), Json(request)).await).await
    }

    /// what the ledger reloaded from its files holds: the errors, the paddings (date, units, account padded from) and
    /// whether each assertion passed
    async fn reloaded(scratch: &Scratch) -> (Vec<ErrorKind>, Vec<String>, Vec<bool>) {
        let ledger = scratch.ledger().await;
        let store = ledger.store.read().unwrap();
        let errors = store.errors.iter().map(|it| it.error_type.clone()).collect();
        let mut paddings = store
            .transactions
            .values()
            .filter(|it| it.flag == zhang_ast::Flag::BalancePad)
            .map(|it| {
                format!(
                    "{} {} from {}",
                    it.datetime.date_naive(),
                    it.postings[0].inferred_amount,
                    it.postings[1].account.name()
                )
            })
            .collect::<Vec<_>>();
        paddings.sort();
        let passed = store.balance_assertions.iter().map(|it| it.passed).collect();
        (errors, paddings, passed)
    }

    fn written(scratch: &Scratch) -> String {
        walkdir(&scratch.dir)
            .into_iter()
            .filter(|it| it != &scratch.dir.join(scratch.main))
            .map(|it| std::fs::read_to_string(it).unwrap())
            .collect()
    }

    fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = vec![];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else {
                files.push(path);
            }
        }
        files
    }

    const OPENS: &str = r#"option "operating_currency" "CNY"
option "timezone" "UTC"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:A
1970-01-01 open Equity:Open
1970-01-01 open Equity:Fx
1970-01-01 open Expenses:Food
"#;

    /// the `pad` directives of the written files
    fn pad_directives(scratch: &Scratch) -> Vec<String> {
        written(scratch)
            .lines()
            .filter(|line| line.split_whitespace().nth(1) == Some("pad"))
            .map(str::to_owned)
            .collect()
    }

    /// add `text` to the end of the main file, as a user editing it
    fn append(scratch: &Scratch, text: &str) {
        let main = scratch.dir.join(scratch.main);
        let content = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{content}\n{text}")).unwrap();
    }

    #[tokio::test]
    async fn a_reconcile_books_the_difference_now_and_checks_tomorrow() {
        // yesterday's pad from the old UI, then dinner; today's reconcile books the 420 dinner left missing
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} pad Assets:A Equity:Open
{} balance Assets:A 100 CNY
{} * "dinner"
  Assets:A -20 CNY
  Expenses:Food
  time: "20:00:00"
"#,
            days_ago(2),
            days_ago(1),
            days_ago(1)
        ));
        let (status, body) = pad(&scratch, "Assets:A", amount(500, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        let files = written(&scratch);
        assert!(pad_directives(&scratch).is_empty(), "{files}");
        assert!(
            files.contains(&format!("{} P \"Balance Pad\" \"pad Assets:A to Equity:Open\"", today())),
            "{files}"
        );
        assert!(files.contains("Assets:A 420 CNY") && files.contains("Equity:Open -420 CNY"), "{files}");
        assert!(files.contains(&format!("{} balance Assets:A 500 CNY", tomorrow())), "{files}");
        // shown as a pad, and not counted as a transaction of the user
        let journal = scratch.journals(None, None, None, None).await;
        let pads = journal["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["type"] == "BalancePad" && it["narration"] == "pad Assets:A to Equity:Open")
            .count();
        assert_eq!(pads, 2, "{journal}");
        let request = zhang_server::request::StatisticRequest {
            from: days_ago(30).to_string(),
            to: tomorrow().checked_add_days(Days::new(30)).unwrap().to_string(),
        };
        let (status, summary) =
            respond(zhang_server::routes::statistics::get_statistic_summary(scratch.state().await, axum::extract::Query(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{summary}");
        assert_eq!(summary["data"]["transaction_number"], json!(1), "the dinner only");

        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            paddings,
            vec![
                format!("{} 100 CNY from Equity:Open", days_ago(2)),
                format!("{} 420 CNY from Equity:Open", today())
            ]
        );
        assert_eq!(passed, vec![true, true]);
        assert_eq!(scratch.balance("Assets:A").await, json!({"CNY": "500"}));
    }

    #[tokio::test]
    async fn a_transaction_added_after_a_reconcile_makes_it_fail_instead_of_being_padded() {
        // the account holds 10 USD; reconciled at 100 CNY, then dinner and coffee are added today
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "seed usd"
  Assets:A 10 USD
  Equity:Fx
"#,
            days_ago(30)
        ));
        let (status, body) = pad(&scratch, "Assets:A", amount(100, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        append(
            &scratch,
            &format!(
                r#"{today} * "Shop" "dinner"
  Expenses:Food 20 CNY
  Assets:A
{today} * "Shop" "coffee"
  Expenses:Food 3 USD
  Assets:A
"#,
                today = today()
            ),
        );
        let (errors, paddings, passed) = reloaded(&scratch).await;
        // the padding is the 100 asked for, no more; tomorrow's balance fails by the dinner, and no balance of USD is
        // written, which the coffee leaves at 7
        assert_eq!(paddings, vec![format!("{} 100 CNY from Equity:Open", today())]);
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(passed, vec![false]);
        assert_eq!(scratch.balance("Assets:A").await, json!({"CNY": "80", "USD": "7"}));
    }

    #[tokio::test]
    async fn a_reconcile_is_checked_after_the_transactions_of_today() {
        // lunch today, then "my balance now is 100": the balance is dated tomorrow, so it covers lunch, as bean-check
        // reads it, and the padding is 130
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "Shop" "lunch"
  time: "12:00:00"
  Expenses:Food 30 CNY
  Assets:A
"#,
            today()
        ));
        let (status, body) = pad(&scratch, "Assets:A", amount(100, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        let files = written(&scratch);
        assert!(files.contains(&format!("{} balance Assets:A 100 CNY", tomorrow())), "{files}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings, vec![format!("{} 130 CNY from Equity:Open", today())]);
        assert_eq!(passed, vec![true]);
        assert_eq!(scratch.balance("Assets:A").await, json!({"CNY": "100"}));

        // a plain check is dated tomorrow too, and replaces the balance of tomorrow
        let (status, body) = check(&scratch, "Assets:A", amount(100, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        assert_eq!(body["data"]["replaced"][0]["amount"]["number"], json!("100"), "{body}");
        assert_eq!(written(&scratch).matches(&format!("{} balance Assets:A 100 CNY", tomorrow())).count(), 1);
        let (errors, _, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(passed, vec![true]);
    }

    #[tokio::test]
    async fn a_batch_books_each_difference_from_its_own_account() {
        // two commodities of one account from two accounts, and a plain check of a third
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}1970-01-01 commodity EUR
{} * "seed"
  Assets:A 5 EUR
  Equity:Fx
"#,
            days_ago(3)
        ));
        let (status, body) = rows(
            &scratch,
            vec![
                ("Assets:A", amount(100, "CNY"), "Equity:Open"),
                ("Assets:A", amount(20, "USD"), "Equity:Fx"),
                ("Assets:A", amount(6, "EUR"), ""),
            ],
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
        assert!(pad_directives(&scratch).is_empty());
        let main = std::fs::read_to_string(scratch.dir.join(scratch.main)).unwrap();
        assert_eq!(main.matches("include ").count(), 1, "{main}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        // the plain EUR check is not padded: it fails
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(
            paddings,
            vec![format!("{} 100 CNY from Equity:Open", today()), format!("{} 20 USD from Equity:Fx", today())]
        );
        assert_eq!(passed, vec![true, true, false]);
    }

    #[tokio::test]
    async fn a_batch_books_the_difference_of_a_parent_after_its_sub_accounts() {
        // the parent's balance covers its sub-accounts: its difference counts theirs, and an account named like it
        // is not one of them
        let scratch = Scratch::beancount(&format!(
            r#"option "operating_currency" "CNY"
option "timezone" "UTC"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Checking:Sub
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Assets:BankX
1970-01-01 open Equity:Open
1970-01-01 open Income:X
{} * "seed"
  Assets:Bank:Checking 60 CNY
  Assets:Bank:Checking:Sub 5 CNY
  Assets:Bank:Savings 40 CNY
  Assets:Bank 5 CNY
  Assets:BankX 1000 CNY
  Income:X
{} * "payday, not yet"
  Assets:Bank:Checking 500 CNY
  Income:X
"#,
            days_ago(3),
            tomorrow().checked_add_days(Days::new(5)).unwrap()
        ));
        let (status, body) = rows(
            &scratch,
            vec![
                ("Assets:Bank", amount(200, "CNY"), "Equity:Open"),
                ("Assets:Bank:Checking:Sub", amount(10, "CNY"), "Equity:Open"),
                ("Assets:Bank:Checking", amount(70, "CNY"), "Equity:Open"),
                ("Assets:Bank:Savings", amount(45, "CNY"), ""),
                ("Assets:BankX", amount(1010, "CNY"), "Equity:Open"),
            ],
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        // Sub is padded by 5, which brings Checking to its 70; the plain Savings check fails at 40; the parent is
        // padded from the 115 the batch leaves it, not counting Assets:BankX and its padding, nor the payday after
        // tomorrow
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(
            paddings,
            vec![
                format!("{} 10 CNY from Equity:Open", today()),
                format!("{} 5 CNY from Equity:Open", today()),
                format!("{} 85 CNY from Equity:Open", today())
            ]
        );
        assert_eq!(passed.iter().filter(|it| **it).count(), 4);
    }

    #[tokio::test]
    async fn a_balance_a_pad_of_the_ledger_would_serve_is_refused() {
        // a pad written by hand three days ago, when the account held no USD: it would pad a USD balance now
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} pad Assets:A Equity:Open
{} balance Assets:A 100 CNY
"#,
            days_ago(3),
            days_ago(2)
        ));
        let before = written(&scratch);
        let close = format!(
            "Close that pad first: edit main.bean, and add a balance of Assets:A in USD on {}, right after it",
            days_ago(2)
        );
        for answer in [
            check(&scratch, "Assets:A", amount(20, "USD")).await,
            pad(&scratch, "Assets:A", amount(20, "USD"), "Equity:Fx").await,
        ] {
            refused(
                &scratch,
                &before,
                answer,
                &[
                    "beancount pads every commodity of an account",
                    &format!("the pad of Assets:A on {} from Equity:Open, in main.bean", days_ago(3)),
                    &close,
                ],
            );
        }
        // closed as told, the balance is written
        append(&scratch, &format!("{} balance Assets:A 0 USD\n", days_ago(2)));
        let (status, body) = pad(&scratch, "Assets:A", amount(20, "USD"), "Equity:Fx").await;
        assert!(status.is_success(), "{status} {body}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            paddings,
            vec![
                format!("{} 100 CNY from Equity:Open", days_ago(3)),
                format!("{} 20 USD from Equity:Fx", today())
            ]
        );
        assert!(passed.iter().all(|it| *it), "{passed:?}");
    }

    #[tokio::test]
    async fn a_balance_of_an_account_not_open_is_refused() {
        let scratch = Scratch::beancount(&format!(
            "{OPENS}1970-01-01 open Assets:Old\n{} close Assets:Old\n1970-01-01 open Equity:Gone\n{} close Equity:Gone\n",
            days_ago(10),
            days_ago(10)
        ));
        let before = written(&scratch);
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:Old", amount(10, "CNY"), "Equity:Open").await,
            &["Assets:Old is closed", "Reopen it"],
        );
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:A", amount(10, "CNY"), "Equity:Gone").await,
            &["Equity:Gone is closed", "a pad from Equity:Gone"],
        );
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:A", amount(10, "CNY"), "Equity:Never").await,
            &["Equity:Never is not open", "Open it first"],
        );
        // a plain balance only records: it may follow the close, as in beancount
        let (status, body) = check(&scratch, "Assets:Old", amount(0, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
    }

    /// whether an account may take a balance is the account lifecycle's, the rule the written balance is checked with: a
    /// balance only records, and may follow the close; a padding books, and needs the account open when it is booked
    #[tokio::test]
    async fn a_balance_is_refused_by_the_rule_it_is_checked_with() {
        let ledger = format!(
            "{OPENS}1970-01-01 open Assets:Today\n{} close Assets:Today\n1970-01-01 open Assets:Again\n{} close Assets:Again\n{} open Assets:Again\n\
             1970-01-01 open Assets:Gone\n{} close Assets:Gone\n",
            today(),
            days_ago(10),
            days_ago(5),
            days_ago(3)
        );

        // a zhang ledger writes now: an account closed today with only a date is open through today, and one opened
        // again after its close is open
        let scratch = Scratch::new(&ledger);
        let (status, body) = check(&scratch, "Assets:Today", amount(0, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        let (status, body) = pad(&scratch, "Assets:Again", amount(10, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        let (status, body) = pad(&scratch, "Assets:A", amount(10, "CNY"), "Assets:Today").await;
        assert!(status.is_success(), "{status} {body}");
        // closed three days ago: a balance may follow the close, a padding may not
        let (status, body) = check(&scratch, "Assets:Gone", amount(0, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        let before = written(&scratch);
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:Gone", amount(10, "CNY"), "Equity:Open").await,
            &["Assets:Gone is closed", "a balance of Assets:Gone"],
        );
        let (errors, paddings, passed) = reloaded(&scratch).await;
        // the padding from the account closed today leaves it at -10 CNY when its close takes effect, at the end of today
        assert_eq!(errors, vec![ErrorKind::CloseNonZeroAccount]);
        assert_eq!(
            paddings,
            vec![format!("{} 10 CNY from Assets:Today", today()), format!("{} 10 CNY from Equity:Open", today())]
        );
        assert!(passed.iter().all(|it| *it), "{passed:?}");

        // a beancount ledger books the padding now, while the account closed today is open, and checks the balance at
        // the start of tomorrow, after its close, which a balance may follow
        let scratch = Scratch::beancount(&ledger);
        let (status, body) = pad(&scratch, "Assets:A", amount(10, "CNY"), "Assets:Today").await;
        assert!(status.is_success(), "{status} {body}");
        let (status, body) = check(&scratch, "Assets:Today", Amount::new(BigDecimal::from(-10), "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        let (status, body) = pad(&scratch, "Assets:Again", amount(10, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        let before = written(&scratch);
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:Gone", amount(10, "CNY"), "Equity:Open").await,
            &["Assets:Gone is closed", "a balance of Assets:Gone"],
        );
        let (errors, _, passed) = reloaded(&scratch).await;
        // as in the zhang ledger, the account closed today holds -10 CNY when its close takes effect
        assert_eq!(errors, vec![ErrorKind::CloseNonZeroAccount]);
        assert!(passed.iter().all(|it| *it), "{passed:?}");
    }

    #[tokio::test]
    async fn a_pad_row_with_nothing_to_pad_books_nothing() {
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "salary"
  Assets:A 100 CNY
  Equity:Open
"#,
            days_ago(3)
        ));
        let (status, body) = rows(&scratch, vec![("Assets:A", amount(100, "CNY"), "Equity:Open")]).await;
        assert!(status.is_success(), "{status} {body}");
        let files = written(&scratch);
        assert!(!files.contains("Balance Pad"), "{files}");
        assert!(files.contains(&format!("{} balance Assets:A 100 CNY", tomorrow())), "{files}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert!(paddings.is_empty());
        assert_eq!(passed, vec![true]);
    }

    #[tokio::test]
    async fn a_plain_check_after_a_pad_that_served_its_commodity_is_written() {
        // a pad with a balance of every commodity the account held: it cannot pad a later check
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "salary"
  Assets:A 20 USD
  Equity:Fx
{} pad Assets:A Equity:Open
{} balance Assets:A 100 CNY
{} balance Assets:A 20 USD
"#,
            days_ago(5),
            days_ago(3),
            days_ago(2),
            days_ago(2)
        ));
        let (status, body) = check(&scratch, "Assets:A", amount(25, "USD")).await;
        assert!(status.is_success(), "{status} {body}");
        // it fails, as it should: the account holds 20
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(paddings, vec![format!("{} 100 CNY from Equity:Open", days_ago(3))]);
        assert_eq!(passed, vec![true, true, false]);
    }

    #[tokio::test]
    async fn a_batch_with_two_rows_for_one_commodity_is_refused() {
        let scratch = Scratch::beancount(OPENS);
        let before = written(&scratch);
        for batch in [
            vec![("Assets:A", amount(100, "CNY"), "Equity:Open"), ("Assets:A", amount(90, "CNY"), "Equity:Open")],
            vec![("Assets:A", amount(100, "CNY"), "Equity:Open"), ("Assets:A", amount(90, "CNY"), "")],
            vec![("Assets:A", amount(100, "CNY"), ""), ("Assets:A", amount(90, "CNY"), "")],
        ] {
            let answer = rows(&scratch, batch).await;
            refused(&scratch, &before, answer, &["two rows for Assets:A in CNY"]);
        }
    }

    #[tokio::test]
    async fn an_old_reconcile_whose_time_is_ignored_is_reported() {
        // what the UI wrote before: breakfast at 08:00, then "100 now, pad from Equity:Open" at 09:30, and a plain
        // check at 12:00. Both balances are checked at the start of the day now, before breakfast, as beancount checks
        // them: the pad pads 50 where it padded 60, and each balance whose meaning changed is reported
        let scratch = Scratch::beancount(
            r#"option "operating_currency" "CNY"
option "timezone" "UTC"
2020-01-01 commodity CNY
2024-01-01 open Assets:A
2024-01-01 open Equity:Open
2024-01-01 open Expenses:Food
2024-01-01 open Income:Salary
2024-03-01 * "salary"
  Assets:A 50 CNY
  Income:Salary
2024-03-05 * "breakfast"
  Assets:A -10 CNY
  Expenses:Food
  time: "08:00:00"
2024-03-04 pad Assets:A Equity:Open
2024-03-05 balance Assets:A 100 CNY
  time: "09:30:00"
2024-03-05 balance Assets:A 100 CNY
  time: "12:00:00"
2024-03-06 balance Assets:A 90 CNY
  time: "07:00:00"
2024-03-06 * "lunch"
  Assets:A -5 CNY
  Expenses:Food
  time: "13:00:00"
"#,
        );
        let (errors, paddings, passed) = reloaded(&scratch).await;
        // the balance of the 6th is timed before lunch: its meaning did not change
        assert_eq!(errors, vec![ErrorKind::BalanceTimeIgnored, ErrorKind::BalanceTimeIgnored]);
        assert_eq!(paddings, vec!["2024-03-04 50 CNY from Equity:Open".to_owned()]);
        assert_eq!(passed, vec![true, true, true]);
        assert_eq!(scratch.balance("Assets:A").await, json!({"CNY": "85"}));
    }

    #[tokio::test]
    async fn a_second_check_of_the_day_replaces_the_first() {
        // reconciled at 100, then dinner is added, then "80 now": the balance of tomorrow is replaced, not doubled
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "seed usd"
  Assets:A 10 USD
  Equity:Fx
"#,
            days_ago(30)
        ));
        let (status, body) = pad(&scratch, "Assets:A", amount(100, "CNY"), "Equity:Open").await;
        assert!(status.is_success(), "{status} {body}");
        assert_eq!(body["data"]["replaced"], json!([]));
        let reconciled = written(&scratch);
        append(
            &scratch,
            &format!(
                r#"{} * "Shop" "dinner"
  Expenses:Food 20 CNY
  Assets:A
"#,
                today()
            ),
        );
        let (status, body) = check(&scratch, "Assets:A", amount(80, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        // the answer says what was replaced, for the UI to tell
        assert_eq!(
            body["data"]["replaced"],
            json!([{"date": tomorrow().to_string(), "account": "Assets:A", "amount": {"number": "100", "commodity": "CNY"}, "tolerance": null}])
        );
        // in its place
        assert_eq!(written(&scratch), reconciled.replace("balance Assets:A 100 CNY", "balance Assets:A 80 CNY"));
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings, vec![format!("{} 100 CNY from Equity:Open", today())]);
        assert_eq!(passed, vec![true]);
    }

    #[tokio::test]
    async fn a_second_reconcile_of_the_day_pads_from_the_first_and_replaces_its_balance() {
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "seed"
  Assets:A 50 CNY
  Equity:Fx
"#,
            days_ago(30)
        ));
        for target in [20, 22] {
            let (status, body) = pad(&scratch, "Assets:A", amount(target, "CNY"), "Equity:Open").await;
            assert!(status.is_success(), "{status} {body}");
        }
        let files = written(&scratch);
        assert_eq!(files.matches(" balance Assets:A ").count(), 1, "{files}");
        let (errors, paddings, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        // the first padding stays; the second books the 2 from where it left the account
        assert_eq!(
            paddings,
            vec![format!("{} -30 CNY from Equity:Open", today()), format!("{} 2 CNY from Equity:Open", today())]
        );
        assert_eq!(passed, vec![true]);
        assert_eq!(scratch.balance("Assets:A").await, json!({"CNY": "22"}));
    }

    #[tokio::test]
    async fn a_pad_that_could_only_fail_is_refused() {
        // a commodity held at cost, also by a sub-account; a pad from the account's own sub-account
        let ledger = |main: &str| {
            format!(
                r#"option "operating_currency" "CNY"
option "timezone" "UTC"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:A
1970-01-01 open Assets:A:Sub
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Broker:Stock
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Open
1970-01-01 open Income:X
{} * "seed"
  Assets:A:Sub 50 CNY
  Income:X
{} * "buy"
  {main} 10 AAPL {{100 USD}}
  Assets:Cash -1000 USD
"#,
                days_ago(5),
                days_ago(4)
            )
        };
        for scratch in [Scratch::beancount(&ledger("Assets:Broker")), Scratch::new(&ledger("Assets:Broker"))] {
            let before = written(&scratch);
            refused(
                &scratch,
                &before,
                pad(&scratch, "Assets:Broker", amount(7, "AAPL"), "Equity:Open").await,
                &[
                    "Assets:Broker holds AAPL at cost",
                    "would book -3 AAPL without a cost",
                    "as a purchase or a sale",
                ],
            );
            refused(
                &scratch,
                &before,
                pad(&scratch, "Assets:A", amount(80, "CNY"), "Assets:A:Sub").await,
                &["Assets:A cannot be padded from Assets:A:Sub", "Pad it from another account"],
            );
            refused(
                &scratch,
                &before,
                pad(&scratch, "Assets:A", amount(80, "CNY"), "Assets:A").await,
                &["Assets:A cannot be padded from Assets:A"],
            );
            // nothing to pad: no padding without a cost
            let (status, body) = pad(&scratch, "Assets:Broker", amount(10, "AAPL"), "Equity:Open").await;
            assert!(status.is_success(), "{status} {body}");
        }
        // the parent of an account holding lots
        let scratch = Scratch::beancount(&ledger("Assets:Broker:Stock"));
        let before = written(&scratch);
        refused(
            &scratch,
            &before,
            pad(&scratch, "Assets:Broker", amount(12, "AAPL"), "Equity:Open").await,
            &["Assets:Broker holds AAPL at cost", "would book 2 AAPL without a cost"],
        );
    }
    fn read(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[tokio::test]
    async fn a_replaced_balance_changes_its_amount_only() {
        // balances for tomorrow written by hand, with metadata, a comment and a tolerance: the new amounts take the
        // places of theirs, and the rest stays as written. Three in one file, the first growing longer
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}include "statements.bean"
{} * "seed"
  Assets:A 100.50 CNY
  Assets:A 6 USD
  Expenses:Food 0.000000001 CNY
  Equity:Fx -100.500000001 CNY
  Equity:Fx -6 USD
"#,
            days_ago(30)
        ));
        let statements = scratch.dir.join("statements.bean");
        let written_by_hand = format!(
            r#"; the statement of the bank, for tomorrow
{tomorrow} balance Assets:A 61 CNY ; a trailing comment
  statement: "bank PDF"
{tomorrow} balance Equity:Open 0 CNY
{tomorrow} balance Assets:A 5 ~ 0.01 USD ; within a cent
  statement: "broker"
{tomorrow} balance Expenses:Food 1 CNY
"#,
            tomorrow = tomorrow()
        );
        std::fs::write(&statements, &written_by_hand).unwrap();
        let (status, body) = rows(
            &scratch,
            vec![
                ("Assets:A", Amount::new(decimal("100.50"), "CNY"), ""),
                ("Assets:A", amount(6, "USD"), ""),
                ("Expenses:Food", Amount::new(decimal("0.000000001"), "CNY"), ""),
            ],
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
        // the answer says what each asserted, with its tolerance
        assert_eq!(
            body["data"]["replaced"],
            json!([
                {"date": tomorrow().to_string(), "account": "Assets:A", "amount": {"number": "61", "commodity": "CNY"}, "tolerance": null},
                {"date": tomorrow().to_string(), "account": "Assets:A", "amount": {"number": "5", "commodity": "USD"}, "tolerance": "0.01"},
                {"date": tomorrow().to_string(), "account": "Expenses:Food", "amount": {"number": "1", "commodity": "CNY"}, "tolerance": null}
            ])
        );
        // a balance from the balance tools is exact: the tolerance goes, and the number is written in plain notation
        assert_eq!(
            read(&statements),
            written_by_hand
                .replace("Assets:A 61 CNY", "Assets:A 100.50 CNY")
                .replace("Assets:A 5 ~ 0.01 USD", "Assets:A 6 USD")
                .replace("Expenses:Food 1 CNY", "Expenses:Food 0.000000001 CNY")
        );
        assert_eq!(walkdir(&scratch.dir).len(), 2, "nothing is appended");
        let (errors, _, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(passed, vec![true, true, true, true]);
    }

    #[tokio::test]
    async fn a_replaced_balance_is_exact() {
        // a balance written by hand within 5 CNY: the check of 53 from the UI replaces it, and fails, as the account
        // holds 50
        let scratch = Scratch::beancount(&format!(
            "{OPENS}{} * \"seed\"\n  Assets:A 50 CNY\n  Equity:Fx\n{} balance Assets:A 50 ~ 5 CNY\n",
            days_ago(30),
            tomorrow()
        ));
        let main = scratch.dir.join(scratch.main);
        let before = read(&main);
        let (status, body) = check(&scratch, "Assets:A", amount(53, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        assert_eq!(body["data"]["replaced"][0]["tolerance"], json!("5"), "{body}");
        assert_eq!(read(&main), before.replace("balance Assets:A 50 ~ 5 CNY", "balance Assets:A 53 CNY"));
        let (errors, _, passed) = reloaded(&scratch).await;
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(passed, vec![false]);
    }

    #[tokio::test]
    async fn every_balance_of_the_commodity_for_tomorrow_is_replaced() {
        // the same balance written twice, in two files, one with CRLF line endings: both assert the new amount, and
        // keep what else they have
        let scratch = Scratch::beancount(&format!(
            "{OPENS}include \"crlf.bean\"\n{} * \"seed\"\n  Assets:A 80 CNY\n  Equity:Fx\n{} balance Assets:A 60 CNY\n",
            days_ago(30),
            tomorrow()
        ));
        let crlf = scratch.dir.join("crlf.bean");
        let written_by_hand = format!(
            "{} * \"Ünïcödé\" \"before\"\r\n  Expenses:Food 1 CNY\r\n  Equity:Fx\r\n{} balance Assets:A 62 CNY\r\n  statement: \"scan ☕\"\r\n",
            days_ago(3),
            tomorrow()
        );
        std::fs::write(&crlf, &written_by_hand).unwrap();
        let main = read(&scratch.dir.join(scratch.main));
        let (status, body) = check(&scratch, "Assets:A", amount(80, "CNY")).await;
        assert!(status.is_success(), "{status} {body}");
        let mut replaced = body["data"]["replaced"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it["amount"]["number"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        replaced.sort();
        assert_eq!(replaced, vec!["60", "62"], "{body}");
        assert_eq!(
            read(&scratch.dir.join(scratch.main)),
            main.replace("balance Assets:A 60 CNY", "balance Assets:A 80 CNY")
        );
        assert_eq!(read(&crlf), written_by_hand.replace("balance Assets:A 62 CNY", "balance Assets:A 80 CNY"));
        assert_eq!(walkdir(&scratch.dir).len(), 2, "nothing is appended");
        let (errors, _, passed) = reloaded(&scratch).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(passed, vec![true, true]);
    }

    #[tokio::test]
    async fn a_balance_for_tomorrow_a_pad_of_the_ledger_serves_is_not_replaced() {
        // the pad of yesterday, written by hand, pads tomorrow's balance: replaced, that balance would leave the pad to
        // pad the new one, which would absorb any transaction added today
        let scratch = Scratch::beancount(&format!(
            r#"{OPENS}{} * "seed"
  Assets:A 50 CNY
  Equity:Fx
{} pad Assets:A Equity:Open
{} balance Assets:A 100 CNY
"#,
            days_ago(30),
            days_ago(1),
            tomorrow()
        ));
        let main = scratch.dir.join(scratch.main);
        let before = read(&main);
        let (status, body) = check(&scratch, "Assets:A", amount(80, "CNY")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let message = body["message"].as_str().unwrap();
        let pad = format!("the pad of Assets:A on {} from Equity:Open, in main.bean", days_ago(1));
        for part in [pad.as_str(), "edit main.bean"] {
            assert!(message.contains(part), "{part:?} in {message}");
        }
        assert_eq!(read(&main), before, "nothing is written");
        assert_eq!(walkdir(&scratch.dir).len(), 1, "nothing is appended");
    }

    #[tokio::test]
    async fn a_commodity_whose_lots_were_all_sold_is_padded() {
        // the lots of AAPL are sold, so AAPL is no longer held at cost: the padding books it without one
        let ledger = format!(
            r#"option "operating_currency" "USD"
option "timezone" "UTC"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Open
{} * "buy"
  Assets:Broker 10 AAPL {{100 USD}}
  Assets:Cash -1000 USD
{} * "sell all"
  Assets:Broker -10 AAPL {{100 USD}}
  Assets:Cash 1000 USD
"#,
            days_ago(5),
            days_ago(4)
        );
        for scratch in [Scratch::beancount(&ledger), Scratch::new(&ledger)] {
            let (status, body) = pad(&scratch, "Assets:Broker", amount(3, "AAPL"), "Equity:Open").await;
            assert!(status.is_success(), "{} {status} {body}", scratch.main);
            let (errors, _, passed) = reloaded(&scratch).await;
            assert!(errors.is_empty(), "{} {errors:?}", scratch.main);
            assert_eq!(passed, vec![true], "{}", scratch.main);
            assert_eq!(scratch.balance("Assets:Broker").await["AAPL"], json!("3"), "{}", scratch.main);
        }
    }

    #[tokio::test]
    async fn a_commodity_sold_after_the_reconcile_is_held_at_cost() {
        // the lots of AAPL are sold in three weeks: today, the account holds them at cost, which a padding would book
        // units without
        let ledger = format!(
            r#"option "operating_currency" "USD"
option "timezone" "UTC"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Open
1970-01-01 open Income:Gains
{} * "buy"
  Assets:Broker 10 AAPL {{100 USD}}
  Assets:Cash -1000 USD
{} * "scheduled sale"
  Assets:Broker -10 AAPL {{100 USD}} @ 120 USD
  Assets:Cash 1200 USD
  Income:Gains -200 USD
"#,
            days_ago(5),
            today().checked_add_days(Days::new(20)).unwrap()
        );
        for scratch in [Scratch::beancount(&ledger), Scratch::new(&ledger)] {
            let before = written(&scratch);
            refused(
                &scratch,
                &before,
                pad(&scratch, "Assets:Broker", amount(12, "AAPL"), "Equity:Open").await,
                &["Assets:Broker holds AAPL at cost", "would book 2 AAPL without a cost"],
            );
        }
    }

    /// a ledger whose main file holds tomorrow's balance after a price, a transaction before them, and one after
    fn ledger_with_tomorrows_balance() -> Scratch {
        Scratch::beancount(&format!(
            r#"{OPENS}1970-01-01 open Income:X
{} * "seed"
  Assets:A 50 CNY
  Income:X
{} price USD 7.1000000 CNY
{} balance Assets:A 50 CNY
{} * "tail" "must survive"
  Expenses:Food 1 CNY
  Income:X
"#,
            days_ago(30),
            days_ago(30),
            tomorrow(),
            days_ago(29)
        ))
    }

    #[tokio::test]
    async fn a_balance_in_a_file_changed_since_the_load_is_not_replaced() {
        // an editor adds a line at the top of the file, as long as the price line or longer, and the ledger is not
        // reloaded yet: replaced at the places loaded, the balance would overwrite the price, or cut a transaction
        for comment in ["; an editor adds a line at the top of the file", "; a line as long as the price line"] {
            let scratch = ledger_with_tomorrows_balance();
            let main = scratch.dir.join(scratch.main);
            let ledger = scratch.state().await;
            let edited = format!("{comment}\n{}", read(&main));
            std::fs::write(&main, &edited).unwrap();

            let (status, body) = check_with(ledger.clone(), "Assets:A", amount(45, "CNY")).await;
            assert_eq!(status, StatusCode::CONFLICT, "{body}");
            let message = body["message"].as_str().unwrap();
            assert!(
                message.contains("main.bean changed since the ledger was loaded, so nothing was written"),
                "{message}"
            );
            assert_eq!(read(&main), edited, "nothing is written");
            assert_eq!(walkdir(&scratch.dir).len(), 1, "nothing is appended");
            let (errors, _, _) = reloaded(&scratch).await;
            assert!(errors.is_empty(), "{errors:?}");

            // tried again, on the ledger reloaded: the balance is replaced, and the line of the editor and the price stay
            let (status, body) = check_with(ledger, "Assets:A", amount(45, "CNY")).await;
            assert!(status.is_success(), "{status} {body}");
            assert_eq!(read(&main), edited.replace("balance Assets:A 50 CNY", "balance Assets:A 45 CNY"));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn writes_at_once_are_written_one_after_the_other() {
        // two reconciles of the same balance and an update of a transaction of the same file, at once: each reads
        // what the one before wrote
        for _ in 0..10 {
            let scratch = ledger_with_tomorrows_balance();
            let ledger = scratch.state().await;
            let tail = {
                let ledger = ledger.read().await;
                let store = ledger.store.read().unwrap();
                store
                    .transactions
                    .values()
                    .find(|it| it.narration.as_deref() == Some("must survive"))
                    .unwrap()
                    .id
            };
            let reconcile = || {
                let ledger = ledger.clone();
                tokio::spawn(async move { pad_with(ledger, "Assets:A", amount(60, "CNY"), "Equity:Open").await })
            };
            let (first, second) = (reconcile(), reconcile());
            let update = {
                let ledger = ledger.clone();
                let request = serde_json::from_value(json!({
                    "datetime": format!("{}T12:00:00Z", days_ago(29)),
                    "payee": "tail",
                    "flag": "Okay",
                    "narration": "updated",
                    "postings": [
                        {"account": "Expenses:Food", "unit": {"number": "1", "commodity": "CNY"}},
                        {"account": "Income:X", "unit": null}
                    ],
                    "metas": [],
                    "tags": [],
                    "links": []
                }))
                .unwrap();
                tokio::spawn(async move { respond(update_single_transaction(ledger, reload(), UrlPath((tail.to_string(),)), Json(request)).await).await })
            };
            for (status, body) in [first.await.unwrap(), second.await.unwrap(), update.await.unwrap()] {
                assert!(status.is_success(), "{status} {body}");
            }
            let main = read(&scratch.dir.join(scratch.main));
            assert!(main.contains(&format!("{} balance Assets:A 60 CNY\n", tomorrow())), "{main}");
            assert!(main.contains("\"tail\" \"updated\""), "{main}");
            assert!(main.contains("price USD 7.1000000 CNY"), "{main}");
            let files = format!("{main}{}", written(&scratch));
            assert_eq!(files.matches(" balance Assets:A ").count(), 1, "{files}");
            let (errors, paddings, passed) = reloaded(&scratch).await;
            assert!(errors.is_empty(), "{errors:?}\n{files}");
            assert_eq!(paddings, vec![format!("{} 10 CNY from Equity:Open", today())], "{files}");
            assert_eq!(passed, vec![true]);
        }
    }
}
