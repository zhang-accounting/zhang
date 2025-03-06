use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use gotcha::api;
use axum::extract::{Multipart, Path, State};
use axum::{debug_handler, Json};
use chrono::Utc;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Currency, Date, Directive, Document, ZhangString};
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_core::utils::calculable::Calculable;

use crate::request::AccountBalanceRequest;
use crate::response::{AccountBalanceItemResponse, AccountInfoResponse, AccountJournalEntity, AccountResponse, AmountResponse, Created, DocumentResponse, ResponseWrapper};
use crate::{ApiResult, LedgerState, ServerResult};
use crate::state::{SharedLedger, SharedReloadSender};

#[api(group = "account")]
pub async fn get_account_list(ledger: State<SharedLedger>) -> ApiResult<Vec<AccountResponse>> {
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let mut ret = vec![];
    for account in operations.all_accounts()? {
        let account_domain = operations.account(&account)?.expect("cannot find account");
        let account_balances = operations
            .single_account_latest_balances(&account)?
            .into_iter()
            .map(|balance| Amount::new(balance.balance_number, balance.balance_commodity))
            .collect_vec();
        let amount = account_balances
            .calculate(Utc::now().with_timezone(timezone), &mut operations)?
            .persist_commodity(&ledger.options.operating_currency);

        ret.push(AccountResponse {
            name: account,
            status: account_domain.status.into(),
            alias: account_domain.alias,
            amount: amount.into(),
        });
    }
    ResponseWrapper::json(ret)
}

#[api(group = "account")]
pub async fn get_account_info(ledger: State<SharedLedger>, path: Path<(String,)>) -> ApiResult<AccountInfoResponse> {
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
        .map(|balance| Amount::new(balance.balance_number, balance.balance_commodity))
        .collect_vec();
    let amount = vec
        .calculate(Utc::now().with_timezone(timezone), &mut operations)?
        .persist_commodity(&ledger.options.operating_currency);

    ResponseWrapper::json(AccountInfoResponse {
        date: account_info.date,
        r#type: account_info.r#type,
        name: account_info.name,
        status: account_info.status.into(),
        alias: account_info.alias,
        amount: amount.into(),
    })
}

#[api(group = "account")]
pub async fn upload_account_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ServerResult<Created> {
    let account_name = path.0 .0;
    let ledger_stage = ledger.read().await;
    let entry = &ledger_stage.entry.0;
    let mut documents = vec![];

    while let Some(field) = multipart.next_field().await.unwrap() {
        let _name = field.name().unwrap().to_string();
        let file_name = field.file_name().unwrap().to_string();
        let _content_type = field.content_type().unwrap().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        let striped_buf = buf.strip_prefix(entry).unwrap();
        info!("uploading document `{}`(id={}) to account {}", file_name, &v4.to_string(), &account_name);

        let content_buf = field.bytes().await.unwrap();

        let striped_path_string = striped_buf.to_string_lossy().to_string();
        ledger_stage
            .data_source
            .async_save(&ledger_stage, striped_path_string.to_owned(), &content_buf)
            .await?;

        documents.push(Directive::Document(Document {
            date: Date::now(&ledger_stage.options.timezone),
            account: Account::from_str(&account_name)?.into(),
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
pub async fn get_account_balance_data(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<HashMap<Currency, Vec<AccountBalanceItemResponse>>> {
    let account_name = params.0 .0;
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    let vec = operations.single_account_all_balances(&account_name)?;
    ResponseWrapper::json(
        vec.into_iter()
            .map(|(commodity, balance_history)| {
                let data = balance_history
                    .into_iter()
                    .map(|(date, amount)| AccountBalanceItemResponse {
                        date,
                        balance: AmountResponse {
                            number: amount.number,
                            commodity: amount.currency,
                        },
                    })
                    .collect_vec();
                (commodity, data)
            })
            .collect(),
    )
}

#[api(group = "account")]
pub async fn get_account_documents(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<Vec<DocumentResponse>> {
    let account_name = params.0 .0;

    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
        .filter(|doc| doc.document_type.match_account(&account_name))
        .cloned()
        .map(|doc| DocumentResponse {
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
pub async fn get_account_journals(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<Vec<AccountJournalEntity>> {
    let account_name = params.0 .0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let journals = operations.account_journals(&account_name)?.into_iter().map(|it| it.into()).collect_vec();

    ResponseWrapper::json(journals)
}

#[api(group = "account")]
pub async fn create_account_balance(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, params: Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> ServerResult<Created> {
    let target_account = params.0 .0;
    let ledger = ledger.read().await;

    let balance = match payload {
        AccountBalanceRequest::Check { amount, .. } => Directive::BalanceCheck(BalanceCheck {
            date: Date::now(&ledger.options.timezone),
            account: Account::from_str(&target_account)?.into(),
            amount: Amount {
                number: amount.number,
                currency: amount.commodity,
            },
            meta: Default::default(),
        }),
        AccountBalanceRequest::Pad { amount, pad, .. } => Directive::BalancePad(BalancePad {
            date: Date::now(&ledger.options.timezone),
            account: Account::from_str(&target_account)?.into(),
            amount: Amount {
                number: amount.number,
                currency: amount.commodity,
            },
            meta: Default::default(),
            pad: Account::from_str(&pad)?.into(),
        }),
    };

    ledger.data_source.async_append(&ledger, vec![balance]).await.unwrap();
    reload_sender.reload();
    Ok(Created)
}

#[api(group = "account")]
pub async fn create_batch_account_balances(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Json(payload): Json<Vec<AccountBalanceRequest>>,
) -> ServerResult<Created> {
    let ledger = ledger.read().await;
    let mut directives = vec![];
    for balance in payload {
        let balance = match balance {
            AccountBalanceRequest::Check { account_name, amount } => Directive::BalanceCheck(BalanceCheck {
                date: Date::now(&ledger.options.timezone),
                account: Account::from_str(&account_name)?.into(),
                amount: Amount {
                    number: amount.number,
                    currency: amount.commodity,
                },
                meta: Default::default(),
            }),
            AccountBalanceRequest::Pad { account_name, amount, pad } => Directive::BalancePad(BalancePad {
                date: Date::now(&ledger.options.timezone),
                account: Account::from_str(&account_name)?.into(),
                amount: Amount {
                    number: amount.number,
                    currency: amount.commodity,
                },
                meta: Default::default(),
                pad: Account::from_str(&pad)?.into(),
            }),
        };
        directives.push(balance);
    }

    ledger.data_source.async_append(&ledger, directives).await?;
    reload_sender.reload();
    Ok(Created)
}
