use axum::extract::State;
use axum::http::header;
use axum::response::{AppendHeaders, IntoResponse};
use base64::engine::general_purpose::URL_SAFE as BASE64_URL_SAFE;
use base64::Engine as _;
use bytes::Bytes;
use gotcha::api;
use itertools::Itertools;
use log::info;

use crate::error::ServerError;
use crate::response::{DocumentEntity, ResponseWrapper};
use crate::routes::Base64Path;
use crate::state::SharedLedger;
use crate::util::cacheable_data;
use crate::{journals, ApiResult, ServerResult};

// #[api(group = "document")]
pub async fn download_document(ledger: State<SharedLedger>, Base64Path(path): Base64Path) -> ServerResult<impl IntoResponse> {
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&path);
    let striped_path = full_path.strip_prefix(entry).map_err(|_| ServerError::BadRequest)?;
    let file_name = striped_path.file_name().ok_or(ServerError::BadRequest)?.to_string_lossy().to_string();
    // the cache file is named after the path in the url-safe alphabet: the standard one can contain `/`, and the
    // two are the same for the paths without `+` or `/` in their standard encoding, so their cache stays valid
    let cache_id = BASE64_URL_SAFE.encode(&path);
    let content = cacheable_data(&cache_id, async {
        info!("loading file [{:?}] data from remote...", striped_path);
        ledger.data_source.async_get(striped_path.to_string_lossy().to_string()).await
    })
    .await?;
    let bytes = Bytes::from(content);
    let headers = AppendHeaders([(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", file_name))]);
    Ok((headers, bytes))
}

/// Every document of the ledger, newest first: the built-in query `journals.documents`.
#[api(group = "document")]
pub async fn get_documents(ledger: State<SharedLedger>) -> ApiResult<Vec<DocumentEntity>> {
    ResponseWrapper::json(journals::documents(&ledger).await?)
}

/// The hand-written [`get_documents`] the built-in query replaces, kept to compare them until it is
/// removed (#479).
pub async fn get_documents_legacy(ledger: State<SharedLedger>) -> ApiResult<Vec<DocumentEntity>> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
        .cloned()
        .rev()
        .map(|doc| DocumentEntity {
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
