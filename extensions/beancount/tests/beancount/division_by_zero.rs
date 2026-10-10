//! A ledger with an amount that divides by zero, such as `1/0 CNY`, made the parser panic, so loading the ledger
//! crashed. Loading it reports a parse error at the divisor, in either format. Beancount 3.2.3 crashes on it.

use std::sync::Arc;

use beancount::Beancount;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;

/// A ledger whose transaction divides by zero on line 6, at column 18.
const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n\
                      1970-01-01 open Assets:Cash\n\
                      1970-01-01 open Expenses:Food\n\
                      \n\
                      2024-01-02 * \"Shop\" \"lunch\"\n  Assets:Cash  1/0 CNY\n  Expenses:Food\n";

/// The error of loading `LEDGER` as the `main` file of a ledger, read by `data_type`.
fn load_error<T: DataType<Carrier = String> + Send + Sync + 'static>(main: &str, data_type: T) -> String {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(main), LEDGER).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(data_type));
    match Ledger::load_with_data_source(dir.path().to_path_buf(), main.to_owned(), source) {
        Ok(_) => panic!("{main}: a ledger that divides by zero loads"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_ledger_dividing_by_zero_reports_where() {
    assert_eq!(
        load_error("main.zhang", ZhangDataType {}),
        "cannot parse main.zhang: failed to parse zhang file: division by zero at line 6, column 18"
    );
    assert_eq!(
        load_error("main.bean", Beancount::default()),
        "cannot parse main.bean: failed to parse beancount file: division by zero at line 6, column 18"
    );
}
