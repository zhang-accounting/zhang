use axum::extract::{Multipart, Path, State};
use axum::{debug_handler, Json};
use gotcha::api;
use log::info;
use zhang_ast::amount::Amount;
use zhang_ast::{Date, Directive, Document, ZhangString};
use zhang_core::domains::schemas::AccountJournalDomain;

use crate::balance_writes::{balance_directives, BalanceRow, BalanceWriteEntity};
use crate::request::{AccountBalanceRequest, AccountJournalRequest, BatchAccountBalanceRequest};
use crate::response::{AccountBalanceHistoryEntity, AccountEntity, AccountInfoEntity, Created, DocumentEntity, Paged, ResponseWrapper};
use crate::routes::Query;
use crate::state::{wrote, SharedLedger, SharedReloadSender};
use crate::validate::Rules;
use crate::{account_queries, validate, ApiResult, ServerResult};

/// `amount`, once its commodity is checked to read back unchanged.
fn validated(amount: Amount, rules: &Rules) -> ServerResult<Amount> {
    validate::amount(&amount, rules)?;
    Ok(amount)
}

/// Every account with an `open` or `close` directive or with postings, by name: its own balance, valued in the
/// operating currency at today's prices, and the balance of the account with its sub-accounts.
///
/// Built-in queries `accounts.list` and `accounts.balances`.
#[api(group = "account")]
pub async fn get_account_list(ledger: State<SharedLedger>) -> ApiResult<Vec<AccountEntity>> {
    ResponseWrapper::json(account_queries::with_ledger(&ledger, account_queries::account_list).await?)
}

/// An account with an `open` or `close` directive or with postings: its own balance, and the balance of the
/// account with its sub-accounts, which its page shows. Any other account is a 404, and a name that is no account
/// name a 400.
///
/// Built-in queries `accounts.subtree` and `accounts.subtree_balances`.
#[api(group = "account")]
pub async fn get_account_info(ledger: State<SharedLedger>, path: Path<(String,)>) -> ApiResult<AccountInfoEntity> {
    let account_name = path.0 .0;
    match account_queries::with_ledger(&ledger, move |ledger| account_queries::account_info(ledger, &account_name)).await? {
        Some(info) => ResponseWrapper::json(info),
        None => ResponseWrapper::not_found(),
    }
}

#[api(group = "account")]
pub async fn upload_account_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ServerResult<Created> {
    let account_name = path.0 .0;
    // the files first, then the ledger, held to write
    let files = super::uploaded_files(&mut multipart).await?;
    let mut ledger_stage = ledger.for_writing().await?;
    let account = validate::account(&account_name, &Rules::of(&ledger_stage))?;
    let written = async {
        let mut documents = vec![];
        for (file_name, content_buf) in files {
            let (v4, striped_path_string) = super::attachment_path(&file_name);
            info!("uploading document `{}`(id={}) to account {}", file_name, v4, account_name);

            ledger_stage
                .data_source
                .async_save(&ledger_stage, striped_path_string.to_owned(), &content_buf)
                .await?;

            documents.push(Directive::Document(Document {
                date: Date::now(&ledger_stage.options.timezone),
                account: account.clone(),
                filename: ZhangString::QuoteString(striped_path_string),
                tags: None,
                links: None,
                meta: Default::default(),
            }));
        }
        ledger_stage.data_source.async_append(&ledger_stage, documents).await?;
        ServerResult::Ok(())
    }
    .await;
    wrote(&mut ledger_stage, &reload_sender, written)?;
    Ok(Created)
}

/// The balance of the account and its sub-accounts at the end of every day it changed, per currency, in date order.
/// An account without a page is a 404, and a name that is no account name a 400, as for `GET /api/accounts/{a}`.
///
/// Built-in query `accounts.balance_history`.
#[api(group = "account")]
#[debug_handler]
pub async fn get_account_balance_data(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<AccountBalanceHistoryEntity> {
    let account_name = params.0 .0;
    ResponseWrapper::json(account_queries::with_ledger(&ledger, move |ledger| account_queries::account_balance_history(ledger, &account_name)).await?)
}

/// The document directives of the account and its sub-accounts, in ledger order. An account without a page is a
/// 404, and a name that is no account name a 400, as for `GET /api/accounts/{a}`.
///
/// Built-in query `accounts.documents`.
#[api(group = "account")]
pub async fn get_account_documents(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<Vec<DocumentEntity>> {
    let account_name = params.0 .0;
    ResponseWrapper::json(account_queries::with_ledger(&ledger, move |ledger| account_queries::account_documents(ledger, &account_name)).await?)
}

/// The journal of the account and its sub-accounts, newest first: a row per posting, with the account it posts
/// to and the running balance of the account with its sub-accounts in its currency, and a row per balance
/// assertion on the account, with the balance it was checked against.
///
/// With `page` and `size` (from 1; `size` 100 by default and at most 1000), one page of the rows, and the number
/// of rows of all the pages in the `X-Total-Count` header. Without them, the whole journal; a journal too large to
/// return at once is a 400 that asks for pages. An account without a page is a 404, and a name that is no account
/// name a 400, as for `GET /api/accounts/{a}`.
///
/// Built-in queries `accounts.journal` (`accounts.journal_page` for a page) and
/// `accounts.balance_assertions`.
#[api(group = "account")]
pub async fn get_account_journals(
    ledger: State<SharedLedger>, params: Path<(String,)>, page: Query<AccountJournalRequest>,
) -> ServerResult<Paged<Vec<AccountJournalDomain>>> {
    let account_name = params.0 .0;
    let window = page.0.window()?;
    let journal = account_queries::with_ledger(&ledger, move |ledger| account_queries::account_journals(ledger, &account_name, window)).await?;
    Ok(Paged {
        data: journal.rows,
        total: journal.total,
    })
}

#[api(group = "account")]
pub async fn create_account_balance(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, params: Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> ApiResult<BalanceWriteEntity> {
    let target_account = params.0 .0;
    let mut ledger = ledger.for_writing().await?;
    let rules = Rules::of(&ledger);

    let row = match payload {
        AccountBalanceRequest::Check { amount } => BalanceRow {
            account: validate::account(&target_account, &rules)?,
            amount: validated(amount, &rules)?,
            pad: None,
        },
        AccountBalanceRequest::Pad { amount, pad } => BalanceRow {
            account: validate::account(&target_account, &rules)?,
            amount: validated(amount, &rules)?,
            pad: Some(validate::account(&pad, &rules)?),
        },
    };

    let writes = balance_directives(&ledger, vec![row], Date::now(&ledger.options.timezone))?;
    let written = writes.write(&ledger).await;
    ResponseWrapper::json(wrote(&mut ledger, &reload_sender, written)?)
}

/// the balances of a batch, those of deeper accounts first, the order of the request kept otherwise
fn sub_accounts_first(mut balances: Vec<BatchAccountBalanceRequest>) -> Vec<BatchAccountBalanceRequest> {
    let depth = |balance: &BatchAccountBalanceRequest| match balance {
        BatchAccountBalanceRequest::Check { account_name, .. } | BatchAccountBalanceRequest::Pad { account_name, .. } => account_name.matches(':').count(),
    };
    balances.sort_by_key(|balance| std::cmp::Reverse(depth(balance)));
    balances
}

#[api(group = "account")]
pub async fn create_batch_account_balances(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Json(payload): Json<Vec<BatchAccountBalanceRequest>>,
) -> ApiResult<BalanceWriteEntity> {
    let mut ledger = ledger.for_writing().await?;
    let rules = Rules::of(&ledger);
    let mut rows = vec![];
    // sub-accounts before their parents, deepest first: a `balance` on a parent covers its sub-accounts, so it
    // must come after their pads to assert, and pad to, the total they leave
    for balance in sub_accounts_first(payload) {
        rows.push(match balance {
            BatchAccountBalanceRequest::Check { account_name, amount } => BalanceRow {
                account: validate::account(&account_name, &rules)?,
                amount: validated(amount, &rules)?,
                pad: None,
            },
            BatchAccountBalanceRequest::Pad { account_name, amount, pad } => BalanceRow {
                account: validate::account(&account_name, &rules)?,
                amount: validated(amount, &rules)?,
                pad: Some(validate::account(&pad, &rules)?),
            },
        });
    }

    // one time for the whole batch, so the file order decides the order of its directives
    let writes = balance_directives(&ledger, rows, Date::now(&ledger.options.timezone))?;
    let written = writes.write(&ledger).await;
    ResponseWrapper::json(wrote(&mut ledger, &reload_sender, written)?)
}

/// Balance requests are checked like transactions: a name the ledger would not read
/// back is a 400, and nothing is written (issue #442).
#[cfg(test)]
mod name_validation_test {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    use bigdecimal::BigDecimal;
    use tokio::sync::{mpsc, RwLock};
    use zhang_ast::amount::Amount;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::{create_account_balance, create_batch_account_balances};
    use crate::request::{AccountBalanceRequest, BatchAccountBalanceRequest};
    use crate::state::{SharedLedger, SharedReloadSender};
    use crate::ReloadSender;

    const MAIN: &str = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Equity:Opening\n";

    async fn states(dir: &std::path::Path) -> (State<SharedLedger>, State<SharedReloadSender>) {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(dir.to_path_buf(), "main.zhang".to_owned(), source).await.unwrap();
        let (sender, _) = mpsc::channel(1);
        (
            State(SharedLedger(Arc::new(RwLock::new(ledger)))),
            State(SharedReloadSender(Arc::new(ReloadSender(sender)))),
        )
    }

    fn cny(commodity: &str) -> Amount {
        Amount::new(BigDecimal::from(1), commodity)
    }

    async fn message(response: axum::response::Response) -> String {
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        body["message"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn balances_with_names_that_would_not_read_back_are_rejected() {
        let dir = std::env::temp_dir().join(format!("zhang-balance-names-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zhang"), MAIN).unwrap();

        let cases = [
            (
                "Assets:My Bank",
                AccountBalanceRequest::Check { amount: cny("CNY") },
                "invalid account \"Assets:My Bank\"",
            ),
            (
                "Assets:Cash",
                AccountBalanceRequest::Check { amount: cny("cny 1") },
                "invalid commodity \"cny 1\"",
            ),
            (
                "Assets:Cash",
                AccountBalanceRequest::Pad {
                    amount: cny("CNY"),
                    pad: "Equity:Opening Balances".to_owned(),
                },
                "invalid account \"Equity:Opening Balances\"",
            ),
        ];
        for (account, request, expected) in cases {
            let (ledger, reload) = states(&dir).await;
            let response = create_account_balance(ledger, reload, Path((account.to_owned(),)), Json(request))
                .await
                .into_response();
            let message = message(response).await;
            assert!(message.starts_with(expected), "{message}");
        }

        let (ledger, reload) = states(&dir).await;
        let batch = vec![
            BatchAccountBalanceRequest::Check {
                account_name: "Assets:Cash".to_owned(),
                amount: cny("CNY"),
            },
            BatchAccountBalanceRequest::Check {
                account_name: "Assets:a,b".to_owned(),
                amount: cny("CNY"),
            },
        ];
        let response = create_batch_account_balances(ledger, reload, Json(batch)).await.into_response();
        assert!(message(response).await.starts_with("invalid account \"Assets:a,b\""));

        // nothing was appended, not even the valid balance of the batch
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!((files, std::fs::read_to_string(dir.join("main.zhang")).unwrap()), (1, MAIN.to_owned()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_beancount_ledger_rejects_balances_with_names_beancount_cannot_read() {
        // beancount 3.2.3 rejects `Assets:bank` (lowercase component) and the
        // commodity `usd`, which zhang's parsers read
        let dir = std::env::temp_dir().join(format!("zhang-beancount-balance-names-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.bean"), MAIN).unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            let ledger = Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.unwrap();
            let (sender, _) = mpsc::channel(1);
            (
                State(SharedLedger(Arc::new(RwLock::new(ledger)))),
                State(SharedReloadSender(Arc::new(ReloadSender(sender)))),
            )
        };

        let cases = [
            (
                "Assets:bank",
                AccountBalanceRequest::Check { amount: cny("CNY") },
                "invalid account \"Assets:bank\": beancount ",
            ),
            (
                "Assets:Cash",
                AccountBalanceRequest::Check { amount: cny("usd") },
                "invalid commodity \"usd\": beancount ",
            ),
        ];
        for (account, request, expected) in cases {
            let (ledger, reload) = load().await;
            let response = create_account_balance(ledger, reload, Path((account.to_owned(),)), Json(request))
                .await
                .into_response();
            let message = message(response).await;
            assert!(message.starts_with(expected), "{message}");
        }
        assert_eq!(std::fs::read_to_string(dir.join("main.bean")).unwrap(), MAIN);
        std::fs::remove_dir_all(dir).ok();
    }
}
