//! Every number the API writes is in plain notation, as the queries and the exporter write it: a tiny amount is
//! `"0.0000001"`, never `BigDecimal`'s `"1E-7"`, and a huge one is all its digits, never `"…E+30"`. Numbers keep
//! their scale (`"0.50"`). This covers the amounts and bare numbers of the account, journal, commodity and error
//! endpoints, the error metas included.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde_json::Value;
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::{AccountJournalRequest, BudgetListRequest, JournalRequest};
use zhang_server::routes::account::{get_account_info, get_account_journals, get_account_list};
use zhang_server::routes::budget::get_budget_list;
use zhang_server::routes::commodity::{get_all_commodities, get_single_commodity};
use zhang_server::routes::common::get_errors;
use zhang_server::routes::transaction::get_journals;
use zhang_server::routes::Query;
use zhang_server::state::SharedLedger;

/// Tiny numbers in BTC; a huge USD holding and expense, which valued in CNY need more than 28 digits and are rounded
/// to 28 (`1.234567890123456789012345679E+30` in `BigDecimal`'s notation).
const LEDGER: &str = r#"option "operating_currency" "CNY"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity BTC
  precision: 8

1970-01-01 open Assets:Wallet
1970-01-01 open Assets:Vault
1970-01-01 open Assets:Short
1970-01-01 open Equity:Open
1970-01-01 budget Big CNY
1970-01-01 open Expenses:Big
  budget: Big

2024-01-01 price BTC 0.0000002 CNY
2024-01-01 price USD 1.0 CNY

2024-01-02 * "Exchange" "a tiny amount"
  Assets:Wallet 0.0000001 BTC {0.50 CNY}
  Equity:Open

2024-01-03 * "Bank" "a huge amount"
  Assets:Vault 1234567890123456789012345678901 USD
  Equity:Open

2024-01-03 * "Bank" "a huge expense, valued in CNY in its budget"
  Expenses:Big 1234567890123456789012345678901 USD
  Equity:Open

2024-01-04 balance Assets:Wallet 0.0000001 ~ 0.00000001 BTC

2024-01-05 * "Exchange" "a sale of what no lot holds"
  Assets:Short -0.0000001 BTC {0.50 CNY}
  Equity:Open
"#;

async fn ledger() -> SharedLedger {
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("main.zhang"), LEDGER).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::load(dir, "main.zhang".to_owned(), source).unwrap();
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

async fn json(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn journal_request() -> JournalRequest {
    JournalRequest {
        page: None,
        size: None,
        keyword: None,
        tags: None,
        links: None,
    }
}

/// every string of `value` that reads as a number with an exponent, or holds one (an error meta lists lots as text,
/// `1E-7 BTC {…}`), with its JSON path
fn exponents(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let with_exponent = text.split([' ', ',', '{', '}']).any(|word| {
                let number = word.strip_prefix('-').unwrap_or(word);
                number.starts_with(|c: char| c.is_ascii_digit()) && (number.contains("E-") || number.contains("E+"))
            });
            if with_exponent {
                found.push(format!("{path}: {text}"));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                exponents(item, &format!("{path}[{index}]"), found);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                exponents(item, &format!("{path}.{key}"), found);
            }
        }
        _ => {}
    }
}

/// the record of `records` whose `key` is `value`
fn find<'a>(records: &'a Value, key: &str, value: &str) -> &'a Value {
    records
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it[key] == value)
        .unwrap_or_else(|| panic!("no {key} {value} in {records}"))
}

#[tokio::test]
async fn the_api_writes_tiny_and_huge_numbers_in_plain_notation() {
    let ledger = ledger().await;
    let wallet = || Path(("Assets:Wallet".to_owned(),));
    let vault = || Path(("Assets:Vault".to_owned(),));
    let mut responses = vec![];

    let list = json(get_account_list(State(ledger.clone())).await).await;
    responses.push(("/api/accounts", list));
    let info = json(get_account_info(State(ledger.clone()), wallet()).await).await;
    responses.push(("/api/accounts/Assets:Wallet", info));
    let info = json(get_account_info(State(ledger.clone()), vault()).await).await;
    responses.push(("/api/accounts/Assets:Vault", info));
    let journal = json(get_account_journals(State(ledger.clone()), wallet(), Query(AccountJournalRequest::default())).await).await;
    responses.push(("/api/accounts/Assets:Wallet/journals", journal));
    let journals = json(get_journals(State(ledger.clone()), Query(journal_request())).await).await;
    responses.push(("/api/journals", journals));
    let commodities = json(get_all_commodities(State(ledger.clone())).await).await;
    responses.push(("/api/commodities", commodities));
    let detail = json(get_single_commodity(State(ledger.clone()), Path(("BTC".to_owned(),))).await).await;
    responses.push(("/api/commodities/BTC", detail));
    let budgets = BudgetListRequest {
        month: Some(1),
        year: Some(2024),
    };
    let budgets = json(get_budget_list(State(ledger.clone()), axum::extract::Query(budgets)).await).await;
    responses.push(("/api/budgets", budgets));
    let errors = json(get_errors(State(ledger.clone()), Query(journal_request())).await).await;
    responses.push(("/api/errors", errors));

    let mut found = vec![];
    for (endpoint, response) in &responses {
        exponents(response, endpoint, &mut found);
    }
    assert_eq!(found, Vec::<String>::new());

    let [list, wallet, vault, journal, journals, commodities, detail, budgets, errors] = responses
        .iter()
        .map(|(_, response)| response["data"].clone())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let posting = find(&journal, "narration", "a tiny amount");
    let check = find(&journals["records"], "type", "BalanceCheck");
    let huge = find(&budgets, "name", "Big");
    let short = find(&errors["records"], "error_type", "NoEnoughCommodityLot");
    assert_eq!(
        [
            &find(&list, "name", "Assets:Wallet")["amount"]["detail"]["BTC"],
            &wallet["amount"]["detail"]["BTC"],
            &wallet["balance_with_sub_accounts"]["BTC"],
            &vault["amount"]["detail"]["USD"],
            &vault["amount"]["calculated"]["number"],
            &posting["inferred_unit"]["number"],
            &posting["account_after"]["number"],
            &check["tolerance"],
            &find(&commodities, "name", "BTC")["latest_price_amount"],
            &detail["prices"][0]["amount"]["number"],
            &huge["activity_amount"]["number"],
            &short["metas"]["transaction_amount"],
        ],
        [
            "0.0000001",
            "0.0000001",
            "0.0000001",
            "1234567890123456789012345678901",
            "1234567890123456789012345679000",
            "0.0000001",
            "0.0000001",
            "0.00000001",
            "0.0000002",
            "0.0000002",
            "1234567890123456789012345679000",
            "-0.0000001",
        ]
    );
}
