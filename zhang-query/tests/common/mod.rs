//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use zhang_ast::{Directive, Spanned};
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};

/// Load a ledger from a directory and entry file.
pub fn load_ledger(dir: PathBuf, entry: &str) -> Ledger {
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, entry.to_owned(), Arc::new(source)).expect("cannot load ledger")
}

/// Load a ledger from text written to a temporary directory.
pub fn load_text(content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    load_ledger(dir, "main.zhang")
}

/// Load a ledger from text, with the current time read from `clock`.
pub fn load_text_at(content: &str, clock: zhang_core::clock::Clock) -> Ledger {
    use zhang_core::data_type::DataType;
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    let directives = ZhangDataType {}
        .transform(content.to_owned(), Some("main.zhang".to_owned()))
        .expect("parse ledger");
    Ledger::process(LedgerProcessContext {
        directives,
        entry: (dir, "main.zhang".to_owned()),
        dialect: zhang_core::data_type::Dialect::Zhang,
        visited_files: vec![],
        data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
        clock,
    })
    .expect("cannot load ledger")
}

/// Load a ledger from text, with `transform` changing its parsed directives first, as a plugin
/// changes the stream it reads.
pub fn load_transformed(content: &str, transform: impl FnOnce(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    let source: Arc<dyn DataSource> = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source
        .load(dir.to_string_lossy().into_owned(), "main.zhang".to_owned())
        .expect("cannot read ledger");
    Ledger::process(LedgerProcessContext {
        directives: transform(loaded.directives),
        entry: (dir, "main.zhang".to_owned()),
        dialect: zhang_core::data_type::Dialect::Zhang,
        visited_files: loaded.visited_files,
        data_source: source,
        clock: zhang_core::clock::Clock::System,
    })
    .expect("cannot process ledger")
}

/// The fava demo ledger shipped with the integration tests.
pub fn fava_demo_ledger() -> Ledger {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
    load_ledger(dir, "main.zhang")
}
