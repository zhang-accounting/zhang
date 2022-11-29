use crate::core::ledger::Ledger;
use crate::server::model::LedgerSchema;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::core::account::Account;
use crate::core::data::{Balance, BalanceCheck, BalancePad, Date, Document};
use crate::core::models::{Directive, ZhangString};
use crate::server::model::mutation::create_folder_if_not_exist;
use crate::server::response::{AccountResponse, DocumentResponse, InfoForNewTransaction, JournalBalancePadItemResponse, JournalItemResponse, JournalTransactionItemResponse, JournalTransactionPostingResponse, MetaResponse};
use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::http::Uri;
use actix_web::web::{Data, Json, Path, Query};
use actix_web::{get, post, put, web, HttpRequest, HttpResponse, Responder};

use crate::core::amount::Amount;
use crate::error::ZhangResult;
use crate::parse_zhang;
use crate::server::request::{AccountBalanceRequest, FileUpdateRequest, JournalRequest, StatisticRequest};
use bigdecimal::{BigDecimal, FromPrimitive};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use futures_util::StreamExt;
use itertools::Itertools;
use log::info;
use rust_embed::RustEmbed;
use serde::Serialize;
use sqlx::{FromRow, SqliteConnection};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use crate::core::database::type_ext::big_decimal::ZhangBigDecimal;

// pub async fn graphql_playground() -> impl IntoResponse {
//     Html(playground_source(GraphQLPlaygroundConfig::new("/graphql")))
// }
//
// pub async fn graphql_handler(schema: Extension<LedgerSchema>, req: GraphQLRequest) -> GraphQLResponse {
//     schema.execute(req.0).await.into()
// }

pub async fn get_metas(type_: &str, type_identifier:&str, conn: &mut SqliteConnection) -> ZhangResult<Vec<MetaResponse>> {

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
    #[derive(FromRow)]
    struct ValueRow {
        value: String,
    }
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
pub async fn get_info_for_new_transactions(ledger: Data<Arc<RwLock<Ledger>>>) -> impl Responder {
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
    .await
    .unwrap();
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
    .await
    .unwrap();

    Json(InfoForNewTransaction {
        payee: payees.into_iter().map(|it| it.payee).collect_vec(),
        account_name: account_names.into_iter().map(|it| it.name).collect_vec(),
    })
}

#[get("/api/statistic")]
pub async fn get_statistic_data(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<StatisticRequest>) -> impl Responder {
    "unimplemented!()"
}

#[get("/api/journals")]
pub async fn get_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> impl Responder {
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;
    let params = params.into_inner();
    #[derive(Debug, FromRow)]
    struct JournalHeader {
        id: String,
        datetime: NaiveDateTime,
        journal_type: String,
        payee: String,
        narration: Option<String>,
    }
    let journal_headers = sqlx::query_as::<_, JournalHeader>(
        r#"
        SELECT id, datetime, type as journal_type, payee, narration FROM transactions ORDER BY "sequence" DESC LIMIT $1 OFFSET $2
        "#,
    )
        .bind(params.limit())
        .bind(params.offset())
    .fetch_all(&mut connection)
    .await
    .unwrap();

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
    .await
    .unwrap();

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
                    let tags = get_transaction_tags(&trx_id, &mut connection).await.unwrap();
                    let links = get_transaction_links(&trx_id, &mut connection).await.unwrap();
                    let metas = get_metas("TransactionMeta", &trx_id, &mut connection).await.unwrap();
                    JournalItemResponse::Transaction(JournalTransactionItemResponse {
                        id: trx_id,
                        datetime: header.datetime,
                        payee: header.payee,
                        narration: header.narration,
                        tags,
                        links,
                        flag: header.journal_type,
                        is_balanced: true,
                        postings,
                        metas
                    })
                }
            };
            ret.push(item);
        }
    }
    Json(ret)
}

#[get("/api/documents/{file_path}")]
pub async fn download_document(ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>) -> impl Responder {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(filename);

    NamedFile::open_async(full_path).await
}

pub async fn serve_frontend(uri: Uri) -> impl Responder {
    dbg!(&uri);
    let path = uri.path().trim_start_matches('/').to_string();
    let buf = PathBuf::from_str(dbg!(&path)).unwrap();
    if dbg!(dbg!(buf).extension()).is_some() {
        StaticFile(path)
    } else {
        StaticFile("index.html".to_string())
    }
}

#[get("/api/accounts")]
pub async fn get_account_list(ledger: Data<Arc<RwLock<Ledger>>>) -> impl Responder {
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
    .await
    .unwrap();
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
    Json(ret)
}

#[get("/api/documents")]
pub async fn get_documents(ledger: Data<Arc<RwLock<Ledger>>>) -> impl Responder {
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
    .await
    .unwrap();
    Json(rows)
}

#[post("/api/accounts/{account_name}/documents")]
pub async fn upload_account_document(
    ledger: Data<Arc<RwLock<Ledger>>>, mut multipart: Multipart, path: web::Path<(String,)>,
) -> impl Responder {
    let account_name = path.into_inner().0;
    let ledger_stage = ledger.read().await;
    let entry = &ledger_stage.entry.0;
    let mut documents = vec![];

    while let Some(item) = multipart.next().await {
        let mut field = item.unwrap();
        let name = field.name().to_string();
        let file_name = field.content_disposition().get_filename().unwrap().to_string();
        let content_type = field.content_type().type_().as_str().to_string();

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
                account: Account::from_str(&account_name).unwrap(),
                filename: ZhangString::QuoteString(path.to_string()),
                tags: None,
                links: None,
                meta: Default::default(),
            }));
        }
    }
    let time = Local::now().naive_local();
    ledger_stage.append_directives(documents, format!("data/{}/{}.zhang", time.year(), time.month()));
    "OK"
}

#[get("/api/accounts/{account_name}/documents")]
pub async fn get_account_documents(ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>) -> impl Responder {
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
    .await
    .unwrap();

    Json(rows)
}

#[get("/api/accounts/{account_name}/journals")]
pub async fn get_account_journals(ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>) -> impl Responder {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    #[derive(FromRow, Serialize)]
    struct AccountJournalItem {
        datetime: NaiveDateTime,
        trx_id: String,
        payee: String,
        narration: Option<String>,
        inferred_unit_number: ZhangBigDecimal,
        inferred_unit_commodity: String,
        account_after_number: ZhangBigDecimal,
        account_after_commodity: String,
    }

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
    .await
    .unwrap();

    Json(rows)
}

#[post("/api/accounts/{account_name}/balances")]
pub async fn create_account_balance(
    ledger: Data<Arc<RwLock<Ledger>>>, params: web::Path<(String,)>, Json(payload): Json<AccountBalanceRequest>,
) -> impl Responder {
    let account_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    let balance = match payload {
        AccountBalanceRequest::Check { amount, commodity } => Balance::BalanceCheck(BalanceCheck {
            date: Date::Datetime(Local::now().naive_local()),
            account: Account::from_str(&account_name).unwrap(),
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
            account: Account::from_str(&account_name).unwrap(),
            amount: Amount::new(amount, commodity),
            tolerance: None,
            diff_amount: None,
            meta: Default::default(),
            pad: Account::from_str(&pad_account).unwrap(),
        }),
    };
    let time = Local::now().naive_local();
    ledger.append_directives(
        vec![Directive::Balance(balance)],
        format!("data/{}/{}.zhang", time.year(), time.month()),
    );
    "OK"
}

#[get("/api/commodities")]
pub async fn get_all_commodities(ledger: Data<Arc<RwLock<Ledger>>>) -> impl Responder {
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;
    #[derive(FromRow, Serialize)]
    struct CommodityListItem {
        name: String,
        precision: i32,
        prefix: Option<String>,
        suffix: Option<String>,
        rounding: Option<String>,
        total_amount: ZhangBigDecimal,
        latest_price_date: Option<NaiveDateTime>,
        latest_price_amount: Option<ZhangBigDecimal>,
        latest_price_commodity: Option<String>,
    }
    let vec = sqlx::query_as::<_, CommodityListItem>(
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
    .await
    .unwrap();
    Json(vec)
}

#[get("/api/commodities/{commodity_name}")]
pub async fn get_single_commodity(ledger: Data<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> impl Responder {
    let commodity_name = params.into_inner().0;
    let ledger = ledger.read().await;
    let mut connection = ledger.connection().await;

    #[derive(FromRow, Serialize)]
    struct CommodityListItem {
        name: String,
        precision: i32,
        prefix: Option<String>,
        suffix: Option<String>,
        rounding: Option<String>,
        total_amount: ZhangBigDecimal,
        latest_price_date: Option<NaiveDateTime>,
        latest_price_amount: Option<ZhangBigDecimal>,
        latest_price_commodity: Option<String>,
    }
    let basic_info = sqlx::query_as::<_, CommodityListItem>(
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
        .await
        .unwrap();

    #[derive(FromRow, Serialize)]
    struct CommodityLot {
        datetime: Option<NaiveDateTime>,
        amount: ZhangBigDecimal,
        price_amount: Option<ZhangBigDecimal>,
        price_commodity: Option<String>,
        account: String,
    }

    let lots = sqlx::query_as::<_, CommodityLot>(
        r#"
            select datetime, amount, price_amount, price_commodity, account
            from commodity_lots
            where commodity = $1
    "#,
    )
    .bind(&commodity_name)
    .fetch_all(&mut connection)
    .await
    .unwrap();

    #[derive(FromRow, Serialize)]
    struct CommodityPrice {
        datetime: NaiveDateTime,
        amount: ZhangBigDecimal,
        target_commodity: Option<String>,
    }

    let prices = sqlx::query_as::<_, CommodityPrice>(
        r#"
            select datetime, amount, target_commodity
            from prices
            where commodity = $1
    "#,
    )
    .bind(&commodity_name)
    .fetch_all(&mut connection)
    .await
    .unwrap();

    #[derive(Serialize)]
    struct CommodityDetail {
        info: CommodityListItem,
        lots: Vec<CommodityLot>,
        prices: Vec<CommodityPrice>,
    }

    Json(CommodityDetail {
        info: basic_info,
        lots,
        prices,
    })
}

#[get("/api/files")]
pub async fn get_files(ledger: Data<Arc<RwLock<Ledger>>>) -> impl Responder {
    let ledger = ledger.read().await;
    let entry_path = &ledger.entry.0;
    let vec = ledger
        .visited_files
        .iter()
        .map(|path| path.strip_prefix(entry_path).unwrap().to_str().map(|it| it.to_string()))
        .collect_vec();
    Json(vec)
}

#[get("/api/files/{file_path}")]
pub async fn get_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>) -> impl Responder {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    #[derive(Debug, Serialize)]
    struct FileDetail {
        path: String,
        content: String,
    }
    Json(FileDetail {
        path: filename,
        content: std::fs::read_to_string(full_path).unwrap(),
    })
}
#[put("/api/files/{file_path}")]
pub async fn update_file_content(
    ledger: Data<Arc<RwLock<Ledger>>>, path: web::Path<(String,)>, Json(payload): Json<FileUpdateRequest>,
) -> impl Responder {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    if parse_zhang(&payload.content, None).is_ok() {
        std::fs::write(full_path, payload.content).unwrap()
    }
    "ok"
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

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let path: String = self.0.into();
        match Asset::get(dbg!(path.as_str())) {
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
