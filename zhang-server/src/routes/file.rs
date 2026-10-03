use axum::extract::State;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use gotcha::api;

use crate::error::ServerError;
use crate::request::FileUpdateRequest;
use crate::response::{Created, FileDetailEntity, ResponseWrapper};
use crate::state::{wrote, SharedLedger, SharedReloadSender};
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
pub async fn get_file_content(ledger: State<SharedLedger>, path: axum::extract::Path<(String,)>) -> ApiResult<FileDetailEntity> {
    let encoded_file_path = path.0 .0;
    let filename = String::from_utf8(BASE64_STANDARD.decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;

    let content = ledger.data_source.async_get(filename.to_owned()).await?;
    let content = String::from_utf8(content).unwrap();

    ResponseWrapper::json(FileDetailEntity { path: filename, content })
}

#[api(group = "file")]
pub async fn update_file_content(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: axum::extract::Path<(String,)>,
    axum::extract::Json(payload): axum::extract::Json<FileUpdateRequest>,
) -> ServerResult<Created> {
    let encoded_file_path = path.0 .0;
    let filename = String::from_utf8(BASE64_STANDARD.decode(encoded_file_path).unwrap()).unwrap();
    // the whole file: it edits no place the ledger loaded, so it needs the ledger reloaded first no more than a
    // ledger that loads. It is saved even when the files cannot be loaded, to fix them
    let mut ledger = ledger.write().await;

    let saved = ledger.data_source.async_save(&ledger, filename, payload.content.as_bytes()).await;
    wrote(&mut ledger, &reload_sender, saved.map_err(ServerError::from))?;
    Ok(Created)
}

#[cfg(test)]
mod save_test {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;
    use tokio::sync::{mpsc, RwLock};
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::ledger::Ledger;

    use super::update_file_content;
    use crate::request::{CreateTransactionRequest, FileUpdateRequest};
    use crate::routes::transaction::create_new_transaction;
    use crate::state::{SharedLedger, SharedReloadSender};
    use crate::ReloadSender;

    async fn answer(response: impl IntoResponse) -> (StatusCode, String) {
        let response = response.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|it| it["message"].as_str().map(str::to_owned))
            .unwrap_or_default();
        (status, message)
    }

    /// A save of a file that does not parse is written, and so is the save fixing it: a save edits no place the
    /// ledger loaded, so it never waits on the ledger loading. A write that edits the ledger as loaded is refused in
    /// between, with what to do, and written once the files are fixed.
    #[tokio::test]
    async fn a_file_that_does_not_parse_can_be_fixed_in_the_editor() {
        let dir = std::env::temp_dir().join(format!("zhang-file-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.bean");
        let ledger = "option \"operating_currency\" \"CNY\"\n2020-01-01 commodity CNY\n2024-01-01 open Assets:A\n2024-01-01 open Income:X\n";
        std::fs::write(&main, ledger).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger");
        let state = State(SharedLedger(Arc::new(RwLock::new(loaded))));
        let (sender, _receiver) = mpsc::channel(8);
        let reload = State(SharedReloadSender(Arc::new(ReloadSender(sender))));
        let save = |content: String| {
            let path = Path((BASE64_STANDARD.encode(main.to_string_lossy().as_bytes()),));
            update_file_content(state.clone(), reload.clone(), path, Json(FileUpdateRequest { content }))
        };
        let create = || {
            let request: CreateTransactionRequest = serde_json::from_value(serde_json::json!({
                "datetime": "2024-06-01T12:00:00Z",
                "payee": "Shop",
                "flag": "Okay",
                "narration": "seed",
                "postings": [
                    {"account": "Assets:A", "unit": {"number": "50", "commodity": "CNY"}},
                    {"account": "Income:X", "unit": null}
                ],
                "metas": [],
                "tags": [],
                "links": []
            }))
            .unwrap();
            create_new_transaction(state.clone(), reload.clone(), Json(request))
        };

        let broken = format!("{ledger}this is not beancount\n");
        assert_eq!(answer(save(broken.clone()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), broken);

        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.contains("the ledger cannot be loaded from its files as they are now, so nothing was written"),
            "{message}"
        );
        assert!(message.contains("Fix them in the file editor, then try again: cannot parse"), "{message}");
        assert!(message.contains("main.bean"), "{message}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), broken, "nothing is written");

        assert_eq!(answer(save(ledger.to_owned()).await).await.0, StatusCode::CREATED);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), ledger);

        let (status, message) = answer(create().await).await;
        assert_eq!(status, StatusCode::OK, "{message}");
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let reloaded = Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger");
        let store = reloaded.store.read().unwrap();
        assert!(store.errors.is_empty(), "{:?}", store.errors);
        assert_eq!(store.transactions.len(), 1);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }
}
