use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::iter::FromIterator;
use std::ops::{Add, AddAssign, Div, Mul};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{get, post, put, Responder};
use bigdecimal::{BigDecimal, Zero};
use chrono::{Local, NaiveDate, NaiveDateTime};
use futures_util::StreamExt;
use glob::glob;
use indexmap::IndexSet;
use itertools::Itertools;
use log::{error, info};
use tokio::sync::RwLock;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, AccountType, Balance, BalanceCheck, BalancePad, Date, Directive, Document, Flag, Meta, Posting, Transaction, ZhangString};
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::ledger::Ledger;
use zhang_core::utils::date_range::NaiveDateRange;
use zhang_core::utils::string_::StringExt;

use crate::broadcast::Broadcaster;
use crate::request::{AccountBalanceRequest, CreateTransactionRequest, FileUpdateRequest, JournalRequest, ReportRequest, StatisticRequest};
use crate::response::{
    AccountInfoResponse, AccountResponse, AmountResponse, BasicInfo, CalculatedAmount, CommodityDetailResponse, CommodityListItemResponse, CommodityLot,
    CommodityPrice, CurrentStatisticResponse, DocumentResponse, FileDetailResponse, InfoForNewTransaction, JournalBalanceCheckItemResponse,
    JournalBalancePadItemResponse, JournalItemResponse, JournalTransactionItemResponse, JournalTransactionPostingResponse, Pageable, ReportRankItemResponse,
    ReportResponse, ResponseWrapper, StatisticResponse,
};
use crate::{ApiResult, ServerResult};

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

#[get("/api/sse")]
pub async fn sse(broadcaster: Data<Broadcaster>) -> impl Responder {
    broadcaster.new_client().await
}

#[get("/api/info")]
pub async fn get_basic_info(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<BasicInfo> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    ResponseWrapper::json(BasicInfo {
        title: operations.option("title")?.map(|it| it.value),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
    })
}

// todo rename api
#[get("/api/for-new-transaction")]
pub async fn get_info_for_new_transactions(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<InfoForNewTransaction> {
    let guard = ledger.read().await;
    let mut operations = guard.operations();

    let all_open_accounts = operations.all_open_accounts()?;
    let account_names = all_open_accounts.into_iter().map(|it| it.name).collect_vec();

    ResponseWrapper::json(InfoForNewTransaction {
        payee: operations.all_payees()?,
        account_name: account_names,
    })
}

#[get("/api/statistic")]
pub async fn get_statistic_data(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<StatisticRequest>) -> ApiResult<StatisticResponse> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.into_inner();

    let rows = operations.static_duration(params.from, params.to)?;

    // 构建每日的统计数据
    let mut ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();
    for (date, dated_rows) in &rows.into_iter().group_by(|row| row.date) {
        let date_entry = ret.entry(date).or_insert_with(HashMap::new);
        for row in dated_rows {
            date_entry.insert(
                row.account_type,
                AmountResponse {
                    number: row.amount,
                    commodity: row.commodity,
                },
            );
        }
    }

    // 补充不存在的日期
    for day in NaiveDateRange::new(params.from.date_naive(), params.to.date_naive()) {
        ret.entry(day).or_insert_with(HashMap::new);
    }

    let accounts = operations.all_accounts()?;

    let mut existing_balances: HashMap<String, AmountResponse> = HashMap::default();
    for account in accounts {
        let balance = operations.account_target_date_balance(&account, params.from)?.into_iter().next();
        if let Some(balance) = balance {
            existing_balances.insert(
                account,
                AmountResponse {
                    number: balance.balance_number,
                    commodity: balance.balance_commodity,
                },
            );
        }
    }

    let detail_ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();

    ResponseWrapper::json(StatisticResponse {
        changes: ret,
        details: detail_ret,
    })
}

#[get("/api/statistic/current")]
pub async fn current_statistic(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<CurrentStatisticResponse> {
    let ledger = ledger.read().await;

    let mut operations = ledger.operations();

    let latest_account_balances = operations.accounts_latest_balance()?;

    let balances = group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )?;

    let liability = group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )?;

    struct CurrentMonthBalance {
        account_type: String,
        amount: BigDecimal,
        commodity: String,
    }

    let current_month_balance = operations
        .accounts_latest_balance()?
        .into_iter()
        .map(|balance| CurrentMonthBalance {
            // todo use Account constructor
            account_type: balance.account.split(':').next().unwrap().to_owned(),
            amount: balance.balance_number,
            commodity: balance.balance_commodity,
        })
        .collect_vec();

    let income = current_month_balance
        .iter()
        .find(|it| it.account_type.eq("Income"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });
    let expense = current_month_balance
        .iter()
        .find(|it| it.account_type.eq("Expenses"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    ResponseWrapper::json(CurrentStatisticResponse {
        balance: balances,
        liability,
        income,
        expense,
    })
}

fn group_and_calculate<T: AmountLike>(operations: &mut Operations, latest_account_balances: Vec<T>) -> ZhangResult<CalculatedAmount> {
    let operating_currency = operations
        .option(KEY_OPERATING_CURRENCY)?
        .ok_or(ZhangError::OptionNotFound(KEY_OPERATING_CURRENCY.to_owned()))?
        .value;

    let mut total_sum = BigDecimal::zero();

    let mut detail = HashMap::new();
    for (commodity, values) in &latest_account_balances.into_iter().group_by(|it| it.commodity().to_owned()) {
        let commodity_sum = values.fold(BigDecimal::zero(), |acc, item| acc.add(item.number()));

        if commodity.eq(&operating_currency) {
            total_sum.add_assign(&commodity_sum);
        } else {
            let target_price = operations.get_price(Local::now().naive_local(), &commodity, &operating_currency)?;
            if let Some(price) = target_price {
                total_sum.add_assign((&commodity_sum).mul(price.amount));
            }
        }
        detail.insert(commodity, commodity_sum);
    }
    Ok(CalculatedAmount {
        calculated: AmountResponse {
            number: total_sum,
            commodity: operating_currency.to_owned(),
        },
        detail,
    })
}

#[get("/api/journals")]
pub async fn get_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.into_inner();

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

    let postings = store.postings.iter().filter(|posting| header_ids.contains(&posting.id)).cloned().collect_vec();

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

#[post("/api/transactions")]
pub async fn create_new_transaction(
    ledger: Data<Arc<RwLock<Ledger>>>, Json(payload): Json<CreateTransactionRequest>, exporter: Data<dyn AppendableExporter>,
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
    exporter.as_ref().append_directives(&ledger, vec![trx])?;

    ResponseWrapper::json("Ok".to_string())
}

// todo(refact): use exporter to update transaction
#[post("/api/transactions/{transaction_id}/documents")]
pub async fn upload_transaction_document(ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: Path<(String,)>) -> ApiResult<String> {
    let transaction_id = path.into_inner().0;
    let ledger_stage = ledger.read().await;
    let mut operations = ledger_stage.operations();
    let entry = &ledger_stage.entry.0;
    let mut documents = vec![];

    while let Some(item) = multipart.next().await {
        let mut field = item.unwrap();
        let _name = field.name().to_string();
        let file_name = field.content_disposition().get_filename().unwrap().to_string();
        let _content_type = field.content_type().type_().as_str().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        info!("uploading document `{}`(id={}) to transaction {}", file_name, &v4.to_string(), &transaction_id);
        create_folder_if_not_exist(&buf);
        let mut f = File::create(&buf).expect("Unable to create file");
        while let Some(chunk) = field.next().await {
            let data = chunk.unwrap();
            f.write_all(&data).expect("cannot wirte content");
        }
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
    insert_line(span_info.source_file, &metas_content, span_info.span_end)?;
    // todo add update method in exporter
    ResponseWrapper::json("Ok".to_string())
}

#[get("/api/documents/{file_path}")]
pub async fn download_document(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> impl Responder {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(filename);

    NamedFile::open_async(full_path).await
}

#[cfg(feature = "frontend")]
pub async fn serve_frontend(uri: actix_web::http::Uri) -> impl Responder {
    let path = uri.path().trim_start_matches('/').to_string();
    let buf = PathBuf::from_str(&path).unwrap();
    if buf.extension().is_some() {
        StaticFile(path)
    } else {
        StaticFile("index.html".to_string())
    }
}

#[get("/api/accounts")]
pub async fn get_account_list(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<AccountResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let mut ret = vec![];
    for account in operations.all_accounts()? {
        let account_domain = operations.account(&account)?.expect("cannot find account");
        let account_balances = operations.single_account_balances(&account)?;
        let amount = group_and_calculate(&mut operations, account_balances)?;

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
    let amount = group_and_calculate(&mut operations, vec)?;

    ResponseWrapper::json(AccountInfoResponse {
        date: account_info.date,
        r#type: account_info.r#type,
        name: account_info.name,
        status: account_info.status,
        alias: account_info.alias,
        amount,
    })
}

#[get("/api/documents")]
pub async fn get_documents(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<DocumentResponse>> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
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
        create_folder_if_not_exist(&buf);
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
        AccountBalanceRequest::Check { amount, .. } => Balance::BalanceCheck(BalanceCheck {
            date: Date::now(&ledger.options.timezone),
            account: Account::from_str(&target_account)?,
            amount: Amount {
                number: amount.number,
                currency: amount.commodity,
            },
            meta: Default::default(),
        }),
        AccountBalanceRequest::Pad { amount, pad, .. } => Balance::BalancePad(BalancePad {
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

    exporter.as_ref().append_directives(&ledger, vec![Directive::Balance(balance)])?;
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
            AccountBalanceRequest::Check { account_name, amount } => Balance::BalanceCheck(BalanceCheck {
                date: Date::now(&ledger.options.timezone),
                account: Account::from_str(&account_name)?,
                amount: Amount {
                    number: amount.number,
                    currency: amount.commodity,
                },
                meta: Default::default(),
            }),
            AccountBalanceRequest::Pad { account_name, amount, pad } => Balance::BalancePad(BalancePad {
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
        directives.push(Directive::Balance(balance));
    }

    exporter.as_ref().append_directives(&ledger, directives)?;
    ResponseWrapper::<()>::created()
}

#[get("/api/commodities")]
pub async fn get_all_commodities(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<CommodityListItemResponse>> {
    let ledger = ledger.read().await;

    let operations = ledger.operations();
    let operating_currency = ledger.options.operating_currency.as_str();
    let store = operations.read();
    let mut ret = vec![];
    for commodity in store.commodities.values().cloned() {
        let commodity: CommodityDomain = commodity;
        let latest_price = operations.get_latest_price(&commodity.name, operating_currency)?;

        let amount = operations.get_commodity_balances(&commodity.name)?;

        ret.push(CommodityListItemResponse {
            name: commodity.name,
            precision: commodity.precision,
            prefix: commodity.prefix,
            suffix: commodity.suffix,
            rounding: commodity.rounding,
            total_amount: amount,
            latest_price_date: latest_price.as_ref().map(|it| it.datetime),
            latest_price_amount: latest_price.as_ref().map(|it| it.amount.clone()),
            latest_price_commodity: latest_price.map(|it| it.commodity),
        });
    }

    ResponseWrapper::json(ret)
}

#[get("/api/commodities/{commodity_name}")]
pub async fn get_single_commodity(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<CommodityDetailResponse> {
    let commodity_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let operating_currency = ledger.options.operating_currency.clone();

    let mut operations = ledger.operations();
    let commodity = operations.commodity(&commodity_name)?.expect("cannot find commodity");
    let latest_price = operations.get_latest_price(&commodity_name, operating_currency)?;

    let amount = operations.get_commodity_balances(&commodity_name)?;
    let commodity_item = CommodityListItemResponse {
        name: commodity.name,
        precision: commodity.precision,
        prefix: commodity.prefix,
        suffix: commodity.suffix,
        rounding: commodity.rounding,
        total_amount: amount,
        latest_price_date: latest_price.as_ref().map(|it| it.datetime),
        latest_price_amount: latest_price.as_ref().map(|it| it.amount.clone()),
        latest_price_commodity: latest_price.map(|it| it.commodity),
    };

    let lots = operations
        .commodity_lots(&commodity_name)?
        .into_iter()
        .map(|it| CommodityLot {
            datetime: it.datetime.map(|date| date.naive_local()),
            amount: it.amount,
            price_amount: it.price.as_ref().map(|price| price.number.clone()),
            price_commodity: it.price.as_ref().map(|price| price.currency.clone()),
            account: it.account.name().to_owned(),
        })
        .collect_vec();

    let prices = operations
        .commodity_prices(&commodity_name)?
        .into_iter()
        .map(|price| CommodityPrice {
            datetime: price.datetime,
            amount: price.amount,
            target_commodity: Some(price.target_commodity),
        })
        .collect_vec();

    ResponseWrapper::json(CommodityDetailResponse {
        info: commodity_item,
        lots,
        prices,
    })
}

#[get("/api/files")]
pub async fn get_files(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<Option<String>>> {
    let ledger = ledger.read().await;
    let entry_path = &ledger.entry.0;

    let mut ret = vec![];
    for patten in &ledger.visited_files {
        for entry in glob(patten.as_str()).unwrap() {
            match entry {
                Ok(path) => {
                    let p = path.strip_prefix(entry_path).unwrap().to_str().map(|it| it.to_string());
                    ret.push(p);
                }
                Err(e) => error!("{:?}", e),
            }
        }
    }
    ResponseWrapper::json(ret)
}

#[get("/api/files/{file_path}")]
pub async fn get_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> ApiResult<FileDetailResponse> {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    ResponseWrapper::json(FileDetailResponse {
        path: filename,
        content: std::fs::read_to_string(full_path)?,
    })
}
#[put("/api/files/{file_path}")]
pub async fn update_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>, Json(payload): Json<FileUpdateRequest>) -> ApiResult<()> {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    // todo(refact) test the syntax valid
    // if parse_zhang(&payload.content, None).is_ok() {
    std::fs::write(full_path, payload.content)?;
    // }
    ResponseWrapper::<()>::created()
}

#[get("api/errors")]
pub async fn get_errors(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let errors = operations.errors()?;
    let total_count = errors.len();
    let ret = errors
        .iter()
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .collect_vec();
    ResponseWrapper::json(Pageable::new(total_count as u32, params.page(), params.limit(), ret))
}

#[get("/api/report")]
pub async fn get_report(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<ReportRequest>) -> ApiResult<ReportResponse> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let accounts = operations.all_accounts()?;

    let mut latest_account_balances = vec![];
    for account in accounts {
        let vec = operations.account_target_date_balance(&account, params.to)?;
        latest_account_balances.extend(vec);
    }

    let balance = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&item.balance_number));

    let liability = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&item.balance_number));

    let store = operations.read();

    let mut account_type_postings_map = HashMap::new();
    for (key, data) in &store
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&params.from))
        .filter(|posting| posting.trx_datetime.le(&params.to))
        .cloned()
        .group_by(|it| it.account.account_type)
    {
        // todo(high) calculate all postings amount
        account_type_postings_map.insert(key, data.collect_vec());
    }

    let income = account_type_postings_map
        .remove(&AccountType::Income)
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|it| AmountResponse {
            number: it.inferred_amount.number.clone(),
            commodity: it.inferred_amount.currency.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });
    let expense = account_type_postings_map
        .remove(&AccountType::Expenses)
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|it| AmountResponse {
            number: it.inferred_amount.number.clone(),
            commodity: it.inferred_amount.currency.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    let transaction_total = store
        .transactions
        .values()
        .filter(|trx| trx.flag != Flag::BalancePad && trx.flag != Flag::BalanceCheck)
        .filter(|trx| trx.datetime.ge(&params.from))
        .filter(|trx| trx.datetime.le(&params.to))
        .count();
    drop(store);

    let income_transactions = operations.account_dated_journals(AccountType::Income, params.from, params.to)?;

    let total_income = income_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &income_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&item.inferred_unit_number);
    }
    let income_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: account_total.div(&total_income),
        })
        .collect_vec();

    let income_top_transactions = income_transactions
        .into_iter()
        .sorted_by(|a, b| a.inferred_unit_number.cmp(&b.inferred_unit_number))
        .take(10)
        .collect_vec();

    // --------

    let expense_transactions = operations.account_dated_journals(AccountType::Expenses, params.from, params.to)?;

    let total_expense = expense_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &expense_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&item.inferred_unit_number);
    }
    let expense_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .rev()
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: account_total.div(&total_expense),
        })
        .collect_vec();

    let expense_top_transactions = expense_transactions
        .into_iter()
        .sorted_by(|a, b| a.inferred_unit_number.cmp(&b.inferred_unit_number))
        .rev()
        .take(10)
        .collect_vec();

    ResponseWrapper::json(ReportResponse {
        from: params.from.naive_local(),
        to: params.to.naive_local(),
        balance: AmountResponse {
            number: balance,
            commodity: ledger.options.operating_currency.to_owned(),
        },
        liability: AmountResponse {
            number: liability,
            commodity: ledger.options.operating_currency.to_owned(),
        },
        income,
        expense,
        transaction_number: transaction_total as i64,
        income_rank,
        income_top_transactions,
        expense_rank,
        expense_top_transactions,
    })
}

#[get("/api/options")]
pub async fn get_all_options(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options)
}

#[cfg(feature = "frontend")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../frontend/build"]
struct Asset;

#[cfg(feature = "frontend")]
pub struct StaticFile<T>(pub T);

#[cfg(feature = "frontend")]
use actix_web::{HttpRequest, HttpResponse};
use zhang_core::constants::KEY_OPERATING_CURRENCY;
use zhang_core::domains::schemas::{AccountJournalDomain, CommodityDomain, ErrorDomain, MetaType, OptionDomain};
use zhang_core::domains::Operations;
use zhang_core::exporter::AppendableExporter;
use zhang_core::{ZhangError, ZhangResult};

use crate::util::AmountLike;

#[cfg(feature = "frontend")]
impl<T> Responder for StaticFile<T>
where
    T: Into<String>,
{
    type Body = actix_web::body::BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let path: String = self.0.into();
        match Asset::get(path.as_str()) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                HttpResponse::Ok()
                    .content_type(mime)
                    .body(actix_web::body::BoxBody::new(content.data.into_owned()))
            }
            None => HttpResponse::NotFound().finish(),
        }
    }
}

pub(crate) fn insert_line(file: PathBuf, content: &str, at: usize) -> ServerResult<()> {
    let mut file_content = std::fs::read_to_string(&file).with_path(&file)?;
    file_content.insert(at, '\n');
    file_content.insert_str(at + 1, content);
    Ok(std::fs::write(&file, file_content).with_path(&file)?)
}
