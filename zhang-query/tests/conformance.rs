//! Conformance harness: runs the beanquery-generated fixtures in `tests/conformance/cases`
//! against the engine on the shared fava demo ledger.
//!
//! The fixtures are produced by `tests/conformance/generate.py` from the official Python
//! beanquery (the oracle), so they are independent of this engine. The comparison rules are
//! those of `tests/conformance/README.md`:
//!
//! - columns are compared by position and type; names are advisory only, except in fixtures with
//!   `"strict_names": true` (pivoted results and `SELECT *` over a table), where they must match too;
//! - decimals (also inside amounts, positions and inventories) are compared numerically;
//! - rows are compared as a sequence when the fixture is `ordered`, otherwise as a multiset;
//! - inventories are compared as multisets of positions;
//! - an `expect: "error"` case passes only when the engine returns an error of the fixture's
//!   `error_class` (see [`error_class`]);
//! - an `expect: "csv"` case compares zhang's CSV export of the result (see [`engine_csv`])
//!   with beanquery's numberified CSV, cell by cell (see [`compare_csv`]).
//!
//! Every case gets one status:
//!
//! | status           | meaning                                                                         | fatal |
//! |------------------|---------------------------------------------------------------------------------|-------|
//! | `PASS`           | matches the oracle                                                              | no    |
//! | `ACCEPTED`       | differs from the oracle exactly as documented in [`ACCEPTED_DEVIATIONS`]        | no    |
//! | `LEDGER-DEP`     | a `ledger-dependent` case listed in [`LEDGER_DEPENDENT_ALLOWED`] that differs   | no    |
//! | `PENDING-PHASE2` | a `"phase": 2` fixture while [`PHASE2_FEATURES_LANDED`] is `false`              | no    |
//! | `PENDING-PHASE3` | a `"phase": 3` fixture while [`PHASE3_FEATURES_LANDED`] is `false`              | no    |
//! | `FAIL`           | anything else, including unlisted `ledger-dependent` cases and missing functions | yes   |
//!
//! Pending fixtures still run, and the detail column shows the status they would get
//! (`would PASS`, `would FAIL: ...`). Phase 1 fixtures (no `phase` field) are always strict.
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

/// Temporary gate for the Phase 2 fixtures (issue #434: BALANCES, JOURNAL, the running `balance`
/// column, `FROM … OPEN/CLOSE/CLEAR` and the CSV export). While it is `false`, every fixture with
/// `"phase": 2` is reported as `PENDING-PHASE2` and cannot fail the test. Flip it to `true` once
/// the Phase 2 features have landed, together with wiring [`engine_csv`]; then delete the gate.
const PHASE2_FEATURES_LANDED: bool = true;

/// Temporary gate for the Phase 3 fixtures (issue #434: `HAVING`, `PIVOT BY` and `FROM #table`
/// over beanquery's tables). While it is `false`, every fixture with `"phase": 3` is reported as
/// `PENDING-PHASE3` and cannot fail the test. Flip it to `true` once the Phase 3 features have
/// landed; then delete the gate.
const PHASE3_FEATURES_LANDED: bool = true;

/// Whether the fixtures of a phase are still pending (non-fatal), see the gates above.
fn phase_pending(phase: u64) -> bool {
    match phase {
        2 => !PHASE2_FEATURES_LANDED,
        3 => !PHASE3_FEATURES_LANDED,
        _ => false,
    }
}

/// zhang's CSV export of a result, compared with the `expect: "csv"` fixtures.
///
/// Phase 2 adds `zhang_query::export::to_csv(&QueryResult) -> String`. Until it lands there is
/// nothing to call, and every csv case reports the export as missing. On integration, replace
/// the body with `Some(zhang_query::export::to_csv(result))`.
fn engine_csv(result: &QueryResult) -> Option<String> {
    Some(zhang_query::export::to_csv(result))
}

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
        QueryErrorKind::TooLarge => "too_large",
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
    PendingPhase2,
    PendingPhase3,
    Fail,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Accepted => "ACCEPTED",
            Status::LedgerDep => "LEDGER-DEP",
            Status::PendingPhase2 => "PENDING-PHASE2",
            Status::PendingPhase3 => "PENDING-PHASE3",
            Status::Fail => "FAIL",
        }
    }
}

/// What a fixture expects (`expect` field).
enum Expect {
    /// `"rows"`: the `columns` and `rows` of the fixture
    Rows,
    /// `"error"`: an error of this `error_class`
    Error(String),
    /// `"csv"`: beanquery's numberified CSV output, one line per item (the `csv` field)
    Csv(Vec<String>),
}

struct Fixture {
    file: String,
    name: String,
    query: String,
    /// 1 for the Phase 1 fixtures (no `phase` field), otherwise the `phase` field (2 to 4); the
    /// Phase 4 fixtures (issue #479, wave 1: beanquery's date functions, intervals and account
    /// and commodity directive functions) are always strict, like Phase 1
    phase: u64,
    kind: String,
    ordered: bool,
    /// `"strict_names": true`: the column names must match too, not only the types
    strict_names: bool,
    expect: Expect,
    column_names: Vec<String>,
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
                phase: match json.get("phase").map(|phase| phase.as_u64()) {
                    None => 1,
                    Some(Some(phase @ (1..=4))) => phase,
                    Some(_) => panic!("{}: `phase` must be 1, 2, 3 or 4", file),
                },
                kind: string("kind"),
                ordered: field("ordered").as_bool().expect("`ordered` is a bool"),
                strict_names: json
                    .get("strict_names")
                    .map(|strict| strict.as_bool().unwrap_or_else(|| panic!("{}: `strict_names` is not a bool", file)))
                    .unwrap_or(false),
                expect: match string("expect").as_str() {
                    "rows" => Expect::Rows,
                    "error" => match string("error_class").as_str() {
                        class @ ("syntax" | "compile" | "runtime") => Expect::Error(class.to_owned()),
                        other => panic!("{}: unknown error_class `{}`", file, other),
                    },
                    "csv" => Expect::Csv(
                        array("csv")
                            .iter()
                            .map(|line| line.as_str().unwrap_or_else(|| panic!("{}: `csv` holds a non-string", file)).to_owned())
                            .collect(),
                    ),
                    other => panic!("{}: unknown expect `{}`", file, other),
                },
                column_names: array("columns")
                    .iter()
                    .map(|column| column["name"].as_str().expect("column name").to_owned())
                    .collect(),
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
        Value::Interval(it) => json!(it.to_string()),
        Value::Metas(it) => json!(it.iter().map(|(key, value)| json!({"key": key, "value": value})).collect::<Vec<_>>()),
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

/// `None` when the columns match, otherwise a one-line diff. Names are compared only for a
/// `strict_names` fixture, after the count and the types.
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
    if !mismatches.is_empty() {
        return Some(mismatches.join("; "));
    }
    let names = result.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>();
    (fixture.strict_names && names != fixture.column_names).then(|| format!("column names: expected {:?}, got {:?}", fixture.column_names, names))
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

// ---------------------------------------------------------------------------
// CSV (`expect: "csv"`, see the README's "CSV fixtures")
// ---------------------------------------------------------------------------

/// Parses CSV text: `,` separates fields, a field starting with `"` is quoted (`""` is a literal
/// quote inside it), and records end with CRLF or LF. A final line terminator is optional.
fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    // `field_started` distinguishes an empty field from no field at all (blank trailing text)
    let (mut quoted, mut field_started) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                field_started = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                field_started = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                field_started = false;
            }
            _ => {
                field.push(c);
                field_started = true;
            }
        }
    }
    if quoted {
        return Err("unterminated quoted field".to_owned());
    }
    if field_started || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

/// `-?digits[.digits]`: the cells compared numerically. Anything else (dates, `TRUE`, account
/// names, sets) is compared as text.
fn is_plain_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (integral, fractional) = digits.split_once('.').unwrap_or((digits, "0"));
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    all_digits(integral) && all_digits(fractional)
}

/// A CSV cell in canonical form: surrounding whitespace removed (beanquery pads numbers to align
/// them) and plain numbers normalised, so `"  4.00"` equals `"4"`.
fn canonical_csv_cell(cell: &str) -> String {
    let cell = cell.trim();
    if is_plain_number(cell) {
        to_plain_string(&BigDecimal::from_str(cell).expect("plain number").normalized())
    } else {
        cell.to_owned()
    }
}

/// `None` when zhang's CSV matches the fixture's, otherwise a short diff. The header row must
/// match cell by cell (names include numberify's ` (<currency>)` suffix), then the data rows are
/// compared like typed rows: as a sequence when `ordered`, otherwise as a multiset.
fn compare_csv(expected: &str, actual: &str, ordered: bool) -> Option<String> {
    let parse = |text: &str, side: &str| parse_csv(text).map_err(|err| format!("{} CSV: {}", side, err));
    let (expected, actual) = match (parse(expected, "fixture"), parse(actual, "engine")) {
        (Ok(expected), Ok(actual)) => (expected, actual),
        (Err(err), _) | (_, Err(err)) => return Some(err),
    };
    let canonical = |records: &[Vec<String>]| {
        records
            .iter()
            .map(|record| Json::Array(record.iter().map(|cell| Json::String(canonical_csv_cell(cell))).collect()).to_string())
            .collect::<Vec<_>>()
    };
    let (expected, actual) = (canonical(&expected), canonical(&actual));
    match (expected.split_first(), actual.split_first()) {
        (Some((expected_header, _)), None) => Some(format!("engine CSV is empty, expected header {}", short(expected_header))),
        (Some((expected_header, _)), Some((actual_header, _))) if expected_header != actual_header => {
            Some(format!("CSV header: expected {} got {}", short(expected_header), short(actual_header)))
        }
        (Some((_, expected_rows)), Some((_, actual_rows))) => compare_rows(expected_rows, actual_rows, ordered).map(|diff| format!("CSV {}", diff)),
        (None, _) => Some("fixture CSV is empty".to_owned()),
    }
}

fn run_case(ledger: &zhang_core::ledger::Ledger, fixture: &Fixture) -> Report {
    let outcome = Query::compile(&fixture.query).and_then(|query| query.execute_at(ledger, &Params::default(), today()));
    let deviation = ACCEPTED_DEVIATIONS.iter().find(|it| it.case == Some(fixture.name.as_str()));

    let mismatch = match (&outcome, &fixture.expect) {
        (Err(err), Expect::Error(class)) if error_class(err.kind) == class => None,
        (Err(err), Expect::Error(class)) => Some(format!("expected a {} error, got a {} error: {}", class, error_class(err.kind), err)),
        (Ok(result), Expect::Error(class)) => Some(format!("expected a {} error, the engine returned {} rows", class, result.rows.len())),
        (Err(err), Expect::Rows | Expect::Csv(_)) => Some(format!("engine error: {}", err)),
        (Ok(result), Expect::Rows) => compare_columns(fixture, result).or_else(|| {
            let expected = canonical_fixture_rows(fixture, &fixture.rows);
            compare_rows(&expected, &canonical_engine_rows(result), fixture.ordered)
        }),
        (Ok(result), Expect::Csv(lines)) => match engine_csv(result) {
            Some(csv) => compare_csv(&lines.join("\n"), &csv, fixture.ordered),
            None => Some("no CSV export to compare: engine_csv() is not wired to zhang_query::export::to_csv yet".to_owned()),
        },
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
    let (status, detail) = if phase_pending(fixture.phase) {
        let would = format!("would {}", status.label());
        let pending = if fixture.phase == 2 { Status::PendingPhase2 } else { Status::PendingPhase3 };
        (pending, if detail.is_empty() { would } else { format!("{}: {}", would, detail) })
    } else {
        (status, detail)
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
    for status in [
        Status::Pass,
        Status::Accepted,
        Status::LedgerDep,
        Status::PendingPhase2,
        Status::PendingPhase3,
        Status::Fail,
    ] {
        let count = reports.iter().filter(|report| report.status == status).count();
        out.push_str(&format!("{:<16} {}\n", status.label(), count));
    }
    out.push_str(&format!("{:<16} {}\n", "TOTAL", reports.len()));
    for (status, gate, phase) in [
        (Status::PendingPhase2, "PHASE2_FEATURES_LANDED", 2),
        (Status::PendingPhase3, "PHASE3_FEATURES_LANDED", 3),
    ] {
        let pending = reports.iter().filter(|report| report.status == status).collect::<Vec<_>>();
        if !pending.is_empty() {
            let would_fail = pending.iter().filter(|report| report.detail.starts_with("would FAIL")).count();
            out.push_str(&format!(
                "\n{} is false: {} phase {} case(s) pending, {} of them would FAIL\n",
                gate,
                pending.len(),
                phase,
                would_fail
            ));
        }
    }
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
