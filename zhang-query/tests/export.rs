//! CSV export oracle: zhang's numberified CSV against `bean-query -f csv -m` on the shared
//! fava demo ledger.
//!
//! The fixtures in `tests/export/cases` hold beanquery's raw CSV output, produced by
//! `tests/export/generate.py`. Each case runs the same query through the engine and
//! [`zhang_query::export::to_csv`], parses both CSV texts and compares them:
//!
//! - the header names must be identical (every query aliases its columns), and so must the
//!   number of rows and their order (every query is fully ordered);
//! - cells of `decimal` and `int` columns are compared numerically after trimming
//!   beanquery's alignment padding, so `" 600.00"` equals `600.00`;
//! - every other cell must be identical, byte for byte.
//!
//! Documented differences, counted per case in the printed table:
//!
//! - **padding**: beanquery renders CSV with its text-table renderers, so numbers are
//!   padded to align on the decimal point. zhang never pads.
//! - **quantized**: beanquery's command line rounds numberified numbers to the ledger's
//!   display precision of each currency. zhang keeps them exact, so `cost(4.088 RGAGX
//!   {88.07 USD})` is `360.03016` in zhang and `360.03` in beanquery. Such a cell is
//!   accepted only when zhang's exact number, rounded half-even to the oracle's scale, equals
//!   the oracle's number.
//! - **scale**: the numbers are equal but are written with a different number of trailing
//!   zeros (`-54500` vs `-54500.00`). zhang writes the scale its engine computed; the cell
//!   is numerically exact.

use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::{BigDecimal, RoundingMode};
use chrono::NaiveDate;
use serde_json::Value as Json;
use zhang_query::export::{numberify, NumberifiedResult};
use zhang_query::{DataType, Params, Query};

struct Fixture {
    file: String,
    query: String,
    csv: String,
}

fn load_fixtures() -> Vec<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/export/cases");
    let mut files = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {}", dir.display(), err))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let json: Json = serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture")).expect("fixture is JSON");
            Fixture {
                file: path.file_name().unwrap().to_string_lossy().into_owned(),
                query: json["query"].as_str().expect("query").to_owned(),
                csv: json["csv"].as_str().expect("csv").to_owned(),
            }
        })
        .collect()
}

/// Parse RFC 4180 CSV with CRLF record separators into records of fields. A blank line is
/// a record with no fields (a header or row without columns); `""` is one empty field.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    // whether the current record has any content, quotes included
    let mut touched = false;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(ch),
            }
            continue;
        }
        match ch {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                if touched {
                    record.push(std::mem::take(&mut field));
                }
                records.push(std::mem::take(&mut record));
                touched = false;
                continue;
            }
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            _ => field.push(ch),
        }
        touched = true;
    }
    assert!(!quoted, "unterminated quoted field");
    assert!(!touched, "the last record must end with CRLF");
    records
}

#[derive(Default)]
struct Tally {
    cells: usize,
    padded: usize,
    quantized: usize,
    scale: usize,
    mismatches: Vec<String>,
}

fn compare(fixture: &Fixture, numberified: &NumberifiedResult, csv: &str) -> Tally {
    let mut tally = Tally::default();
    let expected = parse_csv(&fixture.csv);
    let actual = parse_csv(csv);
    let (expected_header, expected_rows) = expected.split_first().expect("oracle header");
    let (actual_header, actual_rows) = actual.split_first().expect("zhang header");
    if expected_header != actual_header {
        tally
            .mismatches
            .push(format!("header: expected {:?}, got {:?}", expected_header, actual_header));
        return tally;
    }
    if expected_rows.len() != actual_rows.len() {
        tally
            .mismatches
            .push(format!("expected {} rows, got {}", expected_rows.len(), actual_rows.len()));
        return tally;
    }
    for (row, (expected_row, actual_row)) in expected_rows.iter().zip(actual_rows).enumerate() {
        assert_eq!(actual_row.len(), numberified.columns.len(), "zhang row width");
        if expected_row.len() != actual_row.len() {
            tally
                .mismatches
                .push(format!("row {}: expected {} cells, got {}", row, expected_row.len(), actual_row.len()));
            continue;
        }
        for ((column, expected), actual) in numberified.columns.iter().zip(expected_row).zip(actual_row) {
            tally.cells += 1;
            let numeric = matches!(column.ty, DataType::Decimal | DataType::Int);
            let trimmed = expected.trim();
            if numeric && trimmed != expected {
                tally.padded += 1;
            }
            let ok = if !numeric || trimmed.is_empty() || actual.is_empty() {
                actual == if numeric { trimmed } else { expected.as_str() }
            } else {
                let oracle = BigDecimal::from_str(trimmed).expect("oracle number");
                let zhang = BigDecimal::from_str(actual).expect("zhang number");
                if oracle == zhang {
                    if trimmed != actual {
                        tally.scale += 1;
                    }
                    true
                } else if zhang.with_scale_round(oracle.fractional_digit_count(), RoundingMode::HalfEven) == oracle {
                    tally.quantized += 1;
                    true
                } else {
                    false
                }
            };
            if !ok {
                tally
                    .mismatches
                    .push(format!("row {} `{}`: expected {:?}, got {:?}", row, column.name, expected, actual));
            }
        }
    }
    tally
}

#[test]
fn csv_export_matches_beanquery() {
    let fixtures = load_fixtures();
    assert!(fixtures.len() >= 4, "expected the oracle fixtures in tests/export/cases");
    let ledger = zhang_testkit::ledger::fava_demo_ledger();
    let today = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let mut table = String::from("\ncsv export vs bean-query -f csv -m (zhang-query/tests/export)\n\n");
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let result = Query::compile(&fixture.query)
            .and_then(|query| query.execute_at(&ledger, &Params::default(), today))
            .unwrap_or_else(|err| panic!("{}: {}", fixture.file, err));
        let numberified = numberify(&result);
        let csv = numberified.to_csv();
        assert_eq!(csv, zhang_query::export::to_csv(&result));
        let tally = compare(fixture, &numberified, &csv);
        table.push_str(&format!(
            "{:<6} {:<48} {:>4} rows {:>5} cells {:>4} padded {:>3} quantized {:>3} scale\n",
            if tally.mismatches.is_empty() { "PASS" } else { "FAIL" },
            fixture.file,
            result.rows.len(),
            tally.cells,
            tally.padded,
            tally.quantized,
            tally.scale
        ));
        for mismatch in tally.mismatches.iter().take(5) {
            table.push_str(&format!("         {}\n", mismatch));
        }
        if !tally.mismatches.is_empty() {
            failures.push(fixture.file.clone());
        }
    }
    eprintln!("{}", table);
    assert!(failures.is_empty(), "csv export differs from beanquery in {:?}", failures);
}

#[test]
fn the_csv_parser_round_trips_quoting() {
    let records = parse_csv("a,\"b,c\",\"d\"\"e\"\r\n\"\"\r\n\r\n\"x\r\ny\",\r\n");
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
