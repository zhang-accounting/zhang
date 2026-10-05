//! Follow-up to #495: a document of a ledger on a remote data source is served even when its cache cannot be written,
//! as when `zhang serve` runs in a working directory it cannot write to. The cache is `.cache/documents/` in the
//! working directory, so this test moves there; it is alone in its binary, so nothing else runs in that directory.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::IntoResponse;
use tokio::sync::RwLock;
use zhang_core::data_source::{DataSource, LoadResult};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;
use zhang_core::{ZhangError, ZhangResult};
use zhang_server::routes::document::download_document;
use zhang_server::routes::Base64Path;
use zhang_server::state::SharedLedger;

/// a remote source, as a bucket: a main file naming a document, and the document, nothing on the local disk
struct Remote;

impl DataSource for Remote {
    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let main = "1970-01-01 open Assets:Cash\n2024-01-01 document Assets:Cash \"attachments/a.pdf\"\n";
        Ok(LoadResult {
            directives: ZhangDataType {}.transform(main.to_owned(), Some(endpoint.clone()))?,
            visited_files: vec![PathBuf::from(entry).join(endpoint)],
            missing_includes: vec![],
        })
    }

    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        match path.as_str() {
            "attachments/a.pdf" => Ok(b"the statement".to_vec()),
            _ => Err(ZhangError::FileNotFound),
        }
    }
}

/// a logger keeping what is logged at warn level or above
struct Warnings(Mutex<Vec<String>>);

impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static WARNINGS: Warnings = Warnings(Mutex::new(Vec::new()));

/// the status and the body of the download of `path`
async fn download(state: &State<SharedLedger>, path: &str) -> (u16, Vec<u8>) {
    let response = download_document(state.clone(), Base64Path(path.to_owned())).await.into_response();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, body.to_vec())
}

/// In a working directory the server cannot write to, a document on a remote source is served, only not cached: each
/// download reads it from the source and says once why it is not kept. Nothing is written, not even a file set aside,
/// and a document the source does not have is still missing.
#[tokio::test]
async fn a_remote_document_is_served_when_its_cache_cannot_be_written() {
    log::set_logger(&WARNINGS).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let cwd = tempfile::tempdir().unwrap();
    std::fs::set_permissions(cwd.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    // a user the system lets write anywhere, as root, caches the document: nothing to check
    if std::fs::create_dir(cwd.path().join("probe")).is_ok() {
        return;
    }
    std::env::set_current_dir(cwd.path()).unwrap();
    let ledger = Ledger::async_load(PathBuf::from("/bucket/ledger"), "main.zhang".to_owned(), Arc::new(Remote))
        .await
        .unwrap();
    let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));
    WARNINGS.0.lock().unwrap().clear();

    for _ in 0..2 {
        assert_eq!(download(&state, "attachments/a.pdf").await, (200, b"the statement".to_vec()));
    }
    assert_eq!(download(&state, "attachments/missing.pdf").await.0, 404);

    assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), 0, "nothing is written in the working directory");
    let warnings = WARNINGS.0.lock().unwrap().clone();
    assert_eq!(warnings.len(), 2, "one warning per download served uncached: {warnings:?}");
    for warning in &warnings {
        assert!(warning.contains("\"attachments/a.pdf\"") && warning.contains(".cache/documents"), "{warning}");
    }
    std::fs::set_permissions(cwd.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}
