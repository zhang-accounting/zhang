//! The ids zhang gives the transactions and the balance assertions of a ledger appear in URLs, so they must not
//! change between versions. This pins them over every ledger of `integration-tests`.
//!
//! An id is derived from the place of its directive, the file and the offset in it ([`FromSpan::from_span`]), and,
//! when another transaction or assertion has that id already, the `n`-th id derived from it ([`FromSpan::derived`]).
//! The file is an absolute path, which differs from one checkout to another, so the golden file keeps each id as its
//! derivation: the kind of the directive, its file within the ledger, its offset and `n`. Equal derivations of the same
//! directives are equal ids. Set `UPDATE_TRANSACTION_IDS=1` to write the golden file anew.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beancount::Beancount;
use uuid::Uuid;
use zhang_ast::{Directive, Spanned};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_core::utils::id::FromSpan;

const GOLDEN: &str = "tests/transaction_ids.txt";

fn integration_tests() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests").canonicalize().unwrap()
}

/// `id` as derived from the place of `directive` in the ledger at `root`: `file:offset #n`
fn derivation(root: &Path, directive: &Spanned<Directive>, id: Uuid) -> String {
    let span = &directive.span;
    let base = Uuid::from_span(span);
    let n = (0..1000)
        .find(|n| id == if *n == 0 { base } else { Uuid::derived(&base, *n) })
        .unwrap_or_else(|| panic!("the id {id} is not derived from the place of its directive {span:?}"));
    let file = span
        .filename
        .as_deref()
        .map(|file| file.strip_prefix(root).unwrap_or(file).display().to_string());
    format!("{}:{} #{n}", file.unwrap_or_default(), span.start)
}

/// the ids of the transactions and the balance assertions of `ledger`, in the order zhang processed them
fn ids(root: &Path, ledger: &Ledger) -> Vec<String> {
    let mut ids = ledger
        .directives
        .iter()
        .zip(&ledger.outcomes)
        .filter_map(|(directive, outcome)| match outcome.detail {
            Detail::Transaction { id, .. } => Some((outcome.seq, "transaction", directive, id)),
            Detail::Assertion { id, .. } => Some((outcome.seq, "balance", directive, id)),
            _ => None,
        })
        .collect::<Vec<_>>();
    ids.sort_by_key(|(seq, ..)| *seq);
    ids.into_iter()
        .map(|(_, kind, directive, id)| format!("{kind} {}", derivation(root, directive, id)))
        .collect()
}

/// ledgers whose directives share their places, so that ids are derived: a `balance ... with pad` and its padding, a
/// `pad` padding several currencies, in both formats
const SHARED_PLACES: &[(&str, &str)] = &[
    (
        "main.zhang",
        "1970-01-01 open Assets:Bank\n1970-01-01 open Equity:Open\n1970-01-01 open Expenses:Food\n\
         2024-01-01 balance Assets:Bank 100 CNY with pad Equity:Open\n\
         2024-01-02 * \"lunch\"\n  Assets:Bank -10 CNY\n  Expenses:Food\n\
         2024-01-03 balance Assets:Bank 50 CNY with pad Equity:Open\n\
         2024-01-03 balance Assets:Bank 5 USD with pad Equity:Open\n",
    ),
    (
        "main.bean",
        "1970-01-01 open Assets:Bank\n1970-01-01 open Equity:Opening\n\
         2024-01-02 pad Assets:Bank Equity:Opening\n\
         2024-01-03 balance Assets:Bank 10 USD\n\
         2024-01-03 balance Assets:Bank 20 EUR\n",
    ),
];

/// the ledger of the file `main` in the directory `root`, read in the format of its extension
fn load(root: &Path, main: &str) -> Ledger {
    let ledger = if main.ends_with(".bean") {
        Ledger::load_with_data_source(
            root.to_path_buf(),
            main.to_owned(),
            Arc::new(LocalFileSystemDataSource::new(Beancount::default())),
        )
    } else {
        Ledger::load_with_data_source(root.to_path_buf(), main.to_owned(), Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})))
    };
    ledger.unwrap_or_else(|error| panic!("{}/{main} loads: {error}", root.display()))
}

#[test]
fn every_transaction_and_balance_assertion_keeps_its_id() {
    let mut cases = std::fs::read_dir(integration_tests())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    cases.sort();
    let mut text = String::new();
    for (main, content) in SHARED_PLACES {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join(main), content).unwrap();
        writeln!(text, "## shared places/{main}").unwrap();
        for id in ids(&root, &load(&root, main)) {
            writeln!(text, "{id}").unwrap();
        }
    }
    for case in cases {
        for main in ["main.zhang", "main.bean"] {
            if !case.join(main).exists() {
                continue;
            }
            let ledger = load(&case, main);
            writeln!(text, "## {}/{main}", case.file_name().unwrap().to_string_lossy()).unwrap();
            for id in ids(&case, &ledger) {
                writeln!(text, "{id}").unwrap();
            }
        }
    }
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    if std::env::var_os("UPDATE_TRANSACTION_IDS").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap();
    assert!(text == expected, "the ids differ from {GOLDEN}");
}
