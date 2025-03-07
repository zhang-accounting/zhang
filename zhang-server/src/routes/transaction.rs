use std::str::FromStr;

use axum::extract::{Multipart, Path, State};
use axum::Json;
use gotcha::api;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Date, Directive, Flag, Meta, Posting, SpanInfo, Transaction, ZhangString};
use zhang_core::constants::TXN_ID;
use zhang_core::domains::schemas::MetaType;
use zhang_core::store::TransactionDomain;
use zhang_core::utils::string_::{escape_with_quote, StringExt};

use super::Query;
use crate::request::{CreateTransactionRequest, JournalRequest};
use crate::response::{
    InfoForNewTransaction, JournalBalanceCheckItemResponse, JournalBalancePadItemResponse, JournalItemResponse, JournalTransactionItemResponse,
    JournalTransactionPostingResponse, Pageable, ResponseWrapper,
};
use crate::state::{SharedLedger, SharedReloadSender};
use crate::ApiResult;

#[api(group = "transaction")]
// todo rename api
pub async fn get_info_for_new_transactions(ledger: State<SharedLedger>) -> ApiResult<InfoForNewTransaction> {
    let guard = ledger.read().await;
    let mut operations = guard.operations();

    let all_open_accounts = operations.all_open_accounts()?;
    let account_names = all_open_accounts.into_iter().map(|it| it.name).collect_vec();

    ResponseWrapper::json(InfoForNewTransaction {
        payee: operations.all_payees()?,
        account_name: account_names,
    })
}

#[api(group = "transaction")]
pub async fn get_journals(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.0;

    let store = operations.read();

    let total_count = store
        .transactions
        .values()
        .filter(|it| it.match_keywords(params.keyword.as_ref(), &params.tags, &params.links))
        .count();

    let journals: Vec<TransactionDomain> = store
        .transactions
        .values()
        .filter(|it| it.match_keywords(params.keyword.as_ref(), &params.tags, &params.links))
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
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingResponse::from).collect_vec();
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
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingResponse::from).collect_vec();
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
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingResponse::from).collect_vec();
                let metas = operations
                    .metas(MetaType::TransactionMeta, journal_item.id.to_string())
                    .unwrap()
                    .into_iter()
                    .map(|it| it.into())
                    .collect();
                let has_unbalanced_error = operations
                    .errors_by_meta(TXN_ID, &journal_item.id.to_string())?
                    .iter()
                    .any(|error| error.error_type == ErrorKind::UnbalancedTransaction);

                JournalItemResponse::Transaction(JournalTransactionItemResponse {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    tags: journal_item.tags,
                    links: journal_item.links,
                    flag: journal_item.flag.to_string(),
                    is_balanced: !has_unbalanced_error,
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

#[api(group = "transaction")]
pub async fn create_new_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<String> {
    let ledger = ledger.read().await;

    let mut postings = vec![];
    for posting in payload.postings.into_iter() {
        postings.push(Posting {
            flag: None,
            account: Account::from_str(&posting.account)?,
            units: posting.unit.map(|unit| Amount::new(unit.number, unit.commodity)),
            cost: None,
            price: None,
            comment: None,
        });
    }

    let mut metas = Meta::default();
    for meta in payload.metas {
        metas.insert(meta.key, meta.value.to_quote());
    }
    let time = payload.datetime.with_timezone(&ledger.options.timezone).naive_local();
    let trx = Directive::Transaction(Transaction {
        date: Date::Datetime(time),
        flag: payload.flag.map(|it| it.into()).or(Some(Flag::Okay)),
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

// TODO: handle multipart/form-data
#[api(group = "transaction")]
// todo(refact): use exporter to update transaction
pub async fn upload_transaction_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ApiResult<String> {
    let transaction_id = Uuid::from_str(&path.0 .0).expect("invalid txn id");
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let entry = &ledger.entry.0;
    let mut documents = vec![];

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return ResponseWrapper::bad_request();
    };

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

    let metas_content = documents
        .into_iter()
        .map(|document| format!("  document: {}", escape_with_quote(document.as_str())))
        .join("\n");

    let source_file_path = span_info.source_file.to_string_lossy().to_string();
    let mut content = String::from_utf8(ledger.data_source.async_get(source_file_path.clone()).await?).unwrap();
    content.insert(span_info.span_end, '\n');
    content.insert_str(span_info.span_end + 1, &metas_content);
    ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}

#[api(group = "transaction")]
pub async fn update_single_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<()> {
    let Ok(transaction_id) = Uuid::from_str(&path.0 .0) else {
        return ResponseWrapper::bad_request();
    };
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return ResponseWrapper::bad_request();
    };

    let mut postings = vec![];
    for posting in payload.postings.into_iter() {
        postings.push(Posting {
            flag: None,
            account: Account::from_str(&posting.account)?,
            units: posting.unit.map(|unit| Amount::new(unit.number, unit.commodity)),
            cost: None,
            price: None,
            comment: None,
        });
    }
    let mut metas = Meta::default();
    for meta in payload.metas {
        metas.insert(meta.key, meta.value.to_quote());
    }
    let time = payload.datetime.with_timezone(&ledger.options.timezone).naive_local();
    let trx = Directive::Transaction(Transaction {
        date: Date::Datetime(time),
        flag: payload.flag.map(|it| it.into()).or(Some(Flag::Okay)),
        payee: Some(payload.payee.to_quote()),
        narration: payload.narration.map(|it| it.to_quote()),
        tags: IndexSet::from_iter(payload.tags.into_iter()),
        links: IndexSet::from_iter(payload.links.into_iter()),
        postings,
        meta: metas,
    });
    let txn_content = ledger.data_source.export(trx)?;
    let trx_content = String::from_utf8_lossy(&txn_content);
    let source_file_path = span_info.source_file.to_string_lossy().to_string();

    let mut content = String::from_utf8(ledger.data_source.async_get(source_file_path.clone()).await?).unwrap();
    content.replace_by_span(&SpanInfo::simple(span_info.span_start, span_info.span_end), &trx_content);

    ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
    reload_sender.reload();
    ResponseWrapper::json(())
}
