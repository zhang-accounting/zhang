//! Conformance harness: runs the beanquery-generated fixtures in `tests/conformance/cases`
//! against the engine on the shared fava demo ledger.
//!
//! The fixtures are produced by `tests/conformance/generate.py` from the official Python
//! beanquery (the oracle), so they are independent of this engine. The comparison rules are
//! those of `tests/conformance/README.md`:
//!
//! - columns are compared by position and type; names are advisory only;
//! - decimals (also inside amounts, positions and inventories) are compared numerically;
//! - rows are compared as a sequence when the fixture is `ordered`, otherwise as a multiset;
//! - inventories are compared as multisets of positions;
//! - an `expect: "error"` case passes only when the engine returns an error of the fixture's
//!   `error_class` (see [`error_class`]).
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

mod common;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_query::decimal::to_plain_string;
use zhang_query::{Amount, Params, Position, Query, QueryErrorKind, QueryResult, Value};

/// How a documented deviation from beanquery is checked.
enum Accepted {
    /// The engine must return exactly these rows (fixture cell encoding, JSON) instead of the
    /// oracle's rows. Columns are still compared against the fixture.
    Rows(&'static str),
    /// The fixture itself was generated to match zhang's behaviour, so it is compared as usual.
    InFixture,
    /// No fixture can exercise this, because beanquery cannot produce an expectation.
    NoFixture,
}

struct Deviation {
    /// fixture name (`name` field), or `None` when no fixture exercises the deviation
    case: Option<&'static str>,
    reason: &'static str,
    accepted: Accepted,
}

/// Deliberate differences between the engine and beanquery. Keep each entry justified.
const ACCEPTED_DEVIATIONS: &[Deviation] = &[
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
        case: None,
        reason: "x IN (a) with a one-element list works in zhang; beanquery parses (a) as a parenthesised scalar and \
                 crashes at runtime, so there is no oracle result",
        accepted: Accepted::NoFixture,
    },
];

/// A `ledger-dependent` case that may differ from beanquery because zhang processes the ledger
/// differently (booking, the price map, or data the Store does not keep, such as `@` prices,
/// cost dates, cost labels and posting metadata).
struct LedgerGap {
    case: &'static str,
    reason: &'static str,
}

/// The only cases allowed to differ for ledger-processing reasons, each with its reason.
/// A mismatch in any other case, `ledger-dependent` or not, is a `FAIL`. Empty today: every
/// ledger-dependent case matches the oracle.
const LEDGER_DEPENDENT_ALLOWED: &[LedgerGap] = &[];

/// The fixture `error_class` of an engine error: beanquery's `ParseError` is `syntax` and its
/// `CompilationError` (unknown column or function, type and grouping errors) is `compile`.
fn error_class(kind: QueryErrorKind) -> &'static str {
    match kind {
        QueryErrorKind::Parse => "syntax",
        QueryErrorKind::Compile => "compile",
        QueryErrorKind::Eval => "runtime",
        QueryErrorKind::Timeout => "timeout",
    }
}

/// Fixed `today()` for reproducible runs (no fixture uses `today()`).
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid date")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Pass,
    Accepted,
    LedgerDep,
    Fail,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Accepted => "ACCEPTED",
            Status::LedgerDep => "LEDGER-DEP",
            Status::Fail => "FAIL",
        }
    }
}

struct Fixture {
    file: String,
    name: String,
    query: String,
    kind: String,
    ordered: bool,
    /// `Some(class)` for an `expect: "error"` case
    expect_error: Option<String>,
    column_types: Vec<String>,
    rows: Vec<Vec<Json>>,
}

struct Report {
    file: String,
    kind: String,
    status: Status,
    detail: String,
}

fn cases_dir() -> PathBuf {
    match std::env::var_os("ZHANG_QUERY_CONFORMANCE_CASES") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/cases"),
    }
}

fn load_fixtures() -> Vec<Fixture> {
    let mut files = std::fs::read_dir(cases_dir())
        .unwrap_or_else(|err| panic!("cannot read {}: {}", cases_dir().display(), err))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {}", file, err));
            let json: Json = serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {}", file, err));
            let field = |key: &str| json.get(key).unwrap_or_else(|| panic!("{}: missing `{}`", file, key));
            let string = |key: &str| field(key).as_str().unwrap_or_else(|| panic!("{}: `{}` is not a string", file, key)).to_owned();
            let array = |key: &str| field(key).as_array().unwrap_or_else(|| panic!("{}: `{}` is not an array", file, key)).clone();
            Fixture {
                name: string("name"),
                query: string("query"),
                kind: string("kind"),
                ordered: field("ordered").as_bool().expect("`ordered` is a bool"),
                expect_error: match string("expect").as_str() {
                    "rows" => None,
                    "error" => match string("error_class").as_str() {
                        class @ ("syntax" | "compile" | "runtime") => Some(class.to_owned()),
                        other => panic!("{}: unknown error_class `{}`", file, other),
                    },
                    other => panic!("{}: unknown expect `{}`", file, other),
                },
                column_types: array("columns")
                    .iter()
                    .map(|column| column["type"].as_str().expect("column type").to_owned())
                    .collect(),
                rows: array("rows").into_iter().map(|row| row.as_array().expect("row is an array").clone()).collect(),
                file,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Canonical encoding: the fixture cell encoding with decimals normalised, so that equal
// numbers compare equal as JSON ("4.00" == "4") and inventories are order-independent.
// ---------------------------------------------------------------------------

fn canonical_decimal(number: &BigDecimal) -> Json {
    Json::String(to_plain_string(&number.normalized()))
}

fn canonical_decimal_str(text: &str) -> Json {
    canonical_decimal(&BigDecimal::from_str(text).unwrap_or_else(|err| panic!("bad decimal {:?}: {}", text, err)))
}

fn engine_amount(amount: &Amount) -> Json {
    json!({"number": canonical_decimal(&amount.number), "currency": amount.commodity})
}

fn engine_position(position: &Position) -> Json {
    let cost = position.cost.as_ref().map(|cost| {
        json!({
            "number": canonical_decimal(&cost.number),
            "currency": cost.currency,
            "date": cost.date.map(|date| date.format("%Y-%m-%d").to_string()),
            "label": cost.label,
        })
    });
    json!({"units": engine_amount(&position.units), "cost": cost})
}

fn sorted_positions(mut positions: Vec<Json>) -> Json {
    positions.sort_by_cached_key(|position| position.to_string());
    json!({ "positions": positions })
}

/// An engine value in canonical fixture encoding.
fn engine_cell(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(it) => json!(it),
        Value::Int(it) => json!(it),
        Value::Decimal(it) => canonical_decimal(it),
        Value::Str(it) => json!(it),
        Value::Date(it) => json!(it.format("%Y-%m-%d").to_string()),
        Value::Set(it) => json!(it.iter().collect::<Vec<_>>()),
        Value::Amount(it) => engine_amount(it),
        Value::Position(it) => engine_position(it),
        Value::Inventory(it) => sorted_positions(it.positions().map(|position| engine_position(&position)).collect()),
    }
}

fn fixture_amount(cell: &Json) -> Json {
    json!({"number": canonical_decimal_str(cell["number"].as_str().expect("amount number")), "currency": cell["currency"]})
}

fn fixture_position(cell: &Json) -> Json {
    let cost = match &cell["cost"] {
        Json::Null => Json::Null,
        cost => json!({
            "number": canonical_decimal_str(cost["number"].as_str().expect("cost number")),
            "currency": cost["currency"],
            "date": cost["date"],
            "label": cost["label"],
        }),
    };
    json!({"units": fixture_amount(&cell["units"]), "cost": cost})
}

/// A fixture cell in canonical encoding, interpreted with the fixture column type.
fn fixture_cell(cell: &Json, ty: &str) -> Json {
    if cell.is_null() {
        return Json::Null;
    }
    match ty {
        "decimal" => canonical_decimal_str(cell.as_str().expect("decimal cell")),
        "amount" => fixture_amount(cell),
        "position" => fixture_position(cell),
        "inventory" => sorted_positions(cell["positions"].as_array().expect("positions").iter().map(fixture_position).collect()),
        "set" => {
            let items = cell.as_array().expect("set cell").iter().map(|it| it.as_str().expect("set item").to_owned());
            json!(items.collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>())
        }
        _ => cell.clone(),
    }
}

fn canonical_fixture_rows(fixture: &Fixture, rows: &[Vec<Json>]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let cells = row
                .iter()
                .enumerate()
                .map(|(index, cell)| fixture_cell(cell, fixture.column_types.get(index).map(String::as_str).unwrap_or("")))
                .collect::<Vec<_>>();
            Json::Array(cells).to_string()
        })
        .collect()
}

fn canonical_engine_rows(result: &QueryResult) -> Vec<String> {
    result
        .rows
        .iter()
        .map(|row| Json::Array(row.iter().map(engine_cell).collect()).to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

fn short(text: &str) -> String {
    const MAX: usize = 220;
    if text.chars().count() <= MAX {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    }
}

/// `None` when the columns match, otherwise a one-line diff.
fn compare_columns(fixture: &Fixture, result: &QueryResult) -> Option<String> {
    let actual = result.columns.iter().map(|column| column.ty.name()).collect::<Vec<_>>();
    if actual.len() != fixture.column_types.len() {
        let names = result.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>();
        return Some(format!(
            "columns: expected {} {:?}, got {} {:?} named {:?}",
            fixture.column_types.len(),
            fixture.column_types,
            actual.len(),
            actual,
            names
        ));
    }
    let mismatches = fixture
        .column_types
        .iter()
        .zip(&actual)
        .enumerate()
        .filter(|(_, (expected, actual))| expected.as_str() != **actual)
        .map(|(index, (expected, actual))| format!("column {} `{}`: expected {}, got {}", index + 1, result.columns[index].name, expected, actual))
        .collect::<Vec<_>>();
    (!mismatches.is_empty()).then(|| mismatches.join("; "))
}

/// `None` when the rows match, otherwise a short diff.
fn compare_rows(expected: &[String], actual: &[String], ordered: bool) -> Option<String> {
    if ordered {
        if expected == actual {
            return None;
        }
        if let Some(index) = expected.iter().zip(actual).position(|(e, a)| e != a) {
            return Some(format!(
                "{} vs {} rows; first difference at row {}: expected {} got {}",
                expected.len(),
                actual.len(),
                index + 1,
                short(&expected[index]),
                short(&actual[index])
            ));
        }
        return Some(format!("expected {} rows, got {} (common prefix matches)", expected.len(), actual.len()));
    }
    let mut missing = expected.to_vec();
    let mut unexpected = Vec::new();
    for row in actual {
        match missing.iter().position(|it| it == row) {
            Some(index) => {
                missing.swap_remove(index);
            }
            None => unexpected.push(row.clone()),
        }
    }
    if missing.is_empty() && unexpected.is_empty() {
        return None;
    }
    missing.sort();
    unexpected.sort();
    let sample = |rows: &[String]| rows.iter().take(2).map(|row| short(row)).collect::<Vec<_>>().join(" | ");
    Some(format!(
        "{} vs {} rows (multiset); {} missing [{}]; {} unexpected [{}]",
        expected.len(),
        actual.len(),
        missing.len(),
        sample(&missing),
        unexpected.len(),
        sample(&unexpected)
    ))
}

fn run_case(ledger: &zhang_core::ledger::Ledger, fixture: &Fixture) -> Report {
    let outcome = Query::compile(&fixture.query).and_then(|query| query.execute_at(ledger, &Params::default(), today()));
    let deviation = ACCEPTED_DEVIATIONS.iter().find(|it| it.case == Some(fixture.name.as_str()));

    let mismatch = match (&outcome, &fixture.expect_error) {
        (Err(err), Some(class)) if error_class(err.kind) == class => None,
        (Err(err), Some(class)) => Some(format!("expected a {} error, got a {} error: {}", class, error_class(err.kind), err)),
        (Ok(result), Some(class)) => Some(format!("expected a {} error, the engine returned {} rows", class, result.rows.len())),
        (Err(err), None) => Some(format!("engine error: {}", err)),
        (Ok(result), None) => compare_columns(fixture, result).or_else(|| {
            let expected = canonical_fixture_rows(fixture, &fixture.rows);
            compare_rows(&expected, &canonical_engine_rows(result), fixture.ordered)
        }),
    };

    let (status, detail) = match mismatch {
        None if LEDGER_DEPENDENT_ALLOWED.iter().any(|it| it.case == fixture.name) => (
            Status::Pass,
            "matches beanquery although LEDGER_DEPENDENT_ALLOWED lists it; remove the entry".to_owned(),
        ),
        None => match deviation {
            Some(Deviation {
                accepted: Accepted::Rows(_), ..
            }) => (
                Status::Pass,
                "matches beanquery although an accepted deviation is listed; review the entry".to_owned(),
            ),
            Some(Deviation {
                accepted: Accepted::InFixture,
                reason,
                ..
            }) => (Status::Pass, format!("deviation encoded in the fixture: {}", reason)),
            _ => (Status::Pass, String::new()),
        },
        Some(diff) => {
            let ledger_gap = LEDGER_DEPENDENT_ALLOWED.iter().find(|it| it.case == fixture.name);
            match (deviation, &outcome) {
                (
                    Some(Deviation {
                        accepted: Accepted::Rows(rows),
                        reason,
                        ..
                    }),
                    Ok(result),
                ) => {
                    let accepted = serde_json::from_str::<Vec<Vec<Json>>>(rows).expect("accepted rows are JSON");
                    let expected = canonical_fixture_rows(fixture, &accepted);
                    match compare_columns(fixture, result).or_else(|| compare_rows(&expected, &canonical_engine_rows(result), fixture.ordered)) {
                        None => (Status::Accepted, (*reason).to_owned()),
                        Some(diff) => (Status::Fail, format!("differs from the accepted deviation: {}", diff)),
                    }
                }
                _ => match ledger_gap {
                    Some(gap) => (Status::LedgerDep, format!("{} ({})", diff, gap.reason)),
                    None => (Status::Fail, diff),
                },
            }
        }
    };
    Report {
        file: fixture.file.clone(),
        kind: fixture.kind.clone(),
        status,
        detail,
    }
}

#[test]
fn beanquery_conformance() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures found in {}", cases_dir().display());

    for deviation in ACCEPTED_DEVIATIONS {
        if let Some(case) = deviation.case {
            assert!(
                fixtures.iter().any(|fixture| fixture.name == case),
                "ACCEPTED_DEVIATIONS refers to unknown case `{}`",
                case
            );
        }
    }
    for gap in LEDGER_DEPENDENT_ALLOWED {
        assert!(
            fixtures.iter().any(|fixture| fixture.name == gap.case && fixture.kind == "ledger-dependent"),
            "LEDGER_DEPENDENT_ALLOWED refers to `{}`, which is not a ledger-dependent case",
            gap.case
        );
    }

    let ledger = common::fava_demo_ledger();
    let reports = fixtures.iter().map(|fixture| run_case(&ledger, fixture)).collect::<Vec<_>>();

    let mut out = String::from("\nbeanquery conformance (zhang-query/tests/conformance)\n\n");
    for report in &reports {
        out.push_str(&format!(
            "{:<16} {:<50} {:<16} {}\n",
            report.status.label(),
            report.file,
            report.kind,
            short(&report.detail)
        ));
    }
    out.push('\n');
    for status in [Status::Pass, Status::Accepted, Status::LedgerDep, Status::Fail] {
        let count = reports.iter().filter(|report| report.status == status).count();
        out.push_str(&format!("{:<16} {}\n", status.label(), count));
    }
    out.push_str(&format!("{:<16} {}\n", "TOTAL", reports.len()));
    let undocumented = ACCEPTED_DEVIATIONS.iter().filter(|it| matches!(it.accepted, Accepted::NoFixture));
    for deviation in undocumented {
        out.push_str(&format!("\naccepted deviation without a fixture: {}\n", deviation.reason));
    }
    // Written to stderr directly so the table shows even when the test harness captures output.
    let _ = std::io::stderr().write_all(out.as_bytes());

    let failures = reports.iter().filter(|report| report.status == Status::Fail).collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "{} conformance case(s) failed:\n{}",
        failures.len(),
        failures
            .iter()
            .map(|report| format!("  {}: {}", report.file, report.detail))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
