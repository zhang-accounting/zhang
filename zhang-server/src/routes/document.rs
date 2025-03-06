use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{AppendHeaders, IntoResponse};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use gotcha::api;
use itertools::Itertools;
use log::info;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;

use crate::response::{DocumentResponse, ResponseWrapper};
use crate::state::SharedLedger;
use crate::util::cacheable_data;
use crate::ApiResult;

// TODO(refact) #[api(group = "document")]
pub async fn download_document(ledger: State<SharedLedger>, path: Path<(String,)>) -> impl IntoResponse {
    let encoded_file_path = path.0 .0;
    let filename = String::from_utf8(BASE64_STANDARD.decode(&encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(filename);
    let striped_path = full_path.strip_prefix(entry).unwrap();
    let file_name = striped_path.file_name().unwrap().to_string_lossy().to_string();
    let content = cacheable_data(&encoded_file_path, async {
        info!("loading file [{:?}] data from remote...", &striped_path);
        ledger.data_source.async_get(striped_path.to_string_lossy().to_string()).await
    })
    .await
    .expect("cannot get file data");
    let bytes = Bytes::from(content);
    let headers = AppendHeaders([(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", file_name))]);
    (headers, bytes)
}

#[api(group = "document")]
pub async fn get_documents(ledger: State<SharedLedger>) -> ApiResult<Vec<DocumentResponse>> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
        .cloned()
        .rev()
        .map(|doc| DocumentResponse {
            datetime: doc.datetime.naive_local(),
            filename: doc.filename.unwrap_or_default(),
            path: doc.path.clone(),
            extension: mime_guess::from_path(doc.path).first().map(|it| it.to_string()),
            account: doc.document_type.as_account(),
            trx_id: doc.document_type.as_trx(),
        })
        .collect_vec();

    ResponseWrapper::json(rows)
}
