use crate::core::ledger::Ledger;
use crate::server::model::LedgerSchema;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::core::account::Account;
use crate::core::data::{Date, Document};
use crate::core::models::{Directive, ZhangString};
use crate::server::model::mutation::create_folder_if_not_exist;
use crate::server::response::{AccountResponse, DocumentResponse};
use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::body::{BoxBody, EitherBody};
use actix_web::http::Uri;
use actix_web::web::{Data, Json};
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};

use bigdecimal::{BigDecimal, FromPrimitive};
use chrono::{Datelike, Local};
use futures_util::StreamExt;
use itertools::Itertools;
use log::info;
use rust_embed::RustEmbed;
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

#[post("/api/{account_name}/documents")]
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

#[get("/api/{account_name}/documents")]
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
