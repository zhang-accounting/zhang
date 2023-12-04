use std::sync::Arc;

use actix_files::NamedFile;
use actix_web::web::{Data, Path};
use actix_web::{get, Responder};
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;

use crate::response::{DocumentResponse, ResponseWrapper};
use crate::ApiResult;

#[get("/api/documents/{file_path}")]
pub async fn download_document(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> impl Responder {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(filename);

    NamedFile::open_async(full_path).await
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
