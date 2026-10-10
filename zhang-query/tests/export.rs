//! CSV export oracle: zhang's numberified CSV against `bean-query -f csv -m` on the shared
//! fava demo ledger.
//!
//! The fixtures in `tests/export/cases` hold beanquery's raw CSV output, produced by
//! `tests/conformance/generate.py --set export`. Each case runs the same query through the
//! engine and [`zhang_query::export::to_csv`]; `zhang_testkit::oracle` parses both CSV texts
//! and compares them with the conformance rules (`tests/conformance/README.md`): the header
//! names must be identical (every query aliases its columns), and so must the number of rows
//! and their order (every query is fully ordered); every cell is trimmed (beanquery pads
//! numbers to align them on the decimal point, zhang never pads) and plain numbers are
//! compared numerically, so `" 600.00"` equals `600.00` and `-54500` equals `-54500.00`;
//! every other cell must be identical.
//!
//! One rule is this set's own ([`CsvNumbers::RoundedToOracle`]): beanquery's command line
//! rounds numberified numbers to the ledger's display precision of each currency. zhang keeps
//! them exact, so `cost(4.088 RGAGX {88.07 USD})` is `360.03016` in zhang and `360.03` in
//! beanquery. Such a cell is accepted only when zhang's exact number, rounded half-even to the
//! oracle's scale, equals the oracle's number; the printed table counts them per case.

use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_query::export::numberify;
use zhang_testkit::oracle::{assert_no_failures, execute, load_csv_files, parse_csv, run_case, CsvNumbers, Rules};

#[test]
fn csv_export_matches_beanquery() {
    let fixtures = load_csv_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/export/cases"));
    assert!(fixtures.len() >= 4, "expected the oracle fixtures in tests/export/cases");
    let ledger = zhang_testkit::fixtures::fava_demo();
    let rules = Rules {
        today: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        csv_numbers: CsvNumbers::RoundedToOracle,
    };

    let mut table = String::from("\ncsv export vs bean-query -f csv -m (zhang-query/tests/export)\n\n");
    let mut reports = Vec::new();
    for fixture in &fixtures {
        let result = execute(&ledger, fixture, &rules).unwrap_or_else(|err| panic!("{}: {}", fixture.file, err));
        // the export is the numberified result written as CSV, one cell per numberified column
        let numberified = numberify(&result);
        let csv = numberified.to_csv();
        assert_eq!(csv, zhang_query::export::to_csv(&result));
        for record in parse_csv(&csv).expect("zhang writes well-formed CSV") {
            assert_eq!(record.len(), numberified.columns.len(), "zhang row width");
        }
        let report = run_case(&ledger, fixture, &rules, &[], &[]);
        table.push_str(&format!(
            "{:<6} {:<48} {:>4} rows  {}\n",
            report.status.label(),
            fixture.file,
            result.rows.len(),
            report.detail
        ));
        reports.push(report);
    }
    eprintln!("{}", table);
    assert_no_failures("export", &reports);
}

#[test]
fn the_csv_parser_round_trips_quoting() {
    let records = parse_csv("a,\"b,c\",\"d\"\"e\"\r\n\"\"\r\n\r\n\"x\r\ny\",\r\n").unwrap();
    assert_eq!(
        records,
        vec![
            vec!["a".to_owned(), "b,c".to_owned(), "d\"e".to_owned()],
            vec![String::new()],
            vec![],
            vec!["x\r\ny".to_owned(), String::new()]
        ]
    );
}
