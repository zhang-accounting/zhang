use std::path::PathBuf;
use std::str::FromStr;

use beancount::directives::BeancountOnlyDirective;
use beancount::parser::parse;
use bigdecimal::BigDecimal;
use itertools::Either;
use zhang_ast::{Directive, Flag, SpanInfo, Spanned, Transaction};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::inventory::TransactionInference;

fn txn(content: &str) -> Transaction {
    let d = parse(content, None::<PathBuf>).unwrap().into_iter().find_map(|s| s.data.left()).unwrap();
    match d {
        Directive::Transaction(t) => t,
        _ => panic!("expected txn"),
    }
}

#[test]
fn tolerance_and_txn_parse_correctly() {
    let d = parse("2014-01-01 balance Assets:Cash 100 ~ 0.5 USD\n", None::<PathBuf>)
        .unwrap()
        .pop()
        .unwrap()
        .data;
    match d {
        Either::Right(BeancountOnlyDirective::Balance(b)) => assert_eq!(b.tolerance, Some(BigDecimal::from_str("0.5").unwrap())),
        _ => panic!("expected balance"),
    }
    assert_eq!(txn("2014-01-01 txn \"p\" \"n\"\n  Assets:Cash 1 USD\n  Equity:X\n").flag, Some(Flag::Okay));
}

#[test]
fn cost_label_and_total_cost() {
    // label captured
    let t = txn("2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD, \"lot1\"}\n  Equity:X\n");
    assert_eq!(t.postings[0].cost.as_ref().unwrap().label, Some("lot1".to_string()));

    // total cost {{ }} keeps the raw total, and normalises to per-unit for lot bookkeeping
    let t = txn("2014-01-01 * \"x\"\n  Assets:Cash 10 HOOL {{1000 USD}}\n  Equity:X\n");
    let cost = t.postings[0].cost.as_ref().unwrap();
    assert!(cost.total, "total flag set");
    assert_eq!(cost.base.as_ref().unwrap().number, BigDecimal::from(1000), "raw total stored");
    let lot = t.txn_postings()[0].lot_meta();
    assert_eq!(
        lot.cost.unwrap().base.unwrap().number,
        BigDecimal::from(100),
        "1000/10 = 100 per-unit for the lot"
    );

    // round-trips through the zhang exporter
    let exported = ZhangDataType::default().export(Spanned::new(Directive::Transaction(t), SpanInfo::default()));
    assert!(exported.contains("{{"), "total cost re-exports as {{ }}: {exported}");
    let t2 = txn("2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD, \"lot1\"}\n  Equity:X\n");
    let exported2 = ZhangDataType::default().export(Spanned::new(Directive::Transaction(t2), SpanInfo::default()));
    assert!(exported2.contains("\"lot1\""), "label re-exports: {exported2}");
}

#[test]
fn pushmeta_applies_to_following_directives() {
    use beancount::Beancount;
    use zhang_core::data_type::DataType;
    let content = "pushmeta project: \"X\"\n2014-01-01 open Assets:Cash\npopmeta project:\n2014-01-02 open Assets:Bank\n";
    let dirs = Beancount::default().transform(content.to_string(), None).unwrap();
    let opens: Vec<_> = dirs
        .into_iter()
        .filter_map(|s| match s.data {
            Directive::Open(o) => Some(o),
            _ => None,
        })
        .collect();
    assert_eq!(opens.len(), 2);
    assert_eq!(
        opens[0].meta.get_one("project").map(|v| v.as_str().to_string()),
        Some("X".to_string()),
        "inside push/pop gets meta"
    );
    assert_eq!(opens[1].meta.get_one("project"), None, "after popmeta: no meta");
}

#[test]
fn query_directive_round_trips_through_the_beancount_exporter() {
    use beancount::Beancount;
    use zhang_ast::ZhangString;

    let beancount = Beancount::default();
    let content = "2014-07-09 query \"france-balances\" \"SELECT account, sum(position) WHERE 'trip-france-2014' in tags\"\n  owner: \"alice\"\n";
    let directive = beancount.transform(content.to_string(), None).unwrap().pop().unwrap();
    let Directive::Query(query) = &directive.data else {
        panic!("expected a query directive, got {:?}", directive.data);
    };
    assert_eq!(query.name, ZhangString::quote("france-balances"));
    assert_eq!(query.meta.get_one("owner"), Some(&ZhangString::quote("alice")));

    let exported = beancount.export(directive.clone());
    assert_eq!(exported, content.trim_end());
    let reparsed = beancount.transform(exported, None).unwrap().pop().unwrap();
    assert_eq!(reparsed.data, directive.data);
}

#[test]
fn query_directive_keeps_its_time_and_quotes_its_name_for_beancount() {
    use beancount::Beancount;
    use zhang_ast::{Date, ZhangString};

    let beancount = Beancount::default();
    // zhang syntax accepts a bare name and a datetime; beancount needs a quoted name and a date
    let zhang = ZhangDataType::default()
        .transform("2014-07-09 10:30:00 query cash \"SELECT account\"\n".to_string(), None)
        .unwrap()
        .pop()
        .unwrap();
    let exported = beancount.export(zhang);
    assert_eq!(exported, "2014-07-09 query \"cash\" \"SELECT account\"\n  time: \"10:30:00\"");

    let Directive::Query(query) = beancount.transform(exported, None).unwrap().pop().unwrap().data else {
        panic!("expected a query directive");
    };
    assert_eq!(query.name, ZhangString::quote("cash"));
    assert!(matches!(query.date, Date::Datetime(_)), "the time meta is folded back into the date");
    assert_eq!(query.meta.get_one("time"), None);
}

#[test]
fn transaction_metadata_is_exported_before_the_postings() {
    use beancount::Beancount;
    use zhang_ast::ZhangString;

    // beancount 3.2.3 attaches a metadata line that follows a posting to that posting,
    // so transaction metadata has to come first: from this text it loads `memo` on the
    // transaction and none on the postings
    let expected = "2024-01-02 * \"Cafe\" \"coffee\"\n  memo: \"paid\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY";
    let beancount = Beancount::default();
    // the zhang parser reads metadata after the postings, at their indentation, as the
    // transaction's: older zhang wrote it there
    let after = "2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n  memo: \"paid\"\n";
    let directive = ZhangDataType::default().transform(after.to_string(), None).unwrap().pop().unwrap();
    let Directive::Transaction(txn) = &directive.data else {
        panic!("expected a transaction, got {:?}", directive.data);
    };
    assert_eq!(txn.meta.get_one("memo"), Some(&ZhangString::quote("paid")));
    assert!(txn.postings.iter().all(|posting| posting.meta.get_one("memo").is_none()));

    assert_eq!(beancount.export(directive.clone()), expected);
    assert_eq!(ZhangDataType::default().export(directive), expected);

    // the beancount parser, like beancount, reads it as metadata of the last posting
    let directive = beancount.transform(after.to_string(), None).unwrap().pop().unwrap();
    let Directive::Transaction(txn) = &directive.data else {
        panic!("expected a transaction, got {:?}", directive.data);
    };
    assert_eq!(txn.meta.get_one("memo"), None);
    assert_eq!(txn.postings[1].meta.get_one("memo"), Some(&ZhangString::quote("paid")));
    assert_eq!(
        beancount.export(directive),
        "2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n    memo: \"paid\""
    );
}

/// A `document` appended to a file of a beancount ledger, here by the local data source, names its file relative to
/// that file, as beancount reads it, and zhang reads it back by its path within the ledger.
#[test]
fn a_document_appended_to_a_beancount_file_names_its_file_from_there() {
    use std::sync::Arc;

    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::ledger::Ledger;

    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("zhang-document-path-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(dir.join("attachments/u1")).unwrap();
    std::fs::write(dir.join("attachments/u1/a statement.pdf"), "%PDF").unwrap();
    std::fs::write(dir.join("main.bean"), "1970-01-01 open Assets:Cash\n").unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
    let ledger = Ledger::load_with_data_source(dir.clone(), "main.bean".to_owned(), source.clone()).unwrap();
    let document = parse("2024-01-15 document Assets:Cash \"attachments/u1/a statement.pdf\"\n", None::<PathBuf>)
        .unwrap()
        .remove(0)
        .data
        .left()
        .unwrap();

    ledger.data_source.append(&ledger, vec![document]).unwrap();

    let written = std::fs::read_to_string(dir.join("data/2024/01.bean")).unwrap();
    assert!(
        written.contains("2024-01-15 document Assets:Cash \"../../attachments/u1/a statement.pdf\""),
        "{written}"
    );
    let reloaded = Ledger::load_with_data_source(dir.clone(), "main.bean".to_owned(), source).unwrap();
    let store = reloaded.store.read().unwrap();
    assert!(store.errors.is_empty(), "{:?}", store.errors);
    let paths = store.documents.iter().map(|it| it.path.as_str()).collect::<Vec<_>>();
    assert_eq!(paths, vec!["attachments/u1/a statement.pdf"]);
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}

/// On the local disk, whether the document of a `document` exists is looked at with a stat: the source is never asked
/// to list or read anything for it, and what is found is reported.
#[test]
fn on_the_local_disk_documents_are_looked_at_not_listed() {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use zhang_ast::error::ErrorKind;
    use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource, SourceEntry};
    use zhang_core::ledger::Ledger;
    use zhang_core::ZhangResult;

    /// the local source, counting what it is asked besides loading the ledger
    struct Counting(LocalFileSystemDataSource, AtomicUsize);

    impl DataSource for Counting {
        fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.get(path)
        }
        fn local_root(&self, entry: &Path) -> Option<PathBuf> {
            self.0.local_root(entry)
        }
        fn get_limited(&self, path: String, max_len: u64) -> ZhangResult<Vec<u8>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.get_limited(path, max_len)
        }
        fn list(&self, path: String, max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.list(path, max_entries)
        }
        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            self.0.load(entry, endpoint)
        }
    }

    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("zhang-local-documents-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(dir.join("attachments")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("attachments/legacy.pdf"), "%PDF").unwrap();
    std::fs::write(dir.join("main.bean"), "1970-01-01 open Assets:Cash\ninclude \"data/2024.bean\"\n").unwrap();
    std::fs::write(
        dir.join("data/2024.bean"),
        "2024-01-01 document Assets:Cash \"attachments/legacy.pdf\"\n2024-01-02 document Assets:Cash \"missing.pdf\"\n",
    )
    .unwrap();
    let source = Arc::new(Counting(LocalFileSystemDataSource::new(beancount::Beancount {}), AtomicUsize::new(0)));
    let ledger = Ledger::load_with_data_source(dir.clone(), "main.bean".to_owned(), source.clone()).unwrap();

    assert_eq!(source.1.load(Ordering::SeqCst), 0, "nothing is asked of the source for the documents");
    let store = ledger.store.read().unwrap();
    let mut errors = store.errors.iter().map(|it| it.error_type.clone()).collect::<Vec<_>>();
    errors.sort_by_key(|it| format!("{it:?}"));
    assert_eq!(errors, vec![ErrorKind::DocumentNotFound, ErrorKind::DocumentPathRelativeToRoot]);
    let documents = store.documents.iter().map(|it| (it.path.as_str(), it.alternate.clone())).collect::<Vec<_>>();
    assert_eq!(documents, vec![("attachments/legacy.pdf", None), ("data/missing.pdf", None)]);
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}
