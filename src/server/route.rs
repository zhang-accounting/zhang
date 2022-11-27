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
use crate::server::response::{AccountResponse, DocumentResponse};
use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::http::Uri;
use actix_web::web::{Data, Json, Path};
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};

use crate::core::amount::Amount;
use crate::server::request::AccountBalanceRequest;
use bigdecimal::{BigDecimal, FromPrimitive};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use futures_util::StreamExt;
use itertools::Itertools;
use log::info;
use rust_embed::RustEmbed;
use serde::Serialize;
use sqlx::FromRow;
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
        balance_number: f64,
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
            commodities.insert(row.balance_commodity, BigDecimal::from_f64(row.balance_number).unwrap());
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
        inferred_unit_number: f64,
        inferred_unit_commodity: String,
        account_after_number: f64,
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
        total_amount: f64,
        latest_price_date: Option<NaiveDateTime>,
        latest_price_amount: Option<f64>,
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
        total_amount: f64,
        latest_price_date: Option<NaiveDateTime>,
        latest_price_amount: Option<f64>,
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
        amount: f64,
        price_amount: Option<f64>,
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
        datetime: Option<NaiveDateTime>,
        amount: f64,
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
