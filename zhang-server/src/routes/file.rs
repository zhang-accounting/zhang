use axum::extract::State;
use gotcha::api;

use crate::request::FileUpdateRequest;
use crate::response::{Created, FileDetailEntity, ResponseWrapper};
use crate::routes::Base64Path;
use crate::state::{SharedLedger, SharedReloadSender};
use crate::{ApiResult, ServerResult};

#[api(group = "file")]
pub async fn get_files(ledger: State<SharedLedger>) -> ApiResult<Vec<Option<String>>> {
    let ledger = ledger.read().await;
    let entry_path = &ledger.entry.0;

    let mut ret = vec![];
    for path in &ledger.visited_files {
        if let Ok(striped_path) = path.strip_prefix(entry_path) {
            ret.push(striped_path.to_str().map(|it| it.to_string()));
        }
    }
    ResponseWrapper::json(ret)
}

#[api(group = "file")]
pub async fn get_file_content(ledger: State<SharedLedger>, Base64Path(filename): Base64Path) -> ApiResult<FileDetailEntity> {
    let ledger = ledger.read().await;

    let content = ledger.data_source.async_get(filename.to_owned()).await?;
    let content = String::from_utf8(content).unwrap();

    ResponseWrapper::json(FileDetailEntity { path: filename, content })
}

#[api(group = "file")]
pub async fn update_file_content(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Base64Path(filename): Base64Path,
    axum::extract::Json(payload): axum::extract::Json<FileUpdateRequest>,
) -> ServerResult<Created> {
    let ledger = ledger.read().await;

    // todo(refact) check if the syntax valid
    // if parse_zhang(&payload.content, None).is_ok() {
    ledger.data_source.async_save(&ledger, filename, payload.content.as_bytes()).await?;
    reload_sender.reload();
    Ok(Created)
}
