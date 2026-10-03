use axum::extract::{Multipart, Path, State};
use axum::{debug_handler, Json};
use chrono::Utc;
use gotcha::api;
use itertools::Itertools;
use log::info;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{BalanceCheck, BalancePad, Date, Directive, Document, ZhangString};
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::utils::calculable::Calculable;

use crate::request::{AccountBalanceRequest, BatchAccountBalanceRequest};
use crate::response::{AccountBalanceHistoryEntity, AccountBalanceItemEntity, AccountEntity, AccountInfoEntity, Created, DocumentEntity, ResponseWrapper};
use crate::state::{SharedLedger, SharedReloadSender};
use crate::validate::Rules;
use crate::{validate, ApiResult, ServerResult};

/// `amount`, once its commodity is checked to read back unchanged.
fn validated(amount: Amount, rules: &Rules) -> ServerResult<Amount> {
    validate::amount(&amount, rules)?;
    Ok(amount)
}

#[api(group = "account")]
pub async fn get_account_list(ledger: State<SharedLedger>) -> ApiResult<Vec<AccountEntity>> {
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let mut all_with_sub_accounts = operations.balances_with_sub_accounts()?;
    let mut ret = vec![];
    for account in operations.all_accounts()? {
        let account_domain = operations.account(&account)?.expect("cannot find account");
        let account_balances = operations
            .single_account_latest_balances(&account)?
            .into_iter()
            .map(|balance| balance.balance)
            .collect_vec();
        let amount = account_balances
            .calculate(Utc::now().with_timezone(timezone), &mut operations)?
            .persist_commodity(&ledger.options.operating_currency);

        let mut with_sub_accounts = all_with_sub_accounts.remove(&account).unwrap_or_default();
        with_sub_accounts.balance.entry(ledger.options.operating_currency.clone()).or_default();
        ret.push(AccountEntity {
            name: account,
            status: account_domain.status,
            alias: account_domain.alias,
            amount,
            balance_with_sub_accounts: with_sub_accounts.balance.into_iter().collect(),
            has_sub_accounts: with_sub_accounts.has_sub_accounts,
        });
    }
    ResponseWrapper::json(ret)
}

#[api(group = "account")]
pub async fn get_account_info(ledger: State<SharedLedger>, path: Path<(String,)>) -> ApiResult<AccountInfoEntity> {
    let account_name = path.0 .0;
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();
    let account_domain = operations.account(&account_name)?;

    let account_info = match account_domain {
        Some(info) => info,
        None => return ResponseWrapper::not_found(),
    };
    let vec = operations
        .single_account_latest_balances(&account_info.name)?
        .into_iter()
        .map(|balance| balance.balance)
        .collect_vec();
    let amount = vec
        .calculate(Utc::now().with_timezone(timezone), &mut operations)?
        .persist_commodity(&ledger.options.operating_currency);

    let mut with_sub_accounts = operations.balance_with_sub_accounts(&account_info.name)?;
    // like `amount`, it holds the operating currency, so a new account gets a row to set its opening balance
    with_sub_accounts.balance.entry(ledger.options.operating_currency.clone()).or_default();
    ResponseWrapper::json(AccountInfoEntity {
        date: account_info.date,
        r#type: account_info.r#type,
        name: account_info.name,
        status: account_info.status,
        alias: account_info.alias,
        amount,
        balance_with_sub_accounts: with_sub_accounts.balance.into_iter().collect(),
        has_sub_accounts: with_sub_accounts.has_sub_accounts,
    })
}

#[api(group = "account")]
pub async fn upload_account_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ServerResult<Created> {
    let account_name = path.0 .0;
    let ledger_stage = ledger.read().await;
    let account = validate::account(&account_name, &Rules::of(&ledger_stage))?;
    let entry = &ledger_stage.entry.0;
    let mut documents = vec![];

    while let Some(field) = multipart.next_field().await.unwrap() {
        let _name = field.name().unwrap().to_string();
        let file_name = field.file_name().unwrap().to_string();
        let _content_type = field.content_type().unwrap().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        let striped_buf = buf.strip_prefix(entry).unwrap();
        info!("uploading document `{}`(id={}) to account {}", file_name, v4, account_name);

        let content_buf = field.bytes().await.unwrap();

        let striped_path_string = striped_buf.to_string_lossy().to_string();
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
    reload_sender.reload();
    Ok(Created)
}

#[api(group = "account")]
#[debug_handler]
pub async fn get_account_balance_data(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<AccountBalanceHistoryEntity> {
    let account_name = params.0 .0;
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    let vec = operations
        .single_account_all_balances(&account_name)?
        .into_iter()
        .map(|(commodity, balance_history)| {
            let data = balance_history
                .into_iter()
                .map(|(date, amount)| AccountBalanceItemEntity { date, balance: amount })
                .collect_vec();
            (commodity, data)
        })
        .collect();
    ResponseWrapper::json(AccountBalanceHistoryEntity { balance: vec })
}

#[api(group = "account")]
pub async fn get_account_documents(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<Vec<DocumentEntity>> {
    let account_name = params.0 .0;

    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
        .filter(|doc| doc.document_type.match_account(&account_name))
        .cloned()
        .map(|doc| DocumentEntity {
            datetime: doc.datetime.naive_local(),
            filename: doc.filename.unwrap_or_default(),
            path: doc.path,
            extension: None,
            account: doc.document_type.as_account(),
            trx_id: doc.document_type.as_trx(),
        })
        .collect_vec();

    ResponseWrapper::json(rows)
}

#[api(group = "account")]
pub async fn get_account_journals(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<Vec<AccountJournalDomain>> {
    let account_name = params.0 .0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let journals = operations.account_journals(&account_name)?;

    ResponseWrapper::json(journals)
}

#[api(group = "account")]
pub async fn create_account_balance(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, params: Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> ServerResult<Created> {
    let target_account = params.0 .0;
    let ledger = ledger.read().await;
    let rules = Rules::of(&ledger);

    let balance = match payload {
        AccountBalanceRequest::Check { amount } => Directive::BalanceCheck(BalanceCheck {
            date: Date::now(&ledger.options.timezone),
            account: validate::account(&target_account, &rules)?,
            amount: validated(amount, &rules)?,
            tolerance: None,
            meta: Default::default(),
        }),
        AccountBalanceRequest::Pad { amount, pad } => Directive::BalancePad(BalancePad {
            date: Date::now(&ledger.options.timezone),
            account: validate::account(&target_account, &rules)?,
            amount: validated(amount, &rules)?,
            meta: Default::default(),
            pad: validate::account(&pad, &rules)?,
        }),
    };

    ledger.data_source.async_append(&ledger, vec![balance]).await?;
    reload_sender.reload();
    Ok(Created)
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
) -> ServerResult<Created> {
    let ledger = ledger.read().await;
    let rules = Rules::of(&ledger);
    // one time for the whole batch, so the file order decides the order of its directives
    let now = Date::now(&ledger.options.timezone);
    let mut directives = vec![];
    // sub-accounts before their parents, deepest first: a `balance` on a parent covers its sub-accounts, so it
    // must come after their pads to assert, and pad to, the total they leave
    for balance in sub_accounts_first(payload) {
        let balance = match balance {
            BatchAccountBalanceRequest::Check { account_name, amount } => Directive::BalanceCheck(BalanceCheck {
                date: now.clone(),
                account: validate::account(&account_name, &rules)?,
                amount: validated(amount, &rules)?,
                tolerance: None,
                meta: Default::default(),
            }),
            BatchAccountBalanceRequest::Pad { account_name, amount, pad } => Directive::BalancePad(BalancePad {
                date: now.clone(),
                account: validate::account(&account_name, &rules)?,
                amount: validated(amount, &rules)?,
                meta: Default::default(),
                pad: validate::account(&pad, &rules)?,
            }),
        };
        directives.push(balance);
    }

    ledger.data_source.async_append(&ledger, directives).await?;
    reload_sender.reload();
    Ok(Created)
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
