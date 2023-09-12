use std::fs::File;
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::web::{Data, Json, Path};
use actix_web::{get, post};
use futures_util::StreamExt;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Document, ZhangString};
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::exporter::AppendableExporter;
use zhang_core::ledger::Ledger;

use crate::request::AccountBalanceRequest;
use crate::response::{AccountInfoResponse, AccountResponse, DocumentResponse, ResponseWrapper};
use crate::{routes, ApiResult};

#[get("/api/accounts")]
pub async fn get_account_list(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<AccountResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let mut ret = vec![];
    for account in operations.all_accounts()? {
        let account_domain = operations.account(&account)?.expect("cannot find account");
        let account_balances = operations.single_account_balances(&account)?;
        let amount = routes::group_and_calculate(&mut operations, account_balances)?;

        ret.push(AccountResponse {
            name: account,
            status: account_domain.status,
            alias: account_domain.alias,
            amount,
        });
    }
    ResponseWrapper::json(ret)
}

#[get("/api/accounts/{account_name}")]
pub async fn get_account_info(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> ApiResult<AccountInfoResponse> {
    let account_name = path.into_inner().0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let account_domain = operations.account(&account_name)?;

    let account_info = match account_domain {
        Some(info) => info,
        None => return ResponseWrapper::not_found(),
    };
    let vec = operations.single_account_balances(&account_info.name)?;
    let amount = routes::group_and_calculate(&mut operations, vec)?;

    ResponseWrapper::json(AccountInfoResponse {
        date: account_info.date,
        r#type: account_info.r#type,
        name: account_info.name,
        status: account_info.status,
        alias: account_info.alias,
        amount,
    })
}

#[post("/api/accounts/{account_name}/documents")]
pub async fn upload_account_document(
    ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: Path<(String,)>, exporter: Data<dyn AppendableExporter>,
) -> ApiResult<()> {
    let account_name = path.into_inner().0;
    let ledger_stage = ledger.read().await;
    let entry = &ledger_stage.entry.0;
    let mut documents = vec![];

    while let Some(item) = multipart.next().await {
        let mut field = item.unwrap();
        let _name = field.name().to_string();
        let file_name = field.content_disposition().get_filename().unwrap().to_string();
        let _content_type = field.content_type().type_().as_str().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        info!("uploading document `{}`(id={}) to account {}", file_name, &v4.to_string(), &account_name);
        routes::create_folder_if_not_exist(&buf);
        let mut f = File::create(&buf).expect("Unable to create file");
        while let Some(chunk) = field.next().await {
            let data = chunk.unwrap();
            f.write_all(&data).expect("cannot wirte content");
        }
        let path = match buf.strip_prefix(entry) {
            Ok(relative_path) => relative_path.to_str().unwrap(),
            Err(_) => buf.to_str().unwrap(),
        };

        documents.push(Directive::Document(Document {
            date: Date::now(&ledger_stage.options.timezone),
            account: Account::from_str(&account_name)?,
            filename: ZhangString::QuoteString(path.to_string()),
            tags: None,
            links: None,
            meta: Default::default(),
        }));
    }

    exporter.as_ref().append_directives(&ledger_stage, documents)?;

    ResponseWrapper::<()>::created()
}

#[get("/api/accounts/{account_name}/documents")]
pub async fn get_account_documents(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<Vec<DocumentResponse>> {
    let account_name = params.into_inner().0;

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

#[get("/api/accounts/{account_name}/journals")]
pub async fn get_account_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<Vec<AccountJournalDomain>> {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let journals = operations.account_journals(&account_name)?;

    ResponseWrapper::json(journals)
}

#[post("/api/accounts/{account_name}/balances")]
pub async fn create_account_balance(
    ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>, Json(payload): Json<AccountBalanceRequest>, exporter: Data<dyn AppendableExporter>,
) -> ApiResult<()> {
    let target_account = params.into_inner().0;
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

    exporter.as_ref().append_directives(&ledger, vec![balance])?;
    ResponseWrapper::<()>::created()
}

#[post("/api/accounts/batch-balances")]
pub async fn create_batch_account_balances(
    ledger: Data<Arc<RwLock<Ledger>>>, Json(payload): Json<Vec<AccountBalanceRequest>>, exporter: Data<dyn AppendableExporter>,
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

    exporter.as_ref().append_directives(&ledger, directives)?;
    ResponseWrapper::<()>::created()
}
