//! The value of an account with its sub-accounts in `GET /api/accounts` (audit finding D6): the server computes it, as
//! the account's page shows it, so the account list, the sidebar and the Accounts page use its figure instead of adding
//! up the accounts themselves. A closed account that still holds money counts in it.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::response::IntoResponse;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::routes::account::{get_account_info, get_account_list};
use zhang_server::state::SharedLedger;

const LEDGER: &str = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Old
1970-01-01 open Assets:Bank:Dollar
1970-01-01 open Equity:Open
2024-01-01 price USD 7 CNY
2024-01-02 * "opening"
  Assets:Bank 100 CNY
  Assets:Bank:Old 10 CNY
  Assets:Bank:Dollar 2 USD
  Equity:Open -124 CNY
2024-02-01 close Assets:Bank:Old
"#;

async fn ledger(dir: &Path) -> State<SharedLedger> {
    std::fs::write(dir.join("main.zhang"), LEDGER).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::async_load(dir.to_path_buf(), "main.zhang".to_owned(), source).await.unwrap();
    State(SharedLedger(Arc::new(RwLock::new(ledger))))
}

async fn data(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["data"].clone()
}

#[tokio::test]
async fn the_account_list_values_each_account_with_its_sub_accounts_as_its_page_does() {
    let dir = tempfile::tempdir().unwrap();
    let state = ledger(dir.path()).await;
    let list = data(get_account_list(state.clone()).await).await;
    let bank = list.as_array().unwrap().iter().find(|it| it["name"] == "Assets:Bank").unwrap().clone();
    // 100 CNY of its own, 10 CNY in the closed Assets:Bank:Old and 2 USD at 7 CNY
    assert_eq!(bank["amount_with_sub_accounts"]["calculated"], json!({"number": "124", "commodity": "CNY"}));
    assert_eq!(bank["amount_with_sub_accounts"]["detail"], json!({"CNY": "110", "USD": "2"}));
    let page = data(get_account_info(state.clone(), UrlPath(("Assets:Bank".to_owned(),))).await).await;
    assert_eq!(bank["amount_with_sub_accounts"], page["amount_with_sub_accounts"]);
    // an account without sub-accounts is valued as itself
    let old = list.as_array().unwrap().iter().find(|it| it["name"] == "Assets:Bank:Old").unwrap();
    assert_eq!(old["status"], "Close");
    assert_eq!(old["amount_with_sub_accounts"], old["amount"]);
}
