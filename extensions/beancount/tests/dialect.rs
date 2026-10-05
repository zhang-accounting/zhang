//! The format of a ledger is decided once, from its main file, and every rule that differs between zhang and beancount
//! reads it from the ledger: not from the extension of the file a directive is in, nor from a main file name the
//! ledger may not have (the playground's).

use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use zhang_ast::error::ErrorKind;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{DataType, Dialect};
use zhang_core::ledger::{Ledger, LedgerProcessContext};

/// accounts, then a pad and lunch on 2024-03-02, before a balance of that day timed at 20:00, after lunch: beancount
/// checks it at the start of its day, before lunch, and zhang reports that the time it read before is ignored
const TIMED_BALANCE: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:A\n\
                             1970-01-01 open Equity:Open\n1970-01-01 open Expenses:Food\n\
                             2024-03-02 * \"Shop\" \"lunch\"\n  Expenses:Food  30 CNY\n  Assets:A\n\
                             2024-03-01 pad Assets:A Equity:Open\n2024-03-02 balance Assets:A  100 CNY\n  time: \"20:00:00\"\n";

/// the kinds of the errors of `ledger`
fn errors(ledger: &Ledger) -> Vec<ErrorKind> {
    ledger.store.read().unwrap().errors.iter().map(|it| it.error_type.clone()).collect()
}

/// The playground reads its text with the beancount grammar and processes it as a beancount ledger, without a main
/// file or any file: it passed an empty main file name, so its beancount text was processed as a zhang ledger.
#[test]
fn a_ledger_without_files_is_processed_in_the_format_it_is_given() {
    let directives = Beancount::default().transform(TIMED_BALANCE.to_owned(), None).unwrap();
    let ledger = Ledger::process(LedgerProcessContext {
        directives,
        entry: (PathBuf::from("/"), "".to_owned()),
        dialect: Dialect::Beancount,
        visited_files: vec![],
        data_source: Arc::new(LocalFileSystemDataSource::new(Beancount::default())),
        clock: Clock::System,
    })
    .unwrap();

    assert_eq!(ledger.dialect, Dialect::Beancount);
    assert_eq!(errors(&ledger), vec![ErrorKind::BalanceTimeIgnored]);
}

/// Every file of a ledger is read in the format of its main file, whatever its own extension, and its balances are
/// checked by the rules of that format: a balance in a `.zhang` file of a beancount ledger is beancount's, one in a
/// `.bean` file of a zhang ledger is zhang's. The rule went by the extension of the file of the balance.
#[test]
fn a_balance_is_checked_by_the_format_of_its_ledger_not_of_its_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.bean"), "include \"checks.zhang\"\n").unwrap();
    std::fs::write(dir.path().join("checks.zhang"), TIMED_BALANCE).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    let beancount = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.bean".to_owned(), source).unwrap();
    assert_eq!(beancount.dialect, Dialect::Beancount);
    assert_eq!(errors(&beancount), vec![ErrorKind::BalanceTimeIgnored]);

    // in a zhang ledger, the `time` metadata of a balance is zhang's to read, and there is no beancount notice
    std::fs::write(dir.path().join("main.zhang"), "include \"checks.bean\"\n").unwrap();
    std::fs::write(dir.path().join("checks.bean"), TIMED_BALANCE).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let zhang = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
    assert_eq!(zhang.dialect, Dialect::Zhang);
    assert!(!errors(&zhang).contains(&ErrorKind::BalanceTimeIgnored), "{:?}", errors(&zhang));
}
