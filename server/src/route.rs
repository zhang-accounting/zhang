use std::collections::HashMap;
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
use actix_web::{get, post, put, web, Responder};
use bigdecimal::{BigDecimal, Zero};
use chrono::{Local, NaiveDate, NaiveDateTime};
use futures_util::StreamExt;
use glob::glob;
use indexmap::IndexSet;
use itertools::Itertools;
use log::{error, info};
use now::TimeZoneNow;
use sqlx::FromRow;
use tokio::sync::RwLock;
use uuid::Uuid;

use zhang_core::database::type_ext::big_decimal::ZhangBigDecimal;
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::ledger::Ledger;
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
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Balance, BalanceCheck, BalancePad, Date, Directive, Document, Flag, Meta, Posting, Transaction, ZhangString};
use zhang_core::utils::date_range::NaiveDateRange;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

#[derive(FromRow)]
struct ValueRow {
    value: String,
}
#[derive(FromRow)]
pub struct DetailRow {
    date: NaiveDate,
    account: String,
    balance_number: ZhangBigDecimal,
    balance_commodity: String,
}

#[get("/api/sse")]
pub async fn sse(broadcaster: Data<Broadcaster>) -> impl Responder {
    broadcaster.new_client().await
}

#[get("/api/info")]
pub async fn get_basic_info(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<BasicInfo> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations().await;

    ResponseWrapper::json(BasicInfo {
        title: operations.option("title").await?.map(|it| it.value),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
    })
}

// todo rename api
#[get("/api/for-new-transaction")]
pub async fn get_info_for_new_transactions(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<InfoForNewTransaction> {
    let guard = ledger.read().await;
    let mut connection = guard.connection().await;

    #[derive(FromRow)]
    struct AccountNameRow {
        name: String,
    }
    let account_names = sqlx::query_as::<_, AccountNameRow>(
        r#"
        SELECT name FROM accounts WHERE status = 'Open'
        "#,
    )
    .fetch_all(&mut connection)
    .await?;

    #[derive(FromRow)]
    struct PayeeRow {
        payee: String,
    }
    let payees = sqlx::query_as::<_, PayeeRow>(
        r#"
        select distinct payee from transactions
        "#,
    )
    .fetch_all(&mut connection)
    .await?;

    ResponseWrapper::json(InfoForNewTransaction {
        payee: payees.into_iter().map(|it| it.payee).filter(|it| !it.is_empty()).collect_vec(),
        account_name: account_names.into_iter().map(|it| it.name).collect_vec(),
    })
}

#[get("/api/statistic")]
pub async fn get_statistic_data(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<StatisticRequest>) -> ApiResult<StatisticResponse> {
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;
    let params = params.into_inner();
    #[derive(FromRow)]
    pub struct StaticRow {
        date: NaiveDate,
        account_type: String,
        amount: ZhangBigDecimal,
        commodity: String,
    }
    let rows = sqlx::query_as::<_, StaticRow>(
        r#"
        SELECT
            date(datetime) AS date,
            accounts.type AS account_type,
            sum(inferred_unit_number) AS amount,
            inferred_unit_commodity AS commodity
        FROM
            transaction_postings
            JOIN transactions ON transactions.id = transaction_postings.trx_id
            JOIN accounts ON accounts.name = transaction_postings.account
            where transactions.datetime >= $1 and transactions.datetime <= $2
        GROUP BY
            date(datetime),
            accounts.type,
            inferred_unit_commodity
    "#,
    )
    .bind(params.from.naive_local())
    .bind(params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;
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
    for day in NaiveDateRange::new(params.from.date_naive(), params.to.date_naive()) {
        ret.entry(day).or_insert_with(HashMap::new);
    }

    let accounts = sqlx::query_as::<_, ValueRow>("select name as value from accounts")
        .fetch_all(&mut connection)
        .await?
        .into_iter()
        .map(|it| it.value)
        .collect_vec();

    let existing_account_balance = sqlx::query_as::<_, DetailRow>(
        r#"
        SELECT
            date(datetime) AS date,
            account,
            balance_number,
            balance_commodity
        FROM
            account_daily_balance
        WHERE
            datetime < $1
        GROUP BY
            account
        HAVING
            max(datetime)
    "#,
    )
    .bind(params.from.naive_local())
    .fetch_all(&mut connection)
    .await?;

    let mut existing_balances: HashMap<String, AmountResponse> = existing_account_balance
        .into_iter()
        .map(|line| {
            (
                line.account.to_owned(),
                AmountResponse {
                    number: line.balance_number,
                    commodity: line.balance_commodity,
                },
            )
        })
        .collect();

    let details = sqlx::query_as::<_, DetailRow>(
        r#"
        SELECT
        date(datetime) AS date,
        account,
        balance_number,
        balance_commodity
    FROM
        account_daily_balance
    where datetime >= $1 and datetime <= $2
    "#,
    )
    .bind(params.from.naive_local())
    .bind(params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;

    let mut detail_map: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();
    for (date, dated_rows) in &details.into_iter().group_by(|row| row.date) {
        let date_entry = detail_map.entry(date).or_insert_with(HashMap::new);
        for row in dated_rows {
            date_entry.insert(
                row.account,
                AmountResponse {
                    number: row.balance_number,
                    commodity: row.balance_commodity,
                },
            );
        }
    }

    let mut detail_ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();

    for target_day in NaiveDateRange::new(params.from.date_naive(), params.to.date_naive()) {
        let mut target_day_ret = HashMap::new();

        let mut target_day_map = detail_map.remove(&target_day).unwrap_or_default();
        for target_account in &accounts {
            let option = target_day_map.remove(target_account);
            if let Some(target_account_balance) = option {
                // has change in date
                target_day_ret.insert(target_account.to_owned(), target_account_balance.clone());
                existing_balances.insert(target_account.to_owned(), target_account_balance);
            } else {
                // need to get previous day's balance
                let balance = existing_balances.get(target_account).cloned().unwrap_or_else(|| AmountResponse {
                    number: ZhangBigDecimal(BigDecimal::zero()),
                    commodity: ledger.options.operating_currency.to_owned(),
                });
                target_day_ret.insert(target_account.to_owned(), balance);
            }
        }
        detail_ret.insert(target_day, target_day_ret);
    }

    ResponseWrapper::json(StatisticResponse {
        changes: ret,
        details: detail_ret,
    })
}

#[get("/api/statistic/current")]
pub async fn current_statistic(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<CurrentStatisticResponse> {
    let ledger = ledger.read().await;

    let mut connection = ledger.connection().await;

    let month_beginning = Local.beginning_of_month().naive_local();
    let month_end = Local.end_of_month().naive_local();

    let mut operations = ledger.operations().await;

    let latest_account_balances = operations.accounts_latest_balance().await?;

    let balances = group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )
    .await?;

    let liability = group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )
    .await?;

    #[derive(FromRow)]
    struct CurrentMonthBalance {
        account_type: String,
        amount: ZhangBigDecimal,
        commodity: String,
    }

    let current_month_balance = sqlx::query_as::<_, CurrentMonthBalance>(
        r#"
    SELECT accounts.type             AS account_type,
           sum(inferred_unit_number) AS amount,
           inferred_unit_commodity   AS commodity
    FROM transaction_postings
             JOIN transactions ON transactions.id = transaction_postings.trx_id
             JOIN accounts ON accounts.name = transaction_postings.account
    WHERE transactions.datetime >= $1 and transactions.datetime <= $2
    GROUP BY
        accounts.type,
        inferred_unit_commodity
    "#,
    )
    .bind(month_beginning)
    .bind(month_end)
    .fetch_all(&mut connection)
    .await?;

    let income = current_month_balance
        .iter()
        .find(|it| it.account_type.eq("Income"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: ZhangBigDecimal(BigDecimal::zero()),
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
            number: ZhangBigDecimal(BigDecimal::zero()),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    ResponseWrapper::json(CurrentStatisticResponse {
        balance: balances,
        liability,
        income,
        expense,
    })
}

async fn group_and_calculate<T: AmountLike>(operations: &mut Operations, latest_account_balances: Vec<T>) -> ZhangResult<CalculatedAmount> {
    let operating_currency = operations
        .option(KEY_OPERATING_CURRENCY)
        .await?
        .ok_or(ZhangError::OptionNotFound(KEY_OPERATING_CURRENCY.to_owned()))?
        .value;

    let mut total_sum = BigDecimal::zero();

    let mut detail = HashMap::new();
    for (commodity, values) in &latest_account_balances.into_iter().group_by(|it| it.commodity().to_owned()) {
        let commodity_sum = values.fold(BigDecimal::zero(), |acc, item| acc.add(item.number()));

        if commodity.eq(&operating_currency) {
            total_sum.add_assign(&commodity_sum);
        } else {
            let target_price = operations.get_price(Local::now().naive_local(), &commodity, &operating_currency).await?;
            if let Some(price) = target_price {
                total_sum.add_assign((&commodity_sum).mul(price.amount.0));
            }
        }
        detail.insert(commodity, ZhangBigDecimal(commodity_sum));
    }
    Ok(CalculatedAmount {
        calculated: AmountResponse {
            number: ZhangBigDecimal(total_sum),
            commodity: operating_currency.to_owned(),
        },
        detail,
    })
}

#[get("/api/journals")]
pub async fn get_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemResponse>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations().await;
    let mut connection = ledger.connection().await;
    let params = params.into_inner();

    let total_count = operations.transaction_counts().await?;

    #[derive(Debug, FromRow)]
    struct JournalHeader {
        id: String,
        sequence: i64,
        datetime: NaiveDateTime,
        journal_type: String,
        payee: String,
        narration: Option<String>,
    }
    let journal_headers = sqlx::query_as::<_, JournalHeader>(
        r#"
        SELECT id, sequence, datetime, type as journal_type, payee, narration FROM transactions ORDER BY "sequence" DESC LIMIT $1 OFFSET $2
        "#,
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&mut connection)
    .await?;

    #[derive(Debug, FromRow)]
    struct JournalArm {
        trx_id: String,
        account: String,
        unit_number: Option<ZhangBigDecimal>,
        unit_commodity: Option<String>,
        cost_number: Option<ZhangBigDecimal>,
        cost_commodity: Option<String>,
        price_number: Option<ZhangBigDecimal>,
        price_commodity: Option<String>,
        inferred_unit_number: ZhangBigDecimal,
        inferred_unit_commodity: String,
        account_before_number: ZhangBigDecimal,
        account_before_commodity: String,
        account_after_number: ZhangBigDecimal,
        account_after_commodity: String,
    }
    let journal_arms = sqlx::query_as::<_, JournalArm>(
        r#"
        select * from transaction_postings where trx_id in ( SELECT id FROM transactions ORDER BY "sequence" DESC LIMIT $1 OFFSET $2 )
        "#,
    )
    .bind(params.limit())
    .bind(params.offset())
    .fetch_all(&mut connection)
    .await?;

    let mut header_map: HashMap<String, JournalHeader> = journal_headers.into_iter().map(|it| (it.id.to_owned(), it)).collect();
    let mut ret = vec![];
    for (trx_id, arms) in &journal_arms.into_iter().group_by(|it| it.trx_id.to_owned()) {
        let header = header_map.remove(&trx_id);
        if let Some(header) = header {
            let item = match header.journal_type.as_str() {
                "BalancePad" => {
                    let postings = arms
                        .map(|arm| JournalTransactionPostingResponse {
                            account: arm.account,
                            unit_number: arm.unit_number,
                            unit_commodity: arm.unit_commodity,
                            cost_number: arm.cost_number,
                            cost_commodity: arm.cost_commodity,
                            price_number: arm.price_number,
                            price_commodity: arm.price_commodity,
                            inferred_unit_number: arm.inferred_unit_number,
                            inferred_unit_commodity: arm.inferred_unit_commodity,
                            account_before_number: arm.account_before_number,
                            account_before_commodity: arm.account_before_commodity,
                            account_after_number: arm.account_after_number,
                            account_after_commodity: arm.account_after_commodity,
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
                            account: arm.account,
                            unit_number: arm.unit_number,
                            unit_commodity: arm.unit_commodity,
                            cost_number: arm.cost_number,
                            cost_commodity: arm.cost_commodity,
                            price_number: arm.price_number,
                            price_commodity: arm.price_commodity,
                            inferred_unit_number: arm.inferred_unit_number,
                            inferred_unit_commodity: arm.inferred_unit_commodity,
                            account_before_number: arm.account_before_number,
                            account_before_commodity: arm.account_before_commodity,
                            account_after_number: arm.account_after_number,
                            account_after_commodity: arm.account_after_commodity,
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
                            account: arm.account,
                            unit_number: arm.unit_number,
                            unit_commodity: arm.unit_commodity,
                            cost_number: arm.cost_number,
                            cost_commodity: arm.cost_commodity,
                            price_number: arm.price_number,
                            price_commodity: arm.price_commodity,
                            inferred_unit_number: arm.inferred_unit_number,
                            inferred_unit_commodity: arm.inferred_unit_commodity,
                            account_before_number: arm.account_before_number,
                            account_before_commodity: arm.account_before_commodity,
                            account_after_number: arm.account_after_number,
                            account_after_commodity: arm.account_after_commodity,
                        })
                        .collect_vec();
                    let tags = operations.trx_tags(&trx_id).await?;
                    let links = operations.trx_links(&trx_id).await?;
                    let metas = operations
                        .metas(MetaType::TransactionMeta, &trx_id)
                        .await
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
pub async fn upload_transaction_document(ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: web::Path<(String,)>) -> ApiResult<String> {
    let transaction_id = path.into_inner().0;
    let ledger_stage = ledger.read().await;
    let mut operations = ledger_stage.operations().await;
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
    let span_info = operations.transaction_span(&transaction_id).await?;
    let metas_content = documents
        .into_iter()
        .map(|document| format!("  document: {}", document.to_plain_string()))
        .join("\n");
    insert_line(PathBuf::from(span_info.source_file), &metas_content, span_info.span_end as usize)?;
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
    let mut operations = ledger.operations().await;

    let balances = operations.account_balances().await?;
    let mut ret = vec![];
    for (key, group) in &balances.into_iter().group_by(|it| it.account.clone()) {
        let account_balances = group.collect_vec();
        let account_domain = operations.account(&key).await?.ok_or(ZhangError::InvalidAccount)?;

        let amount = group_and_calculate(&mut operations, account_balances).await?;
        ret.push(AccountResponse {
            name: account_domain.name,
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
    let mut operations = ledger.operations().await;
    let account_domain = operations.account(&account_name).await?;

    let account_info = match account_domain {
        Some(info) => info,
        None => return ResponseWrapper::not_found(),
    };
    let vec = operations.single_account_balances(&account_info.name).await?;
    let amount = group_and_calculate(&mut operations, vec).await?;

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
    let mut connection = ledger.connection().await;

    let rows = sqlx::query_as::<_, DocumentResponse>(
        r#"
    select documents.*
    from documents
             left join transaction_postings tp on documents.trx_id = tp.trx_id
    group by path, tp.trx_id
    "#,
    )
    .fetch_all(&mut connection)
    .await?;
    ResponseWrapper::json(rows)
}

#[post("/api/accounts/{account_name}/documents")]
pub async fn upload_account_document(
    ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: web::Path<(String,)>, exporter: Data<dyn AppendableExporter>,
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
    let mut connection = ledger.connection().await;

    let rows = sqlx::query_as::<_, DocumentResponse>(
        r#"
    select documents.*
    from documents
             left join transaction_postings tp on documents.trx_id = tp.trx_id
    where documents.account = $1
       or tp.account = $1
    "#,
    )
    .bind(account_name)
    .fetch_all(&mut connection)
    .await?;

    ResponseWrapper::json(rows)
}

#[get("/api/accounts/{account_name}/journals")]
pub async fn get_account_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<Vec<AccountJournalDomain>> {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut operations = ledger.operations().await;

    let journals = operations.account_journals(&account_name).await?;

    ResponseWrapper::json(journals)
}

#[post("/api/accounts/{account_name}/balances")]
pub async fn create_account_balance(
    ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>, Json(payload): Json<AccountBalanceRequest>, exporter: Data<dyn AppendableExporter>,
) -> ApiResult<()> {
    let target_account = params.into_inner().0;
    let ledger = ledger.read().await;
    let _connection = ledger.connection().await;

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
    let mut connection = ledger.connection().await;

    let vec = sqlx::query_as::<_, CommodityListItemResponse>(
        r#"
            select commodities.*,
                   IFNULL(commodity_total_amount.total_amount, 0.00) as total_amount,
                   latest_price.datetime         latest_price_date,
                   latest_price.amount           latest_price_amount,
                   latest_price.target_commodity latest_price_commodity
            from commodities
                     left join (select commodity, datetime, amount, target_commodity
                                from prices
                                group by commodity
                                having min(datetime)) latest_price on commodities.name = latest_price.commodity
                     left join (select commodity, total(amount) as total_amount
                                from commodity_lots
                                         join accounts on commodity_lots.account = accounts.name
                                where accounts.type in ('Assets', 'Liabilities')
                                group by commodity) commodity_total_amount on commodities.name = commodity_total_amount.commodity
    "#,
    )
    .fetch_all(&mut connection)
    .await?;
    ResponseWrapper::json(vec)
}

#[get("/api/commodities/{commodity_name}")]
pub async fn get_single_commodity(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<CommodityDetailResponse> {
    let commodity_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    let basic_info = sqlx::query_as::<_, CommodityListItemResponse>(
        r#"
            select commodities.*,
                   IFNULL(commodity_total_amount.total_amount, 0.00) as total_amount,
                   latest_price.datetime         latest_price_date,
                   latest_price.amount           latest_price_amount,
                   latest_price.target_commodity latest_price_commodity
            from commodities
                     left join (select commodity, datetime, amount, target_commodity
                                from prices
                                group by commodity
                                having min(datetime)) latest_price on commodities.name = latest_price.commodity
                     left join (select commodity, total(amount) as total_amount
                                from commodity_lots
                                         join accounts on commodity_lots.account = accounts.name
                                where accounts.type in ('Assets', 'Liabilities')
                                group by commodity) commodity_total_amount on commodities.name = commodity_total_amount.commodity
            where commodities.name = $1
    "#,
    )
    .bind(&commodity_name)
    .fetch_one(&mut connection)
    .await?;

    let lots = sqlx::query_as::<_, CommodityLot>(
        r#"
            select datetime, amount, price_amount, price_commodity, account
            from commodity_lots
            where commodity = $1
    "#,
    )
    .bind(&commodity_name)
    .fetch_all(&mut connection)
    .await?;

    let prices = sqlx::query_as::<_, CommodityPrice>(
        r#"
            select datetime, amount, target_commodity
            from prices
            where commodity = $1
    "#,
    )
    .bind(&commodity_name)
    .fetch_all(&mut connection)
    .await?;

    ResponseWrapper::json(CommodityDetailResponse {
        info: basic_info,
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
pub async fn get_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>) -> ApiResult<FileDetailResponse> {
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
pub async fn update_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>, Json(payload): Json<FileUpdateRequest>) -> ApiResult<()> {
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
    let mut operations = ledger.operations().await;
    let errors = operations.errors().await?;
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
    let mut connection = ledger.connection().await;
    let mut operations = ledger.operations().await;

    let latest_account_balances = sqlx::query_as::<_, DetailRow>(
        r#"
        SELECT
            date(datetime) AS date,
            account,
            balance_number,
            balance_commodity
        FROM
            account_daily_balance
        WHERE datetime <= $1
        GROUP BY
            account
        HAVING
            max(datetime)
    "#,
    )
    .bind(params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;
    let balance = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&*item.balance_number));

    let liability = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&*item.balance_number));

    #[derive(FromRow)]
    struct DurationBalance {
        account_type: String,
        amount: ZhangBigDecimal,
        commodity: String,
    }

    let duration_balances = sqlx::query_as::<_, DurationBalance>(
        r#"
    SELECT accounts.type             AS account_type,
           sum(inferred_unit_number) AS amount,
           inferred_unit_commodity   AS commodity
    FROM transaction_postings
             JOIN transactions ON transactions.id = transaction_postings.trx_id
             JOIN accounts ON accounts.name = transaction_postings.account
    WHERE transactions.datetime >= $1 and transactions.datetime <= $2
    GROUP BY
        accounts.type,
        inferred_unit_commodity
    "#,
    )
    .bind(params.from.naive_local())
    .bind(params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;

    let income = duration_balances
        .iter()
        .find(|it| it.account_type.eq("Income"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: ZhangBigDecimal(BigDecimal::zero()),
            commodity: ledger.options.operating_currency.to_owned(),
        });
    let expense = duration_balances
        .iter()
        .find(|it| it.account_type.eq("Expenses"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: ZhangBigDecimal(BigDecimal::zero()),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    let transaction_total = sqlx::query_as::<_, (i64,)>(
        r#"
        select count(1) as total
        from transactions
        where transactions."type" != 'BalancePad'
          and transactions."type" != 'BalanceCheck'
          and datetime >= ?
          and datetime <= ?
    "#,
    )
    .bind(params.from.naive_local())
    .bind(params.to.naive_local())
    .fetch_one(&mut connection)
    .await?
    .0;

    let income_transactions = operations
        .account_dated_journals("Income", params.from.naive_local(), params.to.naive_local())
        .await?;

    let total_income = income_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&*item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &income_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&*item.inferred_unit_number);
    }
    let income_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: ZhangBigDecimal(account_total.div(&total_income)),
        })
        .collect_vec();

    let income_top_transactions = income_transactions
        .into_iter()
        .sorted_by(|a, b| a.inferred_unit_number.cmp(&b.inferred_unit_number))
        .take(10)
        .collect_vec();

    // --------

    let expense_transactions = operations
        .account_dated_journals("Expenses", params.from.naive_local(), params.to.naive_local())
        .await?;

    let total_expense = expense_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&*item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &expense_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&*item.inferred_unit_number);
    }
    let expense_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .rev()
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: ZhangBigDecimal(account_total.div(&total_expense)),
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
            number: ZhangBigDecimal(balance),
            commodity: ledger.options.operating_currency.to_owned(),
        },
        liability: AmountResponse {
            number: ZhangBigDecimal(liability),
            commodity: ledger.options.operating_currency.to_owned(),
        },
        income,
        expense,
        transaction_number: transaction_total,
        income_rank,
        income_top_transactions,
        expense_rank,
        expense_top_transactions,
    })
}

#[get("/api/options")]
pub async fn get_all_options(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations().await;
    let options = operations.options().await?;
    ResponseWrapper::json(options)
}

#[cfg(feature = "frontend")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../frontend/build"]
struct Asset;

#[cfg(feature = "frontend")]
pub struct StaticFile<T>(pub T);

use crate::util::AmountLike;
#[cfg(feature = "frontend")]
use actix_web::{HttpRequest, HttpResponse};
use zhang_core::constants::KEY_OPERATING_CURRENCY;
use zhang_core::domains::schemas::{AccountJournalDomain, ErrorDomain, MetaType, OptionDomain};
use zhang_core::domains::Operations;
use zhang_core::exporter::AppendableExporter;
use zhang_core::{ZhangError, ZhangResult};

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
