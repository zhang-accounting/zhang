use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::Json;
use chrono::Utc;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Document, ZhangString};
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_core::utils::calculable::Calculable;

use crate::request::AccountBalanceRequest;
use crate::response::{AccountInfoResponse, AccountResponse, DocumentResponse, ResponseWrapper};
use crate::{ApiResult, ReloadSender};

pub async fn get_account_list(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<AccountResponse>> {
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let mut ret = vec![];
    for account in operations.all_accounts()? {
        let account_domain = operations.account(&account)?.expect("cannot find account");
        let account_balances = operations
            .single_account_balances(&account)?
            .into_iter()
            .map(|balance| Amount::new(balance.balance_number, balance.balance_commodity))
            .collect_vec();
        let amount = account_balances.calculate(Utc::now().with_timezone(timezone), &mut operations)?;

        ret.push(AccountResponse {
            name: account,
            status: account_domain.status,
            alias: account_domain.alias,
            amount,
        });
    }
    ResponseWrapper::json(ret)
}

pub async fn get_account_info(ledger: State<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> ApiResult<AccountInfoResponse> {
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
        .single_account_balances(&account_info.name)?
        .into_iter()
        .map(|balance| Amount::new(balance.balance_number, balance.balance_commodity))
        .collect_vec();
    let amount = vec.calculate(Utc::now().with_timezone(timezone), &mut operations)?;

    ResponseWrapper::json(AccountInfoResponse {
        date: account_info.date,
        r#type: account_info.r#type,
        name: account_info.name,
        status: account_info.status,
        alias: account_info.alias,
        amount,
    })
}

pub async fn upload_account_document(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, path: Path<(String,)>, mut multipart: Multipart,
) -> ApiResult<()> {
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
            account: Account::from_str(&account_name)?,
            filename: ZhangString::QuoteString(striped_path_string),
            tags: None,
            links: None,
            meta: Default::default(),
        }));
    }

    ledger_stage.data_source.async_append(&ledger_stage, documents).await?;
    reload_sender.reload();
    ResponseWrapper::<()>::created()
}

pub async fn get_account_documents(ledger: State<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<Vec<DocumentResponse>> {
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

pub async fn get_account_journals(ledger: State<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<Vec<AccountJournalDomain>> {
    let account_name = params.0 .0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let journals = operations.account_journals(&account_name)?;

    ResponseWrapper::json(journals)
}

pub async fn create_account_balance(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, params: Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> ApiResult<()> {
    let target_account = params.0 .0;
    let ledger = ledger.read().await;

    let balance = match payload {
        AccountBalanceRequest::Check { amount, .. } => Directive::BalanceCheck(BalanceCheck {
            date: Date::now(&ledger.options.timezone),
            account: Account::from_str(&target_account)?,
            amount: Amount {
                number: amount.number,
                currency: amount.commodity,
            },
            meta: Default::default(),
        }),
        AccountBalanceRequest::Pad { amount, pad, .. } => Directive::BalancePad(BalancePad {
            date: Date::now(&ledger.options.timezone),
            account: Account::from_str(&target_account)?,
            amount: Amount {
                number: amount.number,
                currency: amount.commodity,
            },
            meta: Default::default(),
            pad: Account::from_str(&pad)?,
        }),
    };

    ledger.data_source.async_append(&ledger, vec![balance]).await.unwrap();
    reload_sender.reload();
    ResponseWrapper::<()>::created()
}

pub async fn create_batch_account_balances(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, Json(payload): Json<Vec<AccountBalanceRequest>>,
) -> ApiResult<()> {
    let ledger = ledger.read().await;
    let mut directives = vec![];
    for balance in payload {
        let balance = match balance {
            AccountBalanceRequest::Check { account_name, amount } => Directive::BalanceCheck(BalanceCheck {
                date: Date::now(&ledger.options.timezone),
                account: Account::from_str(&account_name)?,
                amount: Amount {
                    number: amount.number,
                    currency: amount.commodity,
                },
                meta: Default::default(),
            }),
            AccountBalanceRequest::Pad { account_name, amount, pad } => Directive::BalancePad(BalancePad {
                date: Date::now(&ledger.options.timezone),
                account: Account::from_str(&account_name)?,
                amount: Amount {
                    number: amount.number,
                    currency: amount.commodity,
                },
                meta: Default::default(),
                pad: Account::from_str(&pad)?,
            }),
        };
        directives.push(balance);
    }

    ledger.data_source.async_append(&ledger, directives).await?;
    reload_sender.reload();
    ResponseWrapper::<()>::created()
}
