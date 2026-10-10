//! Conformance harness: runs the beanquery-generated fixtures in `tests/oracle/cases/conformance`
//! against the engine on the shared fava demo ledger.
//!
//! The fixtures are produced by `tests/oracle/generate.py` from the official Python
//! beanquery (the oracle), so they are independent of this engine. The comparison is the one
//! of `zhang_testkit::oracle`, with the rules of `tests/oracle/README.md`: columns by
//! position and type (names too in fixtures with `"strict_names": true`), decimals
//! numerically, rows as a sequence when the fixture is `ordered` and as a multiset otherwise,
//! inventories as multisets of positions, errors by class, and CSV cell by cell.
//!
//! Every case gets one status:
//!
//! | status       | meaning                                                                         | fatal |
//! |--------------|---------------------------------------------------------------------------------|-------|
//! | `PASS`       | matches the oracle                                                              | no    |
//! | `ACCEPTED`   | differs from the oracle exactly as documented in [`ACCEPTED_DEVIATIONS`]        | no    |
//! | `LEDGER-DEP` | a `ledger-dependent` case listed in [`LEDGER_DEPENDENT_ALLOWED`] that differs   | no    |
//! | `FAIL`       | anything else, including unlisted `ledger-dependent` cases and missing functions | yes   |
//!
//! The test prints the summary table to stderr (always visible, even without `--nocapture`)
//! and fails when at least one case is `FAIL`, so it serves as a regression gate.
//!
//! Set `ZHANG_QUERY_CONFORMANCE_CASES=<dir>` to run the harness over another fixture
//! directory (e.g. a mutated copy when checking that the gate catches regressions).

use std::io::Write;
use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_testkit::oracle::{
    assert_no_failures, canonical_csv_cell, check_lists, compare_csv, is_plain_number, load_case_files, parse_csv, run_cases, summary, Accepted, Case,
    Deviation, Expect, LedgerGap, Rules,
};

/// Deliberate differences between the engine and beanquery. Keep each entry justified.
const ACCEPTED_DEVIATIONS: &[Deviation] = &[
    Deviation {
        case: Some("aggregate_over_no_rows"),
        reason: "an aggregate without group keys returns one row over empty input (#647): count and numeric sum \
                 are zero, inventory sum is empty, and first/last/min/max are NULL; beanquery 0.2.0 returns no rows",
        accepted: Accepted::Rows(r#"[[0, "0"]]"#),
    },
    Deviation {
        case: Some("null_logic_beanquery_quirks"),
        reason: "standard three-valued logic: NOT NULL is NULL (beanquery: TRUE) and NULL AND FALSE is FALSE \
                 (beanquery: NULL, because its AND stops at the first NULL operand)",
        accepted: Accepted::Rows(r#"[[null, false]]"#),
    },
    Deviation {
        case: Some("not_vs_not_equal_on_null"),
        reason: "standard three-valued logic: NOT (payee = 'Hoogle') is NULL for a NULL payee, like payee != 'Hoogle' \
                 (beanquery: NOT NULL is TRUE)",
        accepted: Accepted::Rows(r#"[[false, false, false, 46], [true, null, null, 230]]"#),
    },
    Deviation {
        case: Some("select_star"),
        reason: "SELECT * expands to date, flag, payee, narration, account, position by user decision (beanquery 0.2.0 \
                 omits account); the fixture was generated from that explicit column list",
        accepted: Accepted::InFixture,
    },
    Deviation {
        case: Some("date_text_compared_with_a_date"),
        reason: "a string compared with a date is read as date(text) reads it (a zhang extension; beanquery 0.2.0 \
                 rejects the comparison); the fixture was generated from the query with each string wrapped in date()",
        accepted: Accepted::InFixture,
    },
    Deviation {
        case: None,
        reason: "x IN (a) with a one-element list works in zhang; beanquery parses (a) as a parenthesised scalar and \
                 crashes at runtime, so there is no oracle result",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: Some("interval_values"),
        reason: "interval() also accepts weeks, seven days each (a zhang extension, by lead decision on #479); \
                 beanquery's interval('2 weeks') is NULL",
        accepted: Accepted::Rows(
            r#"[["1 month", "1 year 1 month", "-1 year", "10 days", "1 day", "14 days", null, null, null, "11 months", "1 month 3 days"]]"#,
        ),
    },
    Deviation {
        case: Some("date_bin_on_boundaries"),
        reason: "a date exactly on a bin boundary starts that bin, also for strides of months and years (by lead \
                 decision on #479); beanquery puts it into the previous bin, e.g. date_bin('1 month', 2000-02-01, \
                 2000-01-01) is 2000-01-01 there and 2000-02-01 here",
        accepted: Accepted::Rows(r#"[["2000-02-01", "2000-03-01", "2015-07-01", "2016-01-01", "2015-01-01", "2014-12-01", "2014-11-01", "2015-01-12"]]"#),
    },
    Deviation {
        case: Some("date_bin_month_end_origin"),
        reason: "the bins of date_bin are origin + k strides, each computed from the origin, so bins from a month end \
                 stay on month ends (2015-01-31, 2015-02-28, 2015-03-31); beanquery adds each stride to the previous \
                 bin, so its bins drift (2015-03-28), and it puts a date on a boundary into the previous bin",
        accepted: Accepted::Rows(r#"[["2015-02-28", "2015-02-28", "2015-02-28", "2014-10-31", "2014-12-31", "2015-01-31", "2017-02-28", null]]"#),
    },
    Deviation {
        case: None,
        reason: "date functions given a NULL literal return NULL (NULL in, NULL out); beanquery types NULL apart and \
                 rejects date_add(NULL, 1) at compile time",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: None,
        reason: "date_bin with a zero stride, or a stride text interval() cannot read, is NULL; beanquery fails with \
                 ZeroDivisionError or AttributeError",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: None,
        reason: "interval - interval is an interval; beanquery declares the result a date (while computing an interval), \
                 and accepts interval - date, which then fails at run time (a compile error here)",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: None,
        reason: "intervals are equal when their months (a year is twelve) and days are: =, !=, IN lists, GROUP BY and \
                 DISTINCT all use that (by lead decision on #479); beanquery rejects = and != on intervals, has no IN \
                 list of them, and groups relativedeltas field by field (1 year - 1 month apart from 11 months). \
                 Ordering (<, ORDER BY, min, max, PIVOT BY) is a compile error; beanquery fails at run time",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: None,
        reason: "dates are those of beancount's calendar, years 1 to 9999: a date function or date arithmetic whose \
                 result falls outside is NULL (by lead decision on #479); beanquery raises an error",
        accepted: Accepted::NoFixture,
    },
    Deviation {
        case: None,
        reason: "open_meta(account) and commodity_meta(currency) are metas (key, value) lists of the directive's own \
                 metadata; beanquery's dicts also hold the filename and lineno zhang does not keep",
        accepted: Accepted::NoFixture,
    },
];

/// The only cases allowed to differ for ledger-processing reasons (booking, the price map, or
/// data the store does not keep, such as `@` prices, cost dates, cost labels and posting
/// metadata), each with its reason. A mismatch in any other case, `ledger-dependent` or not,
/// is a `FAIL`. Empty today: every ledger-dependent case matches the oracle.
const LEDGER_DEPENDENT_ALLOWED: &[LedgerGap] = &[];

/// Fixed `today()` for reproducible runs (no fixture uses `today()`).
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid date")
}

fn cases_dir() -> PathBuf {
    match std::env::var_os("ZHANG_QUERY_CONFORMANCE_CASES") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/conformance"),
    }
}

fn load_fixtures() -> Vec<Case> {
    load_case_files(&cases_dir())
}

#[test]
fn beanquery_conformance() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures found in {}", cases_dir().display());
    check_lists(&fixtures, ACCEPTED_DEVIATIONS, LEDGER_DEPENDENT_ALLOWED);

    let ledger = zhang_testkit::fixtures::fava_demo();
    let reports = run_cases(&ledger, &fixtures, &Rules::on(today()), ACCEPTED_DEVIATIONS, LEDGER_DEPENDENT_ALLOWED);
    let out = summary(
        "beanquery conformance (zhang-query/tests/oracle/cases/conformance)",
        &reports,
        ACCEPTED_DEVIATIONS,
    );
    // Written to stderr directly so the table shows even when the test harness captures output.
    let _ = std::io::stderr().write_all(out.as_bytes());
    assert_no_failures("conformance", &reports);
}

/// The CSV comparison rules, checked on the csv fixtures themselves (independent of the engine):
/// every fixture parses into records of one width, an equivalent rewrite (cells trimmed, numbers
/// normalised, every cell quoted, LF line ends) compares equal, and a changed number does not.
#[test]
fn csv_comparison_rules() {
    let quote = |cell: &str| format!("\"{}\"", cell.replace('"', "\"\""));
    let render = |records: &[Vec<String>]| {
        records
            .iter()
            .map(|record| record.iter().map(|cell| quote(cell)).collect::<Vec<_>>().join(","))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut checked = 0;
    for fixture in load_fixtures() {
        let Expect::Csv(lines) = &fixture.expect else { continue };
        let text = lines.join("\r\n") + "\r\n";
        let records = parse_csv(&text).unwrap_or_else(|err| panic!("{}: {}", fixture.file, err));
        assert_eq!(records.len(), lines.len(), "{}: one record per line", fixture.file);
        assert!(
            records.iter().all(|record| record.len() == records[0].len()),
            "{}: records of different widths",
            fixture.file
        );
        assert_eq!(compare_csv(&text, &text, fixture.ordered), None, "{}: equal to itself", fixture.file);

        let rewritten = records
            .iter()
            .map(|record| record.iter().map(|cell| canonical_csv_cell(cell)).collect())
            .collect::<Vec<Vec<String>>>();
        assert_eq!(
            compare_csv(&text, &render(&rewritten), fixture.ordered),
            None,
            "{}: equivalent rewrite",
            fixture.file
        );

        let mut changed = rewritten.clone();
        let cell = changed
            .iter_mut()
            .skip(1)
            .flat_map(|record| record.iter_mut())
            .find(|cell| is_plain_number(cell))
            .unwrap_or_else(|| panic!("{}: no number to change", fixture.file));
        cell.push('1');
        assert!(
            compare_csv(&text, &render(&changed), fixture.ordered).is_some(),
            "{}: a changed number must differ",
            fixture.file
        );

        let mut renamed = rewritten.clone();
        renamed[0][0].push('x');
        assert!(
            compare_csv(&text, &render(&renamed), fixture.ordered).is_some(),
            "{}: a changed header must differ",
            fixture.file
        );
        checked += 1;
    }
    assert!(checked > 0, "no csv fixtures found in {}", cases_dir().display());

    assert_eq!(
        parse_csv("a,\"b,c\",\"d\"\"e\"\r\n,,\n\"\"").unwrap(),
        vec![vec!["a", "b,c", "d\"e"], vec!["", "", ""], vec![""]]
    );
    assert!(parse_csv("\"open").is_err());
    assert!(["4", "-4.00", "0.5"].iter().all(|it| is_plain_number(it)));
    assert!(["", "-", "4.", ".5", "1e5", "2017-01-12", "TRUE", "NaN"].iter().all(|it| !is_plain_number(it)));
    assert_eq!(canonical_csv_cell("  -4.500 "), canonical_csv_cell("-4.5"));
}
