use std::sync::Arc;

use axum::extract::State;
use tokio::sync::RwLock;

use zhang_core::ledger::Ledger;

use crate::request::FileUpdateRequest;
use crate::response::{FileDetailResponse, ResponseWrapper};
use crate::{ApiResult, ReloadSender};

pub async fn get_files(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<Option<String>>> {
    let ledger = ledger.read().await;
    let entry_path = &ledger.entry.0;

    let mut ret = vec![];
    for path in &ledger.visited_files {
        if let Some(striped_path) = path.strip_prefix(entry_path).ok() {
            ret.push(striped_path.to_str().map(|it| it.to_string()));
        }
    }
    ResponseWrapper::json(ret)
}

pub async fn get_file_content(ledger: State<Arc<RwLock<Ledger>>>, path: axum::extract::Path<(String,)>) -> ApiResult<FileDetailResponse> {
    let encoded_file_path = path.0 .0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;

    let content = ledger.transformer.get_content(filename.to_owned())?;
    let content = String::from_utf8(content).unwrap();

    ResponseWrapper::json(FileDetailResponse { path: filename, content })
}

pub async fn update_file_content(
    ledger: State<Arc<RwLock<Ledger>>>, reload_sender: State<Arc<ReloadSender>>, path: axum::extract::Path<(String,)>,
    axum::extract::Json(payload): axum::extract::Json<FileUpdateRequest>,
) -> ApiResult<()> {
    let encoded_file_path = path.0 .0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;

    // todo(refact) check if the syntax valid
    // if parse_zhang(&payload.content, None).is_ok() {
    ledger.transformer.save_content(&ledger, filename, payload.content.as_bytes())?;
    reload_sender.reload();
    ResponseWrapper::<()>::created()
}
