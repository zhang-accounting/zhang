//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

/// Load a ledger from a directory and entry file.
pub fn load_ledger(dir: PathBuf, entry: &str) -> Ledger {
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, entry.to_owned(), Arc::new(source)).expect("cannot load ledger")
}

/// Load a ledger from text written to a temporary directory.
pub fn load_text(content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").into_path();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    load_ledger(dir, "main.zhang")
}

/// The fava demo ledger shipped with the integration tests.
pub fn fava_demo_ledger() -> Ledger {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
    load_ledger(dir, "main.zhang")
}
