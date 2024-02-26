use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::Json;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use uuid::Uuid;

use zhang_ast::amount::Amount;
use zhang_ast::{Account, Date, Directive, Flag, Meta, Posting, Transaction, ZhangString};
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::store::TransactionDomain;
use zhang_core::utils::string_::StringExt;

use crate::request::{CreateTransactionRequest, JournalRequest};
use crate::response::{
    InfoForNewTransaction, JournalBalanceCheckItemResponse, JournalBalancePadItemResponse, JournalItemResponse, JournalTransactionItemResponse,
    JournalTransactionPostingResponse, Pageable, ResponseWrapper,
};
use crate::{ApiResult, ReloadSender};

// todo rename api
pub async fn get_info_for_new_transactions(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<InfoForNewTransaction> {
    let guard = ledger.read().await;
    let mut operations = guard.operations();

    let all_open_accounts = operations.all_open_accounts()?;
    let account_names = all_open_accounts.into_iter().map(|it| it.name).collect_vec();

    ResponseWrapper::json(InfoForNewTransaction {
        payee: operations.all_payees()?,
        account_name: account_names,
    })
}

pub async fn get_journals(ledger: State<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.0;

    let total_count = operations.transaction_counts()?;

    let store = operations.read();

    let journals: Vec<TransactionDomain> = store
        .transactions
        .values()
        .filter(|it| params.keyword.as_ref().map(|keyword| it.contains_keyword(keyword)).unwrap_or(true))
        .sorted_by_key(|it| -it.sequence)
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .collect_vec();

    drop(store);
    let mut ret = vec![];
    for journal_item in journals {
        let item = match journal_item.flag {
            Flag::BalancePad => {
                let postings = journal_item
                    .postings
                    .into_iter()
                    .map(|arm| JournalTransactionPostingResponse {
                        account: arm.account.name().to_owned(),
                        unit_number: arm.unit.as_ref().map(|it| it.number.clone()),
                        unit_commodity: arm.unit.as_ref().map(|it| it.currency.clone()),
                        cost_number: arm.cost.as_ref().map(|it| it.number.clone()),
                        cost_commodity: arm.cost.as_ref().map(|it| it.currency.clone()),
                        inferred_unit_number: arm.inferred_amount.number,
                        inferred_unit_commodity: arm.inferred_amount.currency,
                        account_before_number: arm.previous_amount.number,
                        account_before_commodity: arm.previous_amount.currency,
                        account_after_number: arm.after_amount.number,
                        account_after_commodity: arm.after_amount.currency,
                    })
                    .collect_vec();
                JournalItemResponse::BalancePad(JournalBalancePadItemResponse {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    type_: journal_item.flag.to_string(),
                    postings,
                })
            }
            Flag::BalanceCheck => {
                let postings = journal_item
                    .postings
                    .into_iter()
                    .map(|arm| JournalTransactionPostingResponse {
                        account: arm.account.name().to_owned(),
                        unit_number: arm.unit.as_ref().map(|it| it.number.clone()),
                        unit_commodity: arm.unit.as_ref().map(|it| it.currency.clone()),
                        cost_number: arm.cost.as_ref().map(|it| it.number.clone()),
                        cost_commodity: arm.cost.as_ref().map(|it| it.currency.clone()),
                        inferred_unit_number: arm.inferred_amount.number,
                        inferred_unit_commodity: arm.inferred_amount.currency,
                        account_before_number: arm.previous_amount.number,
                        account_before_commodity: arm.previous_amount.currency,
                        account_after_number: arm.after_amount.number,
                        account_after_commodity: arm.after_amount.currency,
                    })
                    .collect_vec();
                JournalItemResponse::BalanceCheck(JournalBalanceCheckItemResponse {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    type_: journal_item.flag.to_string(),
                    postings,
                })
            }
            _ => {
                let postings = journal_item
                    .postings
                    .into_iter()
                    .map(|arm| JournalTransactionPostingResponse {
                        account: arm.account.name().to_owned(),
                        unit_number: arm.unit.as_ref().map(|it| it.number.clone()),
                        unit_commodity: arm.unit.as_ref().map(|it| it.currency.clone()),
                        cost_number: arm.cost.as_ref().map(|it| it.number.clone()),
                        cost_commodity: arm.cost.as_ref().map(|it| it.currency.clone()),
                        inferred_unit_number: arm.inferred_amount.number,
                        inferred_unit_commodity: arm.inferred_amount.currency,
                        account_before_number: arm.previous_amount.number,
                        account_before_commodity: arm.previous_amount.currency,
                        account_after_number: arm.after_amount.number,
                        account_after_commodity: arm.after_amount.currency,
                    })
                    .collect_vec();
                let tags = operations.trx_tags(journal_item.id.to_string())?;
                let links = operations.trx_links(journal_item.id.to_string())?;
                let metas = operations
                    .metas(MetaType::TransactionMeta, journal_item.id.to_string())
                    .unwrap()
                    .into_iter()
                    .map(|it| it.into())
                    .collect();
                JournalItemResponse::Transaction(JournalTransactionItemResponse {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    tags,
                    links,
                    flag: journal_item.flag.to_string(),
                    is_balanced: true,
                    postings,
                    metas,
                })
            }
        };
        ret.push(item);
    }
    ret.sort_by_key(|item| item.sequence());
    ret.reverse();
    ResponseWrapper::json(Pageable::new(total_count as u32, params.page(), params.limit(), ret))
}

pub async fn create_new_transaction(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<String> {
    let ledger = ledger.read().await;

    let mut postings = vec![];
    for posting in payload.postings.into_iter() {
        postings.push(Posting {
            flag: None,
            account: Account::from_str(&posting.account)?,
            units: posting.unit.map(|unit| Amount::new(unit.number, unit.commodity)),
            cost: None,
            cost_date: None,
            price: None,
            comment: None,
            meta: Default::default(),
        });
    }

    let mut metas = Meta::default();
    for meta in payload.metas {
        metas.insert(meta.key, meta.value.to_quote());
    }
    let time = payload.datetime.with_timezone(&ledger.options.timezone).naive_local();
    let trx = Directive::Transaction(Transaction {
        date: Date::Datetime(time),
        flag: Some(Flag::Okay),
        payee: Some(payload.payee.to_quote()),
        narration: payload.narration.map(|it| it.to_quote()),
        tags: IndexSet::from_iter(payload.tags.into_iter()),
        links: IndexSet::from_iter(payload.links.into_iter()),
        postings,
        meta: metas,
    });

    ledger.data_source.async_append(&ledger, vec![trx]).await?;
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}

// todo(refact): use exporter to update transaction
pub async fn upload_transaction_document(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, path: Path<(String,)>, mut multipart: Multipart,
) -> ApiResult<String> {
    let transaction_id = path.0 .0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let entry = &ledger.entry.0;
    let mut documents = vec![];

    while let Some(field) = multipart.next_field().await.unwrap() {
        let _name = field.name().unwrap().to_string();
        let file_name = field.file_name().unwrap().to_string();
        let _content_type = field.content_type().unwrap().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        let striped_buf = buf.strip_prefix(entry).unwrap();
        let striped_path_string = striped_buf.to_string_lossy().to_string();
        info!("uploading document `{}`(id={}) to transaction {}", file_name, &v4.to_string(), &transaction_id);
        let content_buf = field.bytes().await.unwrap();

        ledger.data_source.async_save(&ledger, striped_path_string, &content_buf).await?;

        let path = match buf.strip_prefix(entry) {
            Ok(relative_path) => relative_path.to_str().unwrap(),
            Err(_) => buf.to_str().unwrap(),
        };

        documents.push(ZhangString::QuoteString(path.to_string()));
    }
    let span_info = operations.transaction_span(&transaction_id)?;
    let metas_content = documents
        .into_iter()
        .map(|document| format!("  document: {}", document.to_plain_string()))
        .join("\n");

    let source_file_path = span_info.source_file.to_string_lossy().to_string();
    let mut content = String::from_utf8(ledger.data_source.async_get(source_file_path.clone()).await?).unwrap();
    content.insert(span_info.span_end, '\n');
    content.insert_str(span_info.span_end + 1, &metas_content);
    ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}
