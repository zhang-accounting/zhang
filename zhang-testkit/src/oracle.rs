//! The beanquery oracle harness: how the case files the Python generator writes from beanquery are read, and how
//! the engine's results are compared with them.
//!
//! Every case set of `zhang-query/tests` runs through [`run_case`] with one set of rules, those of
//! `zhang-query/tests/conformance/README.md`:
//!
//! - columns are compared by position and type; the names are compared too only when the case is
//!   [`Case::strict_names`] (a set whose column names are data marks every case so after loading it);
//! - decimals, also inside amounts, positions and inventories, are compared numerically;
//! - rows are compared as a sequence when the case is [`Case::ordered`], otherwise as a multiset;
//! - inventories are compared as multisets of positions, sets as sets;
//! - an `expect: "error"` case passes only when the engine returns an error of the case's `error_class`;
//! - an `expect: "csv"` case compares zhang's CSV export of the result with the oracle's CSV cell by cell, every
//!   cell trimmed and plain numbers compared numerically ([`compare_csv`]); a set may additionally accept a number
//!   zhang keeps exact where beanquery's command line rounds it to the display precision
//!   ([`CsvNumbers::RoundedToOracle`]).
//!
//! A deliberate difference from beanquery is a [`Deviation`] the calling test lists; a `ledger-dependent` case that
//! differs for a ledger-processing reason is a [`LedgerGap`]. The harness holds no expectation of its own: what
//! zhang must return is in the case files and in the deviation lists of the tests.
//!
//! Five file formats are read into one [`Case`] model: [`load_case_files`] (one JSON file per case, the
//! conformance format), [`load_query_list`] (a JSON list of `{query, columns, rows}`), [`load_tables`] (the
//! `#table` oracle, with its `ledger` field and inventories written as lists of amounts), [`load_text_rows`] (cells
//! written as `Value::to_string()` writes them, column types only) and [`load_csv_files`] (one `bean-query -f csv`
//! output per file).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bigdecimal::{BigDecimal, RoundingMode};
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_core::ledger::Ledger;
use zhang_query::decimal::to_plain_string;
use zhang_query::{Amount, ExecuteOptions, Params, Position, Query, QueryError, QueryErrorKind, QueryResult, Value};

use crate::golden::read_json;

// ---------------------------------------------------------------------------------------------------------------
// The case model
// ---------------------------------------------------------------------------------------------------------------

/// What a case expects of the engine.
#[derive(Debug, Clone)]
pub enum Expect {
    /// the `columns` and `rows` of the case
    Rows,
    /// an error of this class: `syntax`, `compile` or `runtime`
    Error(String),
    /// beanquery's numberified CSV output, one line per record (the line terminators removed)
    Csv(Vec<String>),
}

/// One oracle case in the conformance model, whatever file format it was read from.
#[derive(Debug, Clone)]
pub struct Case {
    /// where it comes from: the file name, with `#<index>` for a case of a list file
    pub file: String,
    pub name: String,
    pub query: String,
    /// the generator's area (`date_bin`, `having`, ...) when the file records one, else empty
    pub area: String,
    /// the ledger the case runs on, as its file names it (`"extra"`); `None` is the set's ledger
    pub ledger: Option<String>,
    /// 1 for the Phase 1 cases (no `phase` field), otherwise the `phase` field (2 to 4)
    pub phase: u64,
    /// `engine`, `ledger-dependent`, or empty when the file has no `kind`
    pub kind: String,
    pub ordered: bool,
    /// the column names are compared too, not only the types
    pub strict_names: bool,
    pub expect: Expect,
    pub column_names: Vec<String>,
    pub column_types: Vec<String>,
    /// the rows in the conformance cell encoding (see the README)
    pub rows: Vec<Vec<Json>>,
    /// the reason the generator wrote when it marked the case an accepted deviation
    pub accepted_deviation: Option<String>,
    pub notes: String,
}

impl Case {
    fn new(file: String, name: String, query: String) -> Case {
        Case {
            file,
            name,
            query,
            area: String::new(),
            ledger: None,
            phase: 1,
            kind: String::new(),
            ordered: true,
            strict_names: false,
            expect: Expect::Rows,
            column_names: vec![],
            column_types: vec![],
            rows: vec![],
            accepted_deviation: None,
            notes: String::new(),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().expect("a file name").to_string_lossy().into_owned()
}

fn file_stem(path: &Path) -> String {
    path.file_stem().expect("a file name").to_string_lossy().into_owned()
}

/// The `*.json` files of `dir`, in name order.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn field<'a>(json: &'a Json, key: &str, file: &str) -> &'a Json {
    json.get(key).unwrap_or_else(|| panic!("{file}: missing `{key}`"))
}

fn string(json: &Json, key: &str, file: &str) -> String {
    field(json, key, file)
        .as_str()
        .unwrap_or_else(|| panic!("{file}: `{key}` is not a string"))
        .to_owned()
}

fn array<'a>(json: &'a Json, key: &str, file: &str) -> &'a [Json] {
    field(json, key, file).as_array().unwrap_or_else(|| panic!("{file}: `{key}` is not an array"))
}

fn optional_string(json: &Json, key: &str) -> String {
    json.get(key).and_then(Json::as_str).unwrap_or("").to_owned()
}

/// The `(name, type)` lists of a `columns` field of `[{"name": .., "type": ..}, ..]`.
fn columns(columns: &[Json], file: &str) -> (Vec<String>, Vec<String>) {
    let text = |column: &Json, key: &str| column[key].as_str().unwrap_or_else(|| panic!("{file}: a column has no `{key}`")).to_owned();
    (
        columns.iter().map(|column| text(column, "name")).collect(),
        columns.iter().map(|column| text(column, "type")).collect(),
    )
}

fn rows(rows: &[Json], file: &str) -> Vec<Vec<Json>> {
    rows.iter()
        .map(|row| row.as_array().unwrap_or_else(|| panic!("{file}: a row is not an array")).clone())
        .collect()
}

fn error_class_of(class: &str, file: &str) -> String {
    match class {
        "syntax" | "compile" | "runtime" => class.to_owned(),
        other => panic!("{file}: unknown error_class `{other}`"),
    }
}

/// The cases of a directory of conformance-format files (`NNN_<name>.json`, one case each, in name order).
pub fn load_case_files(dir: &Path) -> Vec<Case> {
    json_files(dir)
        .iter()
        .map(|path| {
            let file = file_name(path);
            let json = read_json(path);
            let mut case = Case::new(file.clone(), string(&json, "name", &file), string(&json, "query", &file));
            case.phase = match json.get("phase").map(Json::as_u64) {
                None => 1,
                Some(Some(phase @ 1..=4)) => phase,
                Some(_) => panic!("{file}: `phase` must be 1, 2, 3 or 4"),
            };
            case.kind = string(&json, "kind", &file);
            case.ordered = field(&json, "ordered", &file)
                .as_bool()
                .unwrap_or_else(|| panic!("{file}: `ordered` is not a bool"));
            case.strict_names = json
                .get("strict_names")
                .map(|strict| strict.as_bool().unwrap_or_else(|| panic!("{file}: `strict_names` is not a bool")))
                .unwrap_or(false);
            case.expect = match string(&json, "expect", &file).as_str() {
                "rows" => Expect::Rows,
                "error" => Expect::Error(error_class_of(&string(&json, "error_class", &file), &file)),
                "csv" => Expect::Csv(
                    array(&json, "csv", &file)
                        .iter()
                        .map(|line| line.as_str().unwrap_or_else(|| panic!("{file}: `csv` holds a non-string")).to_owned())
                        .collect(),
                ),
                other => panic!("{file}: unknown expect `{other}`"),
            };
            (case.column_names, case.column_types) = columns(array(&json, "columns", &file), &file);
            case.rows = rows(array(&json, "rows", &file), &file);
            case.notes = optional_string(&json, "notes");
            case
        })
        .collect()
}

/// The cases of a JSON list of `{"query", "columns", "rows"}` (the golden and statements files), each compared as
/// an ordered result. A case is named `<file stem>[<index>]`.
pub fn load_query_list(path: &Path) -> Vec<Case> {
    let (file, stem) = (file_name(path), file_stem(path));
    let json = read_json(path);
    json.as_array()
        .unwrap_or_else(|| panic!("{file}: not a list of cases"))
        .iter()
        .enumerate()
        .map(|(index, json)| {
            let mut case = Case::new(format!("{file}#{index}"), format!("{stem}[{index}]"), string(json, "query", &file));
            (case.column_names, case.column_types) = columns(array(json, "columns", &file), &file);
            case.rows = rows(array(json, "rows", &file), &file);
            case
        })
        .collect()
}

/// The cases of the `#table` oracle: `{"generator", "cases": [{"ledger", "query", "ordered", "columns", "rows"}]}`.
/// Its inventories are written as lists of amounts (no lot has a cost there); they are read as positions without
/// cost, so that an engine inventory holding a cost does not match them.
pub fn load_tables(path: &Path) -> Vec<Case> {
    let (file, stem) = (file_name(path), file_stem(path));
    let json = read_json(path);
    array(&json, "cases", &file)
        .iter()
        .enumerate()
        .map(|(index, json)| {
            let mut case = Case::new(format!("{file}#{index}"), format!("{stem}[{index}]"), string(json, "query", &file));
            case.ledger = Some(string(json, "ledger", &file));
            case.ordered = field(json, "ordered", &file)
                .as_bool()
                .unwrap_or_else(|| panic!("{file}: `ordered` is not a bool"));
            (case.column_names, case.column_types) = columns(array(json, "columns", &file), &file);
            case.rows = rows(array(json, "rows", &file), &file)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .zip(&case.column_types)
                        .map(|(cell, ty)| if ty == "inventory" { amount_list_as_inventory(cell) } else { cell })
                        .collect()
                })
                .collect();
            case
        })
        .collect()
}

fn amount_list_as_inventory(cell: Json) -> Json {
    match cell {
        Json::Array(amounts) => json!({ "positions": amounts.into_iter().map(|units| json!({"units": units, "cost": null})).collect::<Vec<_>>() }),
        other => other,
    }
}

/// The cases of an oracle whose cells are written as `Value::to_string()` writes them and whose `columns` are
/// type names only: `[{"area", "name", "query", "notes", "columns": ["date", ..], "rows": [["2024-01-01", ..]],
/// "accepted_deviation"?}]`. The rows are compared as a sequence; `NULL`, `TRUE` and `FALSE` and the ints are
/// read into the typed cell encoding ([`text_cell`]).
pub fn load_text_rows(path: &Path) -> Vec<Case> {
    let file = file_name(path);
    let json = read_json(path);
    json.as_array()
        .unwrap_or_else(|| panic!("{file}: not a list of cases"))
        .iter()
        .enumerate()
        .map(|(index, json)| {
            let mut case = Case::new(format!("{file}#{index}"), string(json, "name", &file), string(json, "query", &file));
            case.area = optional_string(json, "area");
            case.column_types = array(json, "columns", &file)
                .iter()
                .map(|ty| ty.as_str().unwrap_or_else(|| panic!("{file}: a column type is not a string")).to_owned())
                .collect();
            case.rows = array(json, "rows", &file)
                .iter()
                .map(|row| {
                    row.as_array()
                        .unwrap_or_else(|| panic!("{file}: a row is not an array"))
                        .iter()
                        .zip(&case.column_types)
                        .map(|(cell, ty)| text_cell(cell.as_str().unwrap_or_else(|| panic!("{file}: a cell is not text")), ty))
                        .collect()
                })
                .collect();
            case.accepted_deviation = json.get("accepted_deviation").map(|reason| {
                reason
                    .as_str()
                    .unwrap_or_else(|| panic!("{file}: `accepted_deviation` is not a string"))
                    .to_owned()
            });
            case.notes = optional_string(json, "notes");
            case
        })
        .collect()
}

/// A cell written as `Value::to_string()` writes it, in the JSON cell encoding of its column type: `NULL` is null
/// whatever the type, `TRUE`/`FALSE` are the bools, an int is a number, a decimal its digits, and dates and strings
/// stay text.
pub fn text_cell(text: &str, ty: &str) -> Json {
    match (ty, text) {
        (_, "NULL") => Json::Null,
        ("bool", "TRUE") => json!(true),
        ("bool", "FALSE") => json!(false),
        ("bool", other) => panic!("not a bool: {other:?}"),
        ("int", digits) => json!(digits.parse::<i64>().unwrap_or_else(|error| panic!("not an int: {digits:?}: {error}"))),
        _ => json!(text),
    }
}

/// The cases of a directory of `{"name", "query", "notes", "csv"}` files, `csv` being the whole output of
/// `bean-query -f csv -m` for the query; each is compared as an ordered CSV result.
pub fn load_csv_files(dir: &Path) -> Vec<Case> {
    json_files(dir)
        .iter()
        .map(|path| {
            let file = file_name(path);
            let json = read_json(path);
            let mut case = Case::new(file.clone(), string(&json, "name", &file), string(&json, "query", &file));
            case.expect = Expect::Csv(string(&json, "csv", &file).lines().map(str::to_owned).collect());
            case.notes = optional_string(&json, "notes");
            case
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Canonical encoding: the fixture cell encoding with decimals normalised, so that equal numbers compare equal as
// JSON ("4.00" == "4") and inventories are order-independent.
// ---------------------------------------------------------------------------------------------------------------

pub fn canonical_decimal(number: &BigDecimal) -> Json {
    Json::String(to_plain_string(&number.normalized()))
}

pub fn canonical_decimal_str(text: &str) -> Json {
    canonical_decimal(&BigDecimal::from_str(text).unwrap_or_else(|error| panic!("bad decimal {text:?}: {error}")))
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

/// An engine value in the canonical fixture encoding.
pub fn engine_cell(value: &Value) -> Json {
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

/// A fixture cell in the canonical encoding, interpreted with its column type.
pub fn fixture_cell(cell: &Json, ty: &str) -> Json {
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

/// Fixture rows in the canonical encoding, one JSON text per row.
pub fn canonical_fixture_rows(column_types: &[String], rows: &[Vec<Json>]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let cells = row
                .iter()
                .enumerate()
                .map(|(index, cell)| fixture_cell(cell, column_types.get(index).map(String::as_str).unwrap_or("")))
                .collect::<Vec<_>>();
            Json::Array(cells).to_string()
        })
        .collect()
}

/// Engine rows in the canonical encoding, one JSON text per row.
pub fn canonical_engine_rows(result: &QueryResult) -> Vec<String> {
    result
        .rows
        .iter()
        .map(|row| Json::Array(row.iter().map(engine_cell).collect()).to_string())
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------------------------------------------

fn short(text: &str) -> String {
    const MAX: usize = 220;
    if text.chars().count() <= MAX {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    }
}

/// The fixture `error_class` of an engine error: beanquery's `ParseError` is `syntax` and its `CompilationError`
/// (unknown column or function, type and grouping errors) is `compile`.
pub fn error_class(kind: QueryErrorKind) -> &'static str {
    match kind {
        QueryErrorKind::Parse => "syntax",
        QueryErrorKind::Compile => "compile",
        QueryErrorKind::Eval => "runtime",
        QueryErrorKind::Timeout => "timeout",
        QueryErrorKind::TooLarge => "too_large",
    }
}

/// `None` when the columns match, otherwise a one-line diff. The names are compared only for a `strict_names`
/// case, after the count and the types.
pub fn compare_columns(case: &Case, result: &QueryResult) -> Option<String> {
    let actual = result.columns.iter().map(|column| column.ty.name()).collect::<Vec<_>>();
    let names = result.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>();
    if actual.len() != case.column_types.len() {
        return Some(format!(
            "columns: expected {} {:?}, got {} {:?} named {:?}",
            case.column_types.len(),
            case.column_types,
            actual.len(),
            actual,
            names
        ));
    }
    let mismatches = case
        .column_types
        .iter()
        .zip(&actual)
        .enumerate()
        .filter(|(_, (expected, actual))| expected.as_str() != **actual)
        .map(|(index, (expected, actual))| format!("column {} `{}`: expected {}, got {}", index + 1, names[index], expected, actual))
        .collect::<Vec<_>>();
    if !mismatches.is_empty() {
        return Some(mismatches.join("; "));
    }
    (case.strict_names && names != case.column_names).then(|| format!("column names: expected {:?}, got {:?}", case.column_names, names))
}

/// `None` when the rows match, otherwise a short diff: as a sequence when `ordered`, else as a multiset.
pub fn compare_rows(expected: &[String], actual: &[String], ordered: bool) -> Option<String> {
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

// ---------------------------------------------------------------------------------------------------------------
// CSV (`expect: "csv"`, see the README's "CSV fixtures")
// ---------------------------------------------------------------------------------------------------------------

/// Parses CSV text: `,` separates fields, a field starting with `"` is quoted (`""` is a literal quote inside it),
/// and records end with CRLF or LF. A final line terminator is optional. A blank line is a record with no fields
/// (a header or row without columns); `""` is one empty field.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    // `field_started` distinguishes an empty field from no field at all (a blank line, or blank trailing text)
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
                if field_started || !record.is_empty() {
                    record.push(std::mem::take(&mut field));
                }
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

/// `-?digits[.digits]`: the cells compared numerically. Anything else (dates, `TRUE`, account names, sets) is
/// compared as text.
pub fn is_plain_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (integral, fractional) = digits.split_once('.').unwrap_or((digits, "0"));
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    all_digits(integral) && all_digits(fractional)
}

/// A CSV cell in canonical form: surrounding whitespace removed (beanquery pads numbers to align them) and plain
/// numbers normalised, so `"  4.00"` equals `"4"`.
pub fn canonical_csv_cell(cell: &str) -> String {
    let cell = cell.trim();
    if is_plain_number(cell) {
        to_plain_string(&BigDecimal::from_str(cell).expect("plain number").normalized())
    } else {
        cell.to_owned()
    }
}

/// How the numbers of a CSV result are compared with the oracle's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvNumbers {
    /// numerically, exact (the conformance rule; its generator rejects a case whose numbers beanquery rounds)
    Exact,
    /// numerically, and a number zhang keeps exact is also accepted when, rounded half-even to the scale the
    /// oracle wrote, it equals the oracle's: `bean-query`'s command line rounds numberified numbers to the
    /// ledger's display precision of each currency (`360.03016` is `360.03` there). Ordered results only.
    RoundedToOracle,
}

/// The outcome of a CSV comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvComparison {
    /// `None` when the CSVs match
    pub mismatch: Option<String>,
    /// the cells accepted by [`CsvNumbers::RoundedToOracle`] only
    pub rounded_cells: usize,
}

/// `None` when zhang's CSV matches the oracle's under the exact rule, otherwise a short diff. The header row must
/// match cell by cell (names include numberify's ` (<currency>)` suffix), then the data rows are compared like
/// typed rows: as a sequence when `ordered`, otherwise as a multiset.
pub fn compare_csv(expected: &str, actual: &str, ordered: bool) -> Option<String> {
    compare_csv_with(expected, actual, ordered, CsvNumbers::Exact).mismatch
}

/// [`compare_csv`] under the given number rule.
pub fn compare_csv_with(expected: &str, actual: &str, ordered: bool, numbers: CsvNumbers) -> CsvComparison {
    let parse = |text: &str, side: &str| parse_csv(text).map_err(|err| format!("{side} CSV: {err}"));
    let (expected, mut actual) = match (parse(expected, "fixture"), parse(actual, "engine")) {
        (Ok(expected), Ok(actual)) => (expected, actual),
        (Err(err), _) | (_, Err(err)) => {
            return CsvComparison {
                mismatch: Some(err),
                rounded_cells: 0,
            }
        }
    };
    let mut rounded_cells = 0;
    if numbers == CsvNumbers::RoundedToOracle && ordered {
        for (expected_row, actual_row) in expected.iter().zip(actual.iter_mut()).skip(1) {
            for (expected_cell, actual_cell) in expected_row.iter().zip(actual_row.iter_mut()) {
                let oracle = expected_cell.trim();
                if !is_plain_number(oracle) || !is_plain_number(actual_cell.trim()) {
                    continue;
                }
                let oracle_number = BigDecimal::from_str(oracle).expect("plain number");
                let zhang = BigDecimal::from_str(actual_cell.trim()).expect("plain number");
                if zhang != oracle_number && zhang.with_scale_round(oracle_number.fractional_digit_count(), RoundingMode::HalfEven) == oracle_number {
                    *actual_cell = oracle.to_owned();
                    rounded_cells += 1;
                }
            }
        }
    }
    let canonical = |records: &[Vec<String>]| {
        records
            .iter()
            .map(|record| Json::Array(record.iter().map(|cell| Json::String(canonical_csv_cell(cell))).collect()).to_string())
            .collect::<Vec<_>>()
    };
    let (expected, actual) = (canonical(&expected), canonical(&actual));
    let mismatch = match (expected.split_first(), actual.split_first()) {
        (Some((expected_header, _)), None) => Some(format!("engine CSV is empty, expected header {}", short(expected_header))),
        (Some((expected_header, _)), Some((actual_header, _))) if expected_header != actual_header => {
            Some(format!("CSV header: expected {} got {}", short(expected_header), short(actual_header)))
        }
        (Some((_, expected_rows)), Some((_, actual_rows))) => compare_rows(expected_rows, actual_rows, ordered).map(|diff| format!("CSV {diff}")),
        (None, _) => Some("fixture CSV is empty".to_owned()),
    };
    CsvComparison { mismatch, rounded_cells }
}

// ---------------------------------------------------------------------------------------------------------------
// Running a case
// ---------------------------------------------------------------------------------------------------------------

/// How a set is run: the date of `today()` and the CSV number rule.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    /// the fixed `today()` of every execution (no case depends on it; a fixed date keeps the runs reproducible)
    pub today: NaiveDate,
    pub csv_numbers: CsvNumbers,
}

impl Rules {
    /// The conformance rules with `today()` on `today`.
    pub fn on(today: NaiveDate) -> Rules {
        Rules {
            today,
            csv_numbers: CsvNumbers::Exact,
        }
    }
}

/// How a documented deviation from beanquery is checked.
#[derive(Debug, Clone, Copy)]
pub enum Accepted {
    /// The engine must return exactly these rows (fixture cell encoding, as JSON text) instead of the oracle's.
    /// Columns are still compared against the fixture.
    Rows(&'static str),
    /// The engine must return exactly these rows, cells written as `Value::to_string()` writes them (read with
    /// [`text_cell`] and the case's column types), instead of the oracle's.
    TextRows(&'static [&'static [&'static str]]),
    /// The fixture itself was generated to match zhang's behaviour, so it is compared as usual.
    InFixture,
    /// No fixture can exercise this, because beanquery cannot produce an expectation.
    NoFixture,
}

/// A deliberate difference between the engine and beanquery. Keep each entry justified.
#[derive(Debug, Clone, Copy)]
pub struct Deviation {
    /// the case name (`name` field), or `None` when no case exercises the deviation
    pub case: Option<&'static str>,
    pub reason: &'static str,
    pub accepted: Accepted,
}

/// A `ledger-dependent` case allowed to differ from beanquery because zhang processes the ledger differently
/// (booking, the price map, or data the store does not keep).
#[derive(Debug, Clone, Copy)]
pub struct LedgerGap {
    pub case: &'static str,
    pub reason: &'static str,
}

/// The status of a case. Only `Fail` fails a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// matches the oracle
    Pass,
    /// differs from the oracle exactly as its [`Deviation`] documents
    Accepted,
    /// a `ledger-dependent` case listed as a [`LedgerGap`] that differs
    LedgerDep,
    /// anything else, including unlisted `ledger-dependent` cases and missing functions
    Fail,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Accepted => "ACCEPTED",
            Status::LedgerDep => "LEDGER-DEP",
            Status::Fail => "FAIL",
        }
    }

    /// every status, in the order of the summary
    pub const ALL: [Status; 4] = [Status::Pass, Status::Accepted, Status::LedgerDep, Status::Fail];
}

/// The outcome of one case.
#[derive(Debug, Clone)]
pub struct Report {
    pub file: String,
    pub name: String,
    pub query: String,
    pub kind: String,
    pub status: Status,
    pub detail: String,
}

impl Report {
    /// whether the case does not fail
    pub fn ok(&self) -> bool {
        self.status != Status::Fail
    }

    /// Panics, naming the case and the difference, when the case fails.
    pub fn assert_ok(&self) {
        assert!(self.ok(), "{} ({}): {}\n  {}", self.file, self.name, self.detail, self.query);
    }
}

/// Compile and execute a case's query on `ledger` under `rules`.
pub fn execute(ledger: &Ledger, case: &Case, rules: &Rules) -> Result<QueryResult, QueryError> {
    Query::compile(&case.query).and_then(|query| {
        query.execute_with_options(
            ledger,
            &Params::default(),
            &ExecuteOptions {
                today: Some(rules.today),
                ..ExecuteOptions::default()
            },
        )
    })
}

/// How an outcome compares with its case: `Ok(note)` when it matches (the note says what the rules accepted, or
/// is empty), `Err(diff)` when it differs.
pub fn compare(case: &Case, outcome: &Result<QueryResult, QueryError>, rules: &Rules) -> Result<String, String> {
    match (outcome, &case.expect) {
        (Err(err), Expect::Error(class)) if error_class(err.kind) == class => Ok(String::new()),
        (Err(err), Expect::Error(class)) => Err(format!("expected a {} error, got a {} error: {}", class, error_class(err.kind), err)),
        (Ok(result), Expect::Error(class)) => Err(format!("expected a {} error, the engine returned {} rows", class, result.rows.len())),
        (Err(err), Expect::Rows | Expect::Csv(_)) => Err(format!("engine error: {err}")),
        (Ok(result), Expect::Rows) => match compare_columns(case, result) {
            Some(diff) => Err(diff),
            None => {
                let expected = canonical_fixture_rows(&case.column_types, &case.rows);
                compare_rows(&expected, &canonical_engine_rows(result), case.ordered).map_or(Ok(String::new()), Err)
            }
        },
        (Ok(result), Expect::Csv(lines)) => {
            let comparison = compare_csv_with(&lines.join("\n"), &zhang_query::export::to_csv(result), case.ordered, rules.csv_numbers);
            match comparison.mismatch {
                Some(diff) => Err(diff),
                None if comparison.rounded_cells > 0 => Ok(format!("{} cells equal once rounded to the oracle's scale", comparison.rounded_cells)),
                None => Ok(String::new()),
            }
        }
    }
}

/// The rows a deviation accepts, in the fixture cell encoding.
fn accepted_rows(accepted: &Accepted, case: &Case) -> Option<Vec<Vec<Json>>> {
    match accepted {
        Accepted::Rows(rows) => Some(serde_json::from_str::<Vec<Vec<Json>>>(rows).expect("accepted rows are JSON")),
        Accepted::TextRows(rows) => Some(
            rows.iter()
                .map(|row| row.iter().zip(&case.column_types).map(|(cell, ty)| text_cell(cell, ty)).collect())
                .collect(),
        ),
        Accepted::InFixture | Accepted::NoFixture => None,
    }
}

/// Run `case` on `ledger` and compare, applying the [`Deviation`]s and [`LedgerGap`]s the test lists.
///
/// A deviation whose accepted rows beanquery now agrees with is stale and fails the case, as does an accepted
/// deviation the engine does not reproduce exactly.
pub fn run_case(ledger: &Ledger, case: &Case, rules: &Rules, deviations: &[Deviation], gaps: &[LedgerGap]) -> Report {
    let report = |status: Status, detail: String| Report {
        file: case.file.clone(),
        name: case.name.clone(),
        query: case.query.clone(),
        kind: case.kind.clone(),
        status,
        detail,
    };
    let deviation = deviations.iter().find(|it| it.case == Some(case.name.as_str()));
    let gap = gaps.iter().find(|it| it.case == case.name);
    let accepted = deviation.and_then(|deviation| accepted_rows(&deviation.accepted, case));
    let expected = canonical_fixture_rows(&case.column_types, &case.rows);
    if let Some(accepted) = &accepted {
        if canonical_fixture_rows(&case.column_types, accepted) == expected {
            return report(
                Status::Fail,
                "stale deviation: beanquery agrees with the accepted rows; remove the entry".to_owned(),
            );
        }
    }

    let outcome = execute(ledger, case, rules);
    let (status, detail) = match compare(case, &outcome, rules) {
        Ok(_) if gap.is_some() => (
            Status::Pass,
            "matches beanquery although LEDGER_DEPENDENT_ALLOWED lists it; remove the entry".to_owned(),
        ),
        Ok(note) => match deviation {
            Some(Deviation {
                accepted: Accepted::Rows(_) | Accepted::TextRows(_),
                ..
            }) => (
                Status::Pass,
                "matches beanquery although an accepted deviation is listed; review the entry".to_owned(),
            ),
            Some(Deviation {
                accepted: Accepted::InFixture,
                reason,
                ..
            }) => (Status::Pass, format!("deviation encoded in the fixture: {reason}")),
            _ => (Status::Pass, note),
        },
        Err(diff) => match (deviation, &accepted, &outcome) {
            (Some(deviation), Some(accepted), Ok(result)) => {
                let expected = canonical_fixture_rows(&case.column_types, accepted);
                match compare_columns(case, result).or_else(|| compare_rows(&expected, &canonical_engine_rows(result), case.ordered)) {
                    None => (Status::Accepted, deviation.reason.to_owned()),
                    Some(diff) => (Status::Fail, format!("differs from the accepted deviation: {diff}")),
                }
            }
            _ => match gap {
                Some(gap) => (Status::LedgerDep, format!("{diff} ({})", gap.reason)),
                None => (Status::Fail, diff),
            },
        },
    };
    report(status, detail)
}

/// Check that every named [`Deviation`] and every [`LedgerGap`] refers to a case of `cases` (a gap to a
/// `ledger-dependent` one). Panics naming the first entry that does not.
pub fn check_lists(cases: &[Case], deviations: &[Deviation], gaps: &[LedgerGap]) {
    for deviation in deviations {
        if let Some(case) = deviation.case {
            assert!(cases.iter().any(|it| it.name == case), "ACCEPTED_DEVIATIONS refers to unknown case `{case}`");
        }
    }
    for gap in gaps {
        assert!(
            cases.iter().any(|it| it.name == gap.case && it.kind == "ledger-dependent"),
            "LEDGER_DEPENDENT_ALLOWED refers to `{}`, which is not a ledger-dependent case",
            gap.case
        );
    }
}

/// The reports of `cases` on `ledger`, in order.
pub fn run_cases(ledger: &Ledger, cases: &[Case], rules: &Rules, deviations: &[Deviation], gaps: &[LedgerGap]) -> Vec<Report> {
    cases.iter().map(|case| run_case(ledger, case, rules, deviations, gaps)).collect()
}

/// The summary table of a run: one line per case (status, file, kind, detail), the count per status, and the
/// accepted deviations no fixture exercises.
pub fn summary(title: &str, reports: &[Report], deviations: &[Deviation]) -> String {
    let mut out = format!("\n{title}\n\n");
    for report in reports {
        let _ = writeln!(
            out,
            "{:<16} {:<50} {:<16} {}",
            report.status.label(),
            report.file,
            report.kind,
            short(&report.detail)
        );
    }
    out.push('\n');
    for status in Status::ALL {
        let count = reports.iter().filter(|report| report.status == status).count();
        let _ = writeln!(out, "{:<16} {}", status.label(), count);
    }
    let _ = writeln!(out, "{:<16} {}", "TOTAL", reports.len());
    for deviation in deviations.iter().filter(|it| matches!(it.accepted, Accepted::NoFixture)) {
        let _ = write!(out, "\naccepted deviation without a fixture: {}\n", deviation.reason);
    }
    out
}

/// The reports that fail.
pub fn failures(reports: &[Report]) -> Vec<&Report> {
    reports.iter().filter(|report| report.status == Status::Fail).collect()
}

/// Panics listing every failing report, naming `what` (the set).
pub fn assert_no_failures(what: &str, reports: &[Report]) {
    let failures = failures(reports);
    assert!(
        failures.is_empty(),
        "{} of {} {} case(s) differ from beanquery or from the accepted deviations:\n{}",
        failures.len(),
        reports.len(),
        what,
        failures
            .iter()
            .map(|report| format!("  {}: {}\n    {}", report.file, report.detail, report.query))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
