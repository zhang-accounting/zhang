//! The beanquery oracle of the date and directive-metadata functions of issue #479 (track L):
//! `cases/server_features/oracle.json`, written by beanquery 0.2.0 over
//! `ledgers/server_features/main.zhang` (`generate.py --set server_features`), run through
//! `zhang_testkit::oracle`. Where the lead ruled that zhang deliberately differs from beanquery
//! (`date_bin` starts bin k at origin + k × stride, computed from the origin, and a date on a bin
//! start begins that bin; `interval` accepts weeks), the oracle keeps beanquery's rows, generate.py
//! marks the case `accepted_deviation`, and [`ACCEPTED_DEVIATIONS`] holds the rows zhang must return,
//! as in `conformance.rs`. The engine tests of these features are in
//! `tests/engine/server_features.rs`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_testkit::oracle::{assert_no_failures, load_text_rows, run_cases, Accepted, Case, Deviation, Rules};

fn oracle_ledger() -> &'static Ledger {
    static CELL: OnceLock<Ledger> = OnceLock::new();
    CELL.get_or_init(|| {
        zhang_testkit::ledger::load_ledger(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/ledgers/server_features"),
            "main.zhang",
        )
    })
}

/// `today()` of every execution: after every entry of the ledger.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
}

// =======================================================================================
// track L: the beanquery 0.2.0 functions, against the oracle

/// The oracle cases of `server_features/oracle/oracle.json`, read by `zhang_testkit::oracle`:
/// the cells beanquery wrote as text (`NULL`, `TRUE`, dates, ints) in the typed encoding, the
/// column types, and the generator's `accepted_deviation` mark.
fn oracle_cases() -> Vec<Case> {
    load_text_rows(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/server_features/oracle.json"))
}

/// A deliberate difference from beanquery, as the lead ruled it; the mechanism of
/// `ACCEPTED_DEVIATIONS` in `conformance.rs`. The oracle keeps beanquery's rows, and generate.py
/// marks the case with the same reason; zhang must return the accepted rows instead, written as
/// `Value::to_string()` writes them.
const DATE_BIN_BOUNDARY: &str = "zhang buckets correctly: a date exactly on a month or year bin boundary starts that bin \
                                 (date_bin('1 month', 2000-02-01, 2000-01-01) is 2000-02-01); beanquery 0.2.0 puts it into \
                                 the previous bin";
const DATE_BIN_FROM_ORIGIN: &str = "zhang starts bin k at origin + k x stride, computed from the origin, so month and year \
                                    bins do not drift (from 2020-01-31 monthly: 01-31, 02-29, 03-31, 04-30), and a date on a bin \
                                    start begins that bin; beanquery 0.2.0 adds the stride to the previous start (01-31, 02-29, \
                                    03-29, ...) and puts a date on a start into the previous bin";
const INTERVAL_WEEKS: &str = "zhang extension: interval('<n> week[s]') is 7 x n days; beanquery 0.2.0 returns NULL for weeks";

const ACCEPTED_DEVIATIONS: &[Deviation] = &[
    // With the origin on the first of a month, a monthly bin is the date's month, a quarterly bin
    // its calendar quarter and a yearly bin its year, also before the origin: the bins of
    // date_trunc('month'), date_trunc('quarter') and date_trunc('year'). beanquery differs on
    // the dates that are themselves a boundary: 2001-01-01, 2020-01-01, 2023-01-01, 2024-01-01
    // in every column, and 2020-03-01 in the monthly ones (beanquery: the previous bin).
    Deviation {
        case: Some("date_bin_month_boundaries"),
        reason: DATE_BIN_BOUNDARY,
        accepted: Accepted::TextRows(&[
            &["1969-12-31", "1969-12-01", "1969-12-01", "1969-10-01", "1969-01-01"],
            &["1999-12-31", "1999-12-01", "1999-12-01", "1999-10-01", "1999-01-01"],
            &["2000-01-01", "2000-01-01", "2000-01-01", "2000-01-01", "2000-01-01"],
            &["2000-02-29", "2000-02-01", "2000-02-01", "2000-01-01", "2000-01-01"],
            &["2001-01-01", "2001-01-01", "2001-01-01", "2001-01-01", "2001-01-01"],
            &["2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01"],
            &["2020-01-31", "2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01"],
            &["2020-02-29", "2020-02-01", "2020-02-01", "2020-01-01", "2020-01-01"],
            &["2020-03-01", "2020-03-01", "2020-03-01", "2020-01-01", "2020-01-01"],
            &["2020-12-31", "2020-12-01", "2020-12-01", "2020-10-01", "2020-01-01"],
            &["2021-01-03", "2021-01-01", "2021-01-01", "2021-01-01", "2021-01-01"],
            &["2021-02-28", "2021-02-01", "2021-02-01", "2021-01-01", "2021-01-01"],
            &["2021-03-31", "2021-03-01", "2021-03-01", "2021-01-01", "2021-01-01"],
            &["2023-01-01", "2023-01-01", "2023-01-01", "2023-01-01", "2023-01-01"],
            &["2024-01-01", "2024-01-01", "2024-01-01", "2024-01-01", "2024-01-01"],
            &["2024-02-29", "2024-02-01", "2024-02-01", "2024-01-01", "2024-01-01"],
            &["2024-05-31", "2024-05-01", "2024-05-01", "2024-04-01", "2024-01-01"],
            &["2024-12-30", "2024-12-01", "2024-12-01", "2024-10-01", "2024-01-01"],
            &["2024-12-31", "2024-12-01", "2024-12-01", "2024-10-01", "2024-01-01"],
        ]),
    },
    // The examples of the ruling, origin 2000-01-01: 2000-02-01 and 2000-03-01 start their
    // monthly bins (beanquery: 2000-01-01, 2000-02-01); the origin is its own bin; 2000-01-31 is
    // the last day of January's bin; with two-month bins 2000-03-01 starts one (beanquery:
    // 2000-01-01) while 2000-02-29 ends the first; 2001-01-01 starts a yearly bin (beanquery:
    // 2000-01-01) while 2000-12-31 ends the first; 1999-12-01, a boundary before the origin,
    // starts its bin (as in beanquery).
    Deviation {
        case: Some("date_bin_boundary_examples"),
        reason: DATE_BIN_BOUNDARY,
        accepted: Accepted::TextRows(&[&[
            "2000-02-01",
            "2000-03-01",
            "2000-01-01",
            "2000-01-01",
            "2000-03-01",
            "2000-01-01",
            "2001-01-01",
            "2000-01-01",
            "1999-12-01",
        ]]),
    },
    // Month-end origins (ruled): bin k starts at origin + k × stride, computed from the origin
    // and clamped to the month's last day, never from the previous start; a date belongs to the
    // last bin starting on or before it.
    // - '1 month' from 2020-01-31: every start is the last day of a month (Jan 31 + k months
    //   clamps to the month end), so a date that is its month's last day starts a bin, and any
    //   other date is in the bin of the previous month's last day: 2000-01-01 -> 1999-12-31,
    //   2020-02-29 -> 2020-02-29, 2020-03-01 -> 2020-02-29, 2021-02-28 -> 2021-02-28,
    //   2024-12-30 -> 2024-11-30. (beanquery drifts to 02-29, 03-29, ..., the 29th, then the
    //   28th.)
    // - '2 months' from 2020-01-31: the starts are the last days of the odd months (Jan, Mar,
    //   May, Jul, Sep, Nov): 1969-12-31 -> 1969-11-30, 2000-02-29 -> 2000-01-31,
    //   2021-03-31 -> 2021-03-31, 2024-05-31 -> 2024-05-31, 2024-12-31 -> 2024-11-30.
    // - '1 year' from 2020-02-29: the starts are Feb 29 in leap years and Feb 28 otherwise:
    //   2000-01-01 -> 1999-02-28, 2000-02-29 -> 2000-02-29 (2000 is a leap year),
    //   2020-01-31 -> 2019-02-28, 2021-02-28 -> 2021-02-28 (the 2021 bin starts on Feb 28),
    //   2024-01-01 -> 2023-02-28, 2024-02-29 -> 2024-02-29. (beanquery: 2021-02-28 ->
    //   2020-02-29, and from 2021 on its starts stay on Feb 28.)
    Deviation {
        case: Some("date_bin_month_end_origin"),
        reason: DATE_BIN_FROM_ORIGIN,
        accepted: Accepted::TextRows(&[
            &["1969-12-31", "1969-12-31", "1969-11-30", "1969-02-28"],
            &["1999-12-31", "1999-12-31", "1999-11-30", "1999-02-28"],
            &["2000-01-01", "1999-12-31", "1999-11-30", "1999-02-28"],
            &["2000-02-29", "2000-02-29", "2000-01-31", "2000-02-29"],
            &["2001-01-01", "2000-12-31", "2000-11-30", "2000-02-29"],
            &["2020-01-01", "2019-12-31", "2019-11-30", "2019-02-28"],
            &["2020-01-31", "2020-01-31", "2020-01-31", "2019-02-28"],
            &["2020-02-29", "2020-02-29", "2020-01-31", "2020-02-29"],
            &["2020-03-01", "2020-02-29", "2020-01-31", "2020-02-29"],
            &["2020-12-31", "2020-12-31", "2020-11-30", "2020-02-29"],
            &["2021-01-03", "2020-12-31", "2020-11-30", "2020-02-29"],
            &["2021-02-28", "2021-02-28", "2021-01-31", "2021-02-28"],
            &["2021-03-31", "2021-03-31", "2021-03-31", "2021-02-28"],
            &["2023-01-01", "2022-12-31", "2022-11-30", "2022-02-28"],
            &["2024-01-01", "2023-12-31", "2023-11-30", "2023-02-28"],
            &["2024-02-29", "2024-02-29", "2024-01-31", "2024-02-29"],
            &["2024-05-31", "2024-05-31", "2024-05-31", "2024-02-29"],
            &["2024-12-30", "2024-11-30", "2024-11-30", "2024-02-29"],
            &["2024-12-31", "2024-12-31", "2024-11-30", "2024-02-29"],
        ]),
    },
    // Single dates around month-end starts, by the same rule:
    // - monthly from 2020-01-31 (starts 01-31, 02-29, 03-31, 04-30, 05-31; before the origin
    //   2019-12-31, 11-30, 10-31, 09-30): 2020-03-30 -> 2020-02-29 (beanquery 2020-03-29),
    //   2020-03-31 -> 2020-03-31 (beanquery 2020-03-29), 2020-04-30 -> 2020-04-30 (beanquery
    //   2020-04-29), 2019-10-30 -> 2019-09-30, the day before the start 2019-10-31 (beanquery
    //   2019-10-30, a drifted start);
    // - two-monthly from 2020-01-31 (starts 01-31, 03-31): 2020-03-31 -> 2020-03-31 (beanquery
    //   2020-01-31), 2020-03-30 -> 2020-01-31;
    // - yearly from 2020-02-29 (starts 2019-02-28, 2020-02-29, 2021-02-28, 2022-02-28,
    //   2023-02-28, 2024-02-29): 2021-02-28 -> 2021-02-28 (beanquery 2020-02-29), 2021-02-27 ->
    //   2020-02-29, 2024-02-28 -> 2023-02-28, 2024-02-29 -> 2024-02-29 (beanquery 2024-02-28),
    //   2019-02-28 -> 2019-02-28;
    // - the interval form: 2020-05-31 -> 2020-05-31 (beanquery 2020-05-29).
    Deviation {
        case: Some("date_bin_month_end_examples"),
        reason: DATE_BIN_FROM_ORIGIN,
        accepted: Accepted::TextRows(&[&[
            "2020-02-29",
            "2020-03-31",
            "2020-04-30",
            "2019-09-30",
            "2020-03-31",
            "2020-01-31",
            "2021-02-28",
            "2020-02-29",
            "2023-02-28",
            "2024-02-29",
            "2019-02-28",
            "2020-05-31",
        ]]),
    },
    // date + 7 days, date - 14 days, date - 7 days, date + 21 days (beanquery: NULL), across
    // month, year and leap-day ends: 1969-12-31 + 7 = 1970-01-07, 2000-02-29 + 7 = 2000-03-07,
    // 2021-02-28 + 7 = 2021-03-07, 2024-12-30 + 7 = 2025-01-06.
    Deviation {
        case: Some("interval_weeks_are_seven_days"),
        reason: INTERVAL_WEEKS,
        accepted: Accepted::TextRows(&[
            &["1969-12-31", "1970-01-07", "1969-12-17", "1969-12-24", "1970-01-21"],
            &["1999-12-31", "2000-01-07", "1999-12-17", "1999-12-24", "2000-01-21"],
            &["2000-01-01", "2000-01-08", "1999-12-18", "1999-12-25", "2000-01-22"],
            &["2000-02-29", "2000-03-07", "2000-02-15", "2000-02-22", "2000-03-21"],
            &["2001-01-01", "2001-01-08", "2000-12-18", "2000-12-25", "2001-01-22"],
            &["2020-01-01", "2020-01-08", "2019-12-18", "2019-12-25", "2020-01-22"],
            &["2020-01-31", "2020-02-07", "2020-01-17", "2020-01-24", "2020-02-21"],
            &["2020-02-29", "2020-03-07", "2020-02-15", "2020-02-22", "2020-03-21"],
            &["2020-03-01", "2020-03-08", "2020-02-16", "2020-02-23", "2020-03-22"],
            &["2020-12-31", "2021-01-07", "2020-12-17", "2020-12-24", "2021-01-21"],
            &["2021-01-03", "2021-01-10", "2020-12-20", "2020-12-27", "2021-01-24"],
            &["2021-02-28", "2021-03-07", "2021-02-14", "2021-02-21", "2021-03-21"],
            &["2021-03-31", "2021-04-07", "2021-03-17", "2021-03-24", "2021-04-21"],
            &["2023-01-01", "2023-01-08", "2022-12-18", "2022-12-25", "2023-01-22"],
            &["2024-01-01", "2024-01-08", "2023-12-18", "2023-12-25", "2024-01-22"],
            &["2024-02-29", "2024-03-07", "2024-02-15", "2024-02-22", "2024-03-21"],
            &["2024-05-31", "2024-06-07", "2024-05-17", "2024-05-24", "2024-06-21"],
            &["2024-12-30", "2025-01-06", "2024-12-16", "2024-12-23", "2025-01-20"],
            &["2024-12-31", "2025-01-07", "2024-12-17", "2024-12-24", "2025-01-21"],
        ]),
    },
    // '1 week', '2 weeks', '-1 week', '+2 weeks', '1 weeks' and '2 week' parse (beanquery: NULL).
    // READING: weeks follow the grammar beanquery has for days, months and years (a signed
    // integer, whitespace, the unit with an optional plural s, case-sensitive), so '1 Week' and
    // '1week' are NULL like '1 Month' and '1day'.
    Deviation {
        case: Some("interval_week_parsing"),
        reason: INTERVAL_WEEKS,
        accepted: Accepted::TextRows(&[&["FALSE", "FALSE", "FALSE", "FALSE", "FALSE", "FALSE", "TRUE", "TRUE"]]),
    },
];

fn deviation(case: &str) -> Option<&'static Deviation> {
    ACCEPTED_DEVIATIONS.iter().find(|it| it.case == Some(case))
}

/// Run every oracle case of `area` with zhang over the same ledger, and compare the column
/// types with beanquery's and the rows (in order) with beanquery's, or with the accepted rows
/// of an [`ACCEPTED_DEVIATIONS`] entry. The accepted rows must still differ from beanquery's: an
/// entry beanquery agrees with is stale and fails its case.
fn check_oracle(area: &str) {
    let cases = oracle_cases().into_iter().filter(|case| case.area == area).collect::<Vec<_>>();
    assert!(!cases.is_empty(), "no oracle case of {}", area);
    assert_no_failures(area, &run_cases(oracle_ledger(), &cases, &Rules::on(today()), ACCEPTED_DEVIATIONS, &[]));
}

#[test]
fn l_oracle_date_trunc() {
    check_oracle("date_trunc");
}

#[test]
fn l_oracle_date_part() {
    check_oracle("date_part");
}

#[test]
fn l_oracle_date_add() {
    check_oracle("date_add");
}

#[test]
fn l_oracle_date_diff() {
    check_oracle("date_diff");
}

#[test]
fn l_oracle_date_from_year_month_day() {
    check_oracle("date_ymd");
}

#[test]
fn l_oracle_interval_and_date_arithmetic() {
    check_oracle("interval");
}

/// `interval('<n> week[s]')` is 7 × n days, a zhang extension (accepted deviation: beanquery
/// returns NULL).
#[test]
fn l_oracle_interval_weeks() {
    check_oracle("interval_weeks");
}

#[test]
fn l_oracle_date_bin() {
    check_oracle("date_bin");
}

/// Accepted deviations, as the lead ruled:
/// - bin k starts at origin + k × stride, computed from the origin itself, so month and year
///   bins from a month-end origin do not drift (from 2020-01-31, monthly bins start on 01-31,
///   02-29, 03-31, 04-30, ...; beanquery 0.2.0 adds the stride to the previous start: 01-31,
///   02-29, 03-29, ...);
/// - a date exactly on a bin start begins that bin (`date_bin('1 month', 2000-02-01,
///   2000-01-01)` is 2000-02-01); beanquery puts it into the previous bin, the kind of
///   off-by-one the report graph must not have.
///
/// The oracle keeps beanquery's rows and `ACCEPTED_DEVIATIONS` has zhang's.
#[test]
fn l_oracle_date_bin_month_boundaries() {
    check_oracle("date_bin_boundary");
}

#[test]
fn l_oracle_open_and_commodity_directive_functions() {
    check_oracle("directive_meta");
}

/// The oracle file is the one generate.py writes: every case has rows, and their widths
/// match the column lists. The cases generate.py marks as accepted deviations are exactly those
/// of [`ACCEPTED_DEVIATIONS`], with the same shape of rows.
#[test]
fn l_oracle_fixture_is_well_formed() {
    let cases = oracle_cases();
    assert!(cases.len() >= 20, "{} cases", cases.len());
    let mut names = BTreeSet::new();
    for case in &cases {
        assert!(names.insert(case.name.clone()), "duplicate case {}", case.name);
        assert!(!case.rows.is_empty(), "{} has no rows", case.name);
        for row in &case.rows {
            assert_eq!(row.len(), case.column_types.len(), "{}", case.name);
        }
        match (deviation(&case.name), &case.accepted_deviation) {
            (Some(deviation), Some(reason)) => {
                assert!(!reason.is_empty() && !deviation.reason.is_empty(), "{}: no reason", case.name);
                let Accepted::TextRows(rows) = deviation.accepted else {
                    panic!("{}: the accepted rows are written as text", case.name)
                };
                assert_eq!(rows.len(), case.rows.len(), "{}: accepted rows", case.name);
                for row in rows {
                    assert_eq!(row.len(), case.column_types.len(), "{}: accepted row {:?}", case.name, row);
                }
            }
            (None, None) => {}
            (Some(_), None) => panic!("{} is in ACCEPTED_DEVIATIONS but generate.py does not mark it", case.name),
            (None, Some(_)) => panic!("generate.py marks {} as an accepted deviation, ACCEPTED_DEVIATIONS lacks it", case.name),
        }
    }
    for deviation in ACCEPTED_DEVIATIONS {
        let case = deviation.case.expect("every deviation here names its case");
        assert!(names.contains(case), "ACCEPTED_DEVIATIONS refers to unknown case {}", case);
    }
}
