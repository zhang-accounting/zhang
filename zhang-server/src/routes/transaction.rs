use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::Json;
use chrono::NaiveDateTime;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use uuid::Uuid;

use zhang_ast::amount::Amount;
use zhang_ast::{Account, Date, Directive, Flag, Meta, Posting, Transaction, ZhangString};
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
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

    #[derive(Debug)]
    struct JournalHeader {
        id: Uuid,
        sequence: i32,
        datetime: NaiveDateTime,
        journal_type: String,
        payee: String,
        narration: Option<String>,
    }
    let store = operations.read();
    let journal_headers = store
        .transactions
        .values()
        .sorted_by_key(|it| -it.sequence)
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .map(|it| JournalHeader {
            id: it.id,
            sequence: it.sequence,
            datetime: it.datetime.naive_local(),
            journal_type: it.flag.to_string(),
            payee: it.payee.unwrap_or_default(),
            narration: it.narration,
        })
        .collect_vec();

    let header_ids: HashSet<Uuid> = journal_headers.iter().map(|it| it.id).collect();

    let postings = store
        .postings
        .iter()
        .filter(|posting| header_ids.contains(&posting.trx_id))
        .cloned()
        .collect_vec();

    drop(store);
    let mut header_map: HashMap<Uuid, JournalHeader> = journal_headers.into_iter().map(|it| (it.id.to_owned(), it)).collect();

    let mut ret = vec![];
    for (trx_id, arms) in &postings.into_iter().group_by(|it| it.trx_id.to_owned()) {
        let header = header_map.remove(&trx_id).expect("cannot found trx header");
        let item = match header.journal_type.as_str() {
            "BalancePad" => {
                let postings = arms
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
                    id: trx_id,
                    sequence: header.sequence,
                    datetime: header.datetime,
                    payee: header.payee,
                    narration: header.narration,
                    type_: header.journal_type,
                    postings,
                })
            }
            "BalanceCheck" => {
                let postings = arms
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
                    id: trx_id,
                    sequence: header.sequence,
                    datetime: header.datetime,
                    payee: header.payee,
                    narration: header.narration,
                    type_: header.journal_type,
                    postings,
                })
            }
            _ => {
                let postings = arms
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
                let tags = operations.trx_tags(trx_id.to_string())?;
                let links = operations.trx_links(trx_id.to_string())?;
                let metas = operations
                    .metas(MetaType::TransactionMeta, trx_id.to_string())
                    .unwrap()
                    .into_iter()
                    .map(|it| it.into())
                    .collect();
                JournalItemResponse::Transaction(JournalTransactionItemResponse {
                    id: trx_id,
                    sequence: header.sequence,
                    datetime: header.datetime,
                    payee: header.payee,
                    narration: header.narration,
                    tags,
                    links,
                    flag: header.journal_type,
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
