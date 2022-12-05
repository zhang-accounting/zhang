use crate::core::ledger::Ledger;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::iter::FromIterator;
use std::ops::Add;

use crate::core::account::Account;
use crate::core::data::{Balance, BalanceCheck, BalancePad, Date, Document, Meta, Posting, Transaction};
use crate::core::models::{Directive, Flag, ZhangString};
use crate::server::model::mutation::create_folder_if_not_exist;
use crate::server::response::{
    AccountJournalItem, AccountResponse, AmountResponse, CommodityDetailResponse, CommodityListItemResponse,
    CommodityLot, CommodityPrice, CurrentStatisticResponse, DocumentResponse, FileDetailResponse,
    InfoForNewTransaction, JournalBalancePadItemResponse, JournalItemResponse, JournalTransactionItemResponse,
    JournalTransactionPostingResponse, MetaResponse, ResponseWrapper, StatisticResponse,
};
use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::body::BoxBody;
use actix_web::http::Uri;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{get, post, put, web, HttpRequest, HttpResponse, Responder};

use crate::core::amount::Amount;
use crate::core::database::type_ext::big_decimal::ZhangBigDecimal;
use crate::core::utils::date_range::NaiveDateRange;
use crate::core::utils::string_::StringExt;
use crate::error::ZhangResult;
use crate::parse_zhang;
use crate::server::request::{
    AccountBalanceRequest, CreateTransactionRequest, FileUpdateRequest, JournalRequest, StatisticRequest,
};
use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use futures_util::StreamExt;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use rust_embed::RustEmbed;

use now::TimeZoneNow;
use sqlx::{FromRow, SqliteConnection};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// pub async fn graphql_playground() -> impl IntoResponse {
//     Html(playground_source(GraphQLPlaygroundConfig::new("/graphql")))
// }
//
// pub async fn graphql_handler(schema: Extension<LedgerSchema>, req: GraphQLRequest) -> GraphQLResponse {
//     schema.execute(req.0).await.into()
// }

pub type ApiResult<T> = ZhangResult<ResponseWrapper<T>>;

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

pub async fn get_metas(
    type_: &str, type_identifier: &str, conn: &mut SqliteConnection,
) -> ZhangResult<Vec<MetaResponse>> {
    let rows = sqlx::query_as::<_, MetaResponse>(
        r#"
        select key, value from metas where type = $1 and type_identifier = $2
        "#,
    )
    .bind(type_)
    .bind(type_identifier)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}
pub async fn get_transaction_tags(trx_id: &str, conn: &mut SqliteConnection) -> ZhangResult<Vec<String>> {
    let rows = sqlx::query_as::<_, ValueRow>(
        r#"
        select tag as value from transaction_tags where trx_id = $1
        "#,
    )
    .bind(trx_id)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|it| it.value).collect_vec())
}
pub async fn get_transaction_links(trx_id: &str, conn: &mut SqliteConnection) -> ZhangResult<Vec<String>> {
    #[derive(FromRow)]
    struct ValueRow {
        value: String,
    }
    let rows = sqlx::query_as::<_, ValueRow>(
        r#"
        select link as value from transaction_links where trx_id = $1
        "#,
    )
    .bind(trx_id)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|it| it.value).collect_vec())
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
        payee: payees.into_iter().map(|it| it.payee).collect_vec(),
        account_name: account_names.into_iter().map(|it| it.name).collect_vec(),
    })
}

#[get("/api/statistic")]
pub async fn get_statistic_data(
    ledger: Data<Arc<RwLock<Ledger>>>, params: Query<StatisticRequest>,
) -> ApiResult<StatisticResponse> {
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
    .bind(&params.from.naive_local())
    .bind(&params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;
    let mut ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();
    for (date, dated_rows) in &rows.into_iter().group_by(|row| row.date) {
        let date_entry = ret.entry(date).or_insert_with(|| HashMap::new());
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
    for day in NaiveDateRange::new(params.from.date().naive_local(), params.to.date().naive_local()) {
        ret.entry(day).or_insert_with(|| HashMap::new());
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
    .bind(&params.from.naive_local())
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
    .bind(&params.from.naive_local())
    .bind(&params.to.naive_local())
    .fetch_all(&mut connection)
    .await?;

    let mut detail_map: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();
    for (date, dated_rows) in &details.into_iter().group_by(|row| row.date) {
        let date_entry = detail_map.entry(date).or_insert_with(|| HashMap::new());
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

    for target_day in NaiveDateRange::new(params.from.date().naive_local(), params.to.date().naive_local()) {
        let mut target_day_ret = HashMap::new();

        let mut target_day_map = detail_map.remove(&target_day).unwrap_or_else(|| HashMap::new());
        for target_account in &accounts {
            let option = target_day_map.remove(target_account);
            if let Some(target_account_balance) = option {
                // has change in date
                target_day_ret.insert(target_account.to_owned(), target_account_balance.clone());
                existing_balances.insert(target_account.to_owned(), target_account_balance);
            } else {
                // need to get previous day's balance
                let balance = existing_balances
                    .get(target_account)
                    .map(|it| it.clone())
                    .unwrap_or_else(|| AmountResponse {
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
    let latest_account_balances = sqlx::query_as::<_, DetailRow>(
        r#"
        SELECT
            date(datetime) AS date,
            account,
            balance_number,
            balance_commodity
        FROM
            account_daily_balance
        GROUP BY
            account
        HAVING
            max(datetime)
    "#,
    )
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
    })
}

#[get("/api/journals")]
pub async fn get_journals(
    ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>,
) -> ApiResult<Vec<JournalItemResponse>> {
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;
    let params = params.into_inner();
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

    let mut header_map: HashMap<String, JournalHeader> =
        journal_headers.into_iter().map(|it| (it.id.to_owned(), it)).collect();
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
                    todo!()
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
                    let tags = get_transaction_tags(&trx_id, &mut connection).await?;
                    let links = get_transaction_links(&trx_id, &mut connection).await?;
                    let metas = get_metas("TransactionMeta", &trx_id, &mut connection).await.unwrap();
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
    ResponseWrapper::json(ret)
}

#[post("/api/transactions")]
pub async fn create_new_transaction(
    ledger: Data<Arc<RwLock<Ledger>>>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<String> {
    let ledger = ledger.read().await;

    let postings = payload
        .postings
        .into_iter()
        .map(|posting| Posting {
            flag: None,
            account: Account::from_str(&posting.account).unwrap(),
            units: posting.unit.map(|unit| Amount::new(unit.number, unit.commodity)),
            cost: None,
            cost_date: None,
            price: None,
            meta: Default::default(),
        })
        .collect_vec();
    let mut metas = Meta::default();
    for meta in payload.metas {
        metas.insert(meta.key, meta.value.to_quote());
    }
    let time = payload.datetime.naive_local();
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
    ledger.append_directives(vec![trx], format!("data/{}/{}.zhang", time.year(), time.month()));
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

pub async fn serve_frontend(uri: Uri) -> impl Responder {
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
    let mut connection = ledger.connection().await;

    #[derive(FromRow)]
    struct AccountBalanceRow {
        name: String,
        status: String,
        balance_number: ZhangBigDecimal,
        balance_commodity: String,
    }

    let rows = sqlx::query_as::<_, AccountBalanceRow>(
        r#"
    select name, status, balance_number, balance_commodity
    from account_balance
             join accounts on accounts.name = account_balance.account
    "#,
    )
    .fetch_all(&mut connection)
    .await?;
    let mut ret = vec![];
    for (key, group) in &rows.into_iter().group_by(|it| it.name.clone()) {
        let mut status = "".to_string();
        let mut commodities = HashMap::new();
        for row in group {
            status = row.status;
            commodities.insert(row.balance_commodity, row.balance_number);
        }
        ret.push(AccountResponse {
            name: key,
            status,
            commodities,
        });
    }
    ResponseWrapper::json(ret)
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
    "#,
    )
    .fetch_all(&mut connection)
    .await?;
    ResponseWrapper::json(rows)
}

#[post("/api/accounts/{account_name}/documents")]
pub async fn upload_account_document(
    ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: web::Path<(String,)>,
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

        while let Some(chunk) = field.next().await {
            let data = chunk.unwrap();

            let v4 = Uuid::new_v4();
            let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);

            info!(
                "uploading document `{}`({} bytes, id={}) to account {}",
                file_name,
                data.len(),
                &v4.to_string(),
                &account_name
            );
            create_folder_if_not_exist(&buf);

            let f = File::create(&buf).expect("Unable to create file");
            let mut f = BufWriter::new(f);
            f.write_all(&data).expect("cannot wirte content");
            let path = match buf.strip_prefix(&entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => buf.to_str().unwrap(),
            };

            documents.push(Directive::Document(Document {
                date: Date::Datetime(Local::now().naive_local()),
                account: Account::from_str(&account_name)?,
                filename: ZhangString::QuoteString(path.to_string()),
                tags: None,
                links: None,
                meta: Default::default(),
            }));
        }
    }
    let time = Local::now().naive_local();
    ledger_stage.append_directives(documents, format!("data/{}/{}.zhang", time.year(), time.month()));
    ResponseWrapper::<()>::created()
}

#[get("/api/accounts/{account_name}/documents")]
pub async fn get_account_documents(
    ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>,
) -> ApiResult<Vec<DocumentResponse>> {
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
pub async fn get_account_journals(
    ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>,
) -> ApiResult<Vec<AccountJournalItem>> {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    let rows = sqlx::query_as::<_, AccountJournalItem>(
        r#"
            select datetime,
                   trx_id,
                   payee,
                   narration,
                   inferred_unit_number,
                   inferred_unit_commodity,
                   account_after_number,
                   account_after_commodity
            from transaction_postings
                     join transactions on transactions.id = transaction_postings.trx_id
            where account = $1
            order by datetime desc, transactions.sequence desc
    "#,
    )
    .bind(account_name)
    .fetch_all(&mut connection)
    .await?;

    ResponseWrapper::json(rows)
}

#[post("/api/accounts/{account_name}/balances")]
pub async fn create_account_balance(
    ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> ApiResult<()> {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let _connection = ledger.connection().await;

    let balance = match payload {
        AccountBalanceRequest::Check { amount, commodity } => Balance::BalanceCheck(BalanceCheck {
            date: Date::Datetime(Local::now().naive_local()),
            account: Account::from_str(&account_name)?,
            amount: Amount::new(amount, commodity),
            tolerance: None,
            distance: None,
            current_amount: None,
            meta: Default::default(),
        }),
        AccountBalanceRequest::Pad {
            amount,
            commodity,
            pad_account,
        } => Balance::BalancePad(BalancePad {
            date: Date::Datetime(Local::now().naive_local()),
            account: Account::from_str(&account_name)?,
            amount: Amount::new(amount, commodity),
            tolerance: None,
            diff_amount: None,
            meta: Default::default(),
            pad: Account::from_str(&pad_account)?,
        }),
    };
    let time = Local::now().naive_local();
    ledger.append_directives(
        vec![Directive::Balance(balance)],
        format!("data/{}/{}.zhang", time.year(), time.month()),
    );
    ResponseWrapper::<()>::created()
}

#[get("/api/commodities")]
pub async fn get_all_commodities(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<CommodityListItemResponse>> {
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    let vec = sqlx::query_as::<_, CommodityListItemResponse>(
        r#"
            select commodities.*,
                   commodity_total_amount.total_amount,
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
pub async fn get_single_commodity(
    ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>,
) -> ApiResult<CommodityDetailResponse> {
    let commodity_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    let basic_info = sqlx::query_as::<_, CommodityListItemResponse>(
        r#"
            select commodities.*,
                   commodity_total_amount.total_amount,
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
    "#, )
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
    let vec = ledger
        .visited_files
        .iter()
        .map(|path| path.strip_prefix(entry_path).unwrap().to_str().map(|it| it.to_string()))
        .collect_vec();
    ResponseWrapper::json(vec)
}

#[get("/api/files/{file_path}")]
pub async fn get_file_content(
    ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>,
) -> ApiResult<FileDetailResponse> {
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
pub async fn update_file_content(
    ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>, Json(payload): Json<FileUpdateRequest>,
) -> ApiResult<()> {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    if parse_zhang(&payload.content, None).is_ok() {
        std::fs::write(full_path, payload.content)?;
    }
    ResponseWrapper::<()>::created()
}

#[derive(RustEmbed)]
#[folder = "frontend/build"]
struct Asset;

pub struct StaticFile<T>(pub T);
impl<T> Responder for StaticFile<T>
where
    T: Into<String>,
{
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let path: String = self.0.into();
        match Asset::get(path.as_str()) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                HttpResponse::Ok()
                    .content_type(mime)
                    .body(BoxBody::new(content.data.into_owned()))
            }
            None => HttpResponse::NotFound().finish(),
        }
    }
}
