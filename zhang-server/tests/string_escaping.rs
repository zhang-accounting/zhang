//! Acceptance test for #442 through the server: a transaction created via the API with `$`, a
//! backtick and a no-break space in its strings must read back unchanged after a reload.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use bigdecimal::BigDecimal;
use chrono::{TimeZone, Utc};
use tokio::sync::RwLock;
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Transaction};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::request::{CreateTransactionPostingRequest, CreateTransactionRequest, MetaRequest};
use zhang_server::routes::transaction::create_new_transaction;
use zhang_server::state::{SharedLedger, SharedReloadSender};
use zhang_server::ReloadSender;

const MAIN: &str = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// A scratch ledger directory under the system temp dir, removed on drop.
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

async fn load(dir: &Path) -> Ledger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::async_load(dir.to_path_buf(), "main.zhang".to_owned(), source)
        .await
        .unwrap_or_else(|error| panic!("ledger should load: {error}"))
}

/// The text of every `.zhang` file under `dir`.
fn ledger_text(dir: &Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            text.push_str(&ledger_text(&path));
        } else if path.extension().is_some_and(|extension| extension == "zhang") {
            text.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    text
}

#[tokio::test]
async fn created_transaction_strings_survive_a_reload() {
    let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-escaping-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&dir.0).unwrap();
    std::fs::write(dir.0.join("main.zhang"), MAIN).unwrap();
    // appending reads the month's data file first, so it has to exist (a separate quirk)
    std::fs::create_dir_all(dir.0.join("data/2024")).unwrap();
    std::fs::write(dir.0.join("data/2024/05.zhang"), "").unwrap();

    let ledger = SharedLedger(Arc::new(RwLock::new(load(&dir.0).await)));
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let reload_sender = SharedReloadSender(Arc::new(ReloadSender::new(sender)));
    let request = CreateTransactionRequest {
        datetime: Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap().into(),
        payee: "Cafe `Central`".to_owned(),
        flag: None,
        narration: Some("coffee $5".to_owned()),
        postings: vec![
            CreateTransactionPostingRequest {
                account: "Expenses:Food".to_owned(),
                unit: Some(Amount::new(BigDecimal::from(5), "CNY").into()),
                metas: None,
                cost: None,
                price: None,
                comment: None,
            },
            CreateTransactionPostingRequest {
                account: "Assets:Cash".to_owned(),
                unit: Some(Amount::new(BigDecimal::from(-5), "CNY").into()),
                metas: None,
                cost: None,
                price: None,
                comment: None,
            },
        ],
        metas: vec![MetaRequest {
            key: "memo".to_owned(),
            value: "paid $5\u{a0}in cash".to_owned(),
        }],
        tags: vec![],
        links: vec![],
    };
    if let Err(error) = create_new_transaction(State(ledger), State(reload_sender), Json(request)).await {
        panic!("creating the transaction should succeed: {error}");
    }

    let reloaded = load(&dir.0).await;
    let errors = reloaded
        .store
        .read()
        .unwrap()
        .errors
        .iter()
        .map(|it| format!("{:?}", it.error_type))
        .collect::<Vec<_>>();
    assert_eq!(errors, Vec::<String>::new(), "the reloaded ledger has errors");
    let created: Vec<&Transaction> = reloaded
        .directives
        .iter()
        .filter_map(|directive| match &directive.data {
            Directive::Transaction(txn) => Some(txn),
            _ => None,
        })
        .collect();
    assert_eq!(created.len(), 1, "exactly the created transaction is loaded");
    let txn = created[0];
    assert_eq!(txn.payee.as_ref().map(|it| it.as_str()), Some("Cafe `Central`"));
    assert_eq!(txn.narration.as_ref().map(|it| it.as_str()), Some("coffee $5"));
    assert_eq!(txn.meta.get_one("memo").map(|it| it.as_str()), Some("paid $5\u{a0}in cash"));

    // nothing above contains a backslash, so any of these in the file is an escape the exporter wrote
    let written = ledger_text(&dir.0);
    for legacy in [r"\$", r"\`", r"\u{"] {
        assert!(!written.contains(legacy), "the ledger file holds the legacy escape {legacy}:\n{written}");
    }
}
