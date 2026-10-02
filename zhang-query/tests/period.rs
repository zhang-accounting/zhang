//! Oracle tests of the FROM period modifiers (`OPEN ON`, `CLOSE [ON]`, `CLEAR`): runs the
//! beanquery-generated fixtures in `tests/period/cases` against the engine on the shared fava
//! demo ledger, with the comparison rules of the conformance suite
//! (`tests/conformance/README.md`): columns by position and type, decimals numerically,
//! inventories and unordered results as multisets, errors by class.
//!
//! The fixtures come from `tests/period/generate.py`, which runs the official beanquery with
//! the conformance generator's validation (determinism, zhang's balance-check rows).

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_query::decimal::to_plain_string;
use zhang_query::{Amount, DataType, ParamTypes, Params, Position, Query, QueryErrorKind, QueryResult, Value};

struct Fixture {
    file: String,
    query: String,
    ordered: bool,
    /// `Some(class)` for an `expect: "error"` case
    expect_error: Option<String>,
    column_types: Vec<String>,
    rows: Vec<Vec<Json>>,
}

fn load_fixtures() -> Vec<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/period/cases");
    let mut files = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {}", dir.display(), err))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let json: Json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|err| panic!("{}: {}", file, err));
            Fixture {
                query: json["query"].as_str().expect("query").to_owned(),
                ordered: json["ordered"].as_bool().expect("ordered"),
                expect_error: (json["expect"] == "error").then(|| json["error_class"].as_str().expect("error_class").to_owned()),
                column_types: json["columns"]
                    .as_array()
                    .expect("columns")
                    .iter()
                    .map(|column| column["type"].as_str().expect("column type").to_owned())
                    .collect(),
                rows: json["rows"]
                    .as_array()
                    .expect("rows")
                    .iter()
                    .map(|row| row.as_array().expect("row").clone())
                    .collect(),
                file,
            }
        })
        .collect()
}

// canonical encoding: the fixture cell encoding with normalised decimals and sorted positions

fn decimal(number: &BigDecimal) -> Json {
    Json::String(to_plain_string(&number.normalized()))
}

fn decimal_str(text: &str) -> Json {
    decimal(&BigDecimal::from_str(text).unwrap_or_else(|err| panic!("bad decimal {:?}: {}", text, err)))
}

fn engine_amount(amount: &Amount) -> Json {
    json!({"number": decimal(&amount.number), "currency": amount.commodity})
}

fn engine_position(position: &Position) -> Json {
    let cost = position.cost.as_ref().map(|cost| {
        json!({
            "number": decimal(&cost.number),
            "currency": cost.currency,
            "date": cost.date.map(|date| date.to_string()),
            "label": cost.label,
        })
    });
    json!({"units": engine_amount(&position.units), "cost": cost})
}

fn sorted_positions(mut positions: Vec<Json>) -> Json {
    positions.sort_by_cached_key(|position| position.to_string());
    json!({ "positions": positions })
}

fn engine_cell(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(it) => json!(it),
        Value::Int(it) => json!(it),
        Value::Decimal(it) => decimal(it),
        Value::Str(it) => json!(it),
        Value::Date(it) => json!(it.to_string()),
        Value::Set(it) => json!(it.iter().collect::<Vec<_>>()),
        Value::Amount(it) => engine_amount(it),
        Value::Position(it) => engine_position(it),
        Value::Inventory(it) => sorted_positions(it.positions().map(|position| engine_position(&position)).collect()),
    }
}

fn fixture_amount(cell: &Json) -> Json {
    json!({"number": decimal_str(cell["number"].as_str().expect("amount number")), "currency": cell["currency"]})
}

fn fixture_position(cell: &Json) -> Json {
    let cost = match &cell["cost"] {
        Json::Null => Json::Null,
        cost => json!({
            "number": decimal_str(cost["number"].as_str().expect("cost number")),
            "currency": cost["currency"],
            "date": cost["date"],
            "label": cost["label"],
        }),
    };
    json!({"units": fixture_amount(&cell["units"]), "cost": cost})
}

fn fixture_cell(cell: &Json, ty: &str) -> Json {
    if cell.is_null() {
        return Json::Null;
    }
    match ty {
        "decimal" => decimal_str(cell.as_str().expect("decimal cell")),
        "amount" => fixture_amount(cell),
        "position" => fixture_position(cell),
        "inventory" => sorted_positions(cell["positions"].as_array().expect("positions").iter().map(fixture_position).collect()),
        "set" => json!(cell
            .as_array()
            .expect("set cell")
            .iter()
            .map(|it| it.as_str().expect("set item").to_owned())
            .collect::<BTreeSet<_>>()),
        _ => cell.clone(),
    }
}

fn error_class(kind: QueryErrorKind) -> &'static str {
    match kind {
        QueryErrorKind::Parse => "syntax",
        QueryErrorKind::Compile => "compile",
        QueryErrorKind::Eval => "runtime",
        QueryErrorKind::Timeout => "timeout",
    }
}

/// `None` when the engine matches the fixture, otherwise what differs.
fn compare(fixture: &Fixture, outcome: Result<QueryResult, zhang_query::QueryError>) -> Option<String> {
    let result = match (outcome, &fixture.expect_error) {
        (Err(err), Some(class)) if error_class(err.kind) == class => return None,
        (Err(err), _) => return Some(format!("engine error: {}", err)),
        (Ok(result), Some(class)) => return Some(format!("expected a {} error, got {} rows", class, result.rows.len())),
        (Ok(result), None) => result,
    };
    let types = result.columns.iter().map(|column| column.ty.name()).collect::<Vec<_>>();
    if types != fixture.column_types {
        return Some(format!("columns: expected {:?}, got {:?}", fixture.column_types, types));
    }
    let mut expected = fixture
        .rows
        .iter()
        .map(|row| {
            let cells = row.iter().zip(&fixture.column_types).map(|(cell, ty)| fixture_cell(cell, ty)).collect();
            Json::Array(cells).to_string()
        })
        .collect::<Vec<_>>();
    let mut actual = result
        .rows
        .iter()
        .map(|row| Json::Array(row.iter().map(engine_cell).collect()).to_string())
        .collect::<Vec<_>>();
    if !fixture.ordered {
        expected.sort();
        actual.sort();
    }
    if expected == actual {
        return None;
    }
    let missing = expected.iter().filter(|row| !actual.contains(row)).take(3).cloned().collect::<Vec<_>>();
    let unexpected = actual.iter().filter(|row| !expected.contains(row)).take(3).cloned().collect::<Vec<_>>();
    Some(format!(
        "{} expected rows, {} actual; missing {:?}; unexpected {:?}",
        expected.len(),
        actual.len(),
        missing,
        unexpected
    ))
}

#[test]
fn period_modifiers_match_beanquery() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures in tests/period/cases");
    let ledger = common::fava_demo_ledger();
    let today = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let failures = fixtures
        .iter()
        .filter_map(|fixture| {
            let outcome = Query::compile(&fixture.query).and_then(|query| query.execute_at(&ledger, &Params::default(), today));
            compare(fixture, outcome).map(|diff| format!("  {}: {}", fixture.file, diff))
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from beanquery:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn run(ledger: &zhang_core::ledger::Ledger, query: &str) -> QueryResult {
    Query::compile(query)
        .and_then(|query| query.execute_at(ledger, &Params::default(), date(2025, 1, 1)))
        .unwrap_or_else(|err| panic!("{}: {}", query, err))
}

fn cells(result: &QueryResult) -> Vec<Vec<String>> {
    result.rows.iter().map(|row| row.iter().map(ToString::to_string).collect()).collect()
}

#[test]
fn period_dates_can_be_parameters() {
    let ledger = common::fava_demo_ledger();
    let literal = run(
        &ledger,
        "SELECT account, sum(position) FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ '^Income' GROUP BY 1 ORDER BY 1",
    );
    let query = Query::compile_with_params(
        "SELECT account, sum(position) FROM OPEN ON :from CLOSE ON $1 WHERE account ~ '^Income' GROUP BY 1 ORDER BY 1",
        &ParamTypes::new().bind("from", DataType::Date).push(DataType::Date),
    )
    .unwrap();
    let params = Params::new().bind("from", date(2016, 1, 1)).push(date(2017, 1, 1));
    let bound = query.execute_at(&ledger, &params, date(2025, 1, 1)).unwrap();
    assert_eq!(bound, literal);
    assert!(query.explain().contains("period: OPEN ON :from CLOSE ON $1\n"), "{}", query.explain());

    // a NULL date and a CLOSE before OPEN are execution errors
    let null = query.execute_at(&ledger, &Params::new().bind("from", Value::Null).push(date(2017, 1, 1)), date(2025, 1, 1));
    assert!(null.unwrap_err().message.contains("OPEN ON needs a date"));
    let reversed = query.execute_at(&ledger, &Params::new().bind("from", date(2017, 1, 1)).push(date(2016, 1, 1)), date(2025, 1, 1));
    assert!(reversed.unwrap_err().message.contains("before the OPEN date"));

    // a parameter of another type is rejected when compiling
    let err = Query::compile_with_params("SELECT count(*) FROM OPEN ON :from", &ParamTypes::new().bind("from", DataType::Str))
        .err()
        .expect("a str parameter is rejected");
    assert_eq!(err.kind, QueryErrorKind::Compile);
}

/// Lots, costs, price conversions and the equity options on a small ledger whose expected
/// rows are worked out by hand from beancount's summarize, truncate and clear operations.
#[test]
fn period_modifiers_on_a_small_ledger() {
    let ledger = common::load_text(
        r#"
option "operating_currency" "USD"
option "account_previous_earnings" "Retained"
option "conversion_currency" "ZERO"
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Euro
1970-01-01 open Assets:Stock
1970-01-01 open Income:Salary
1970-01-01 open Expenses:Food
1970-01-01 open Equity:Opening-Balances

2023-01-10 * "salary"
  Assets:Cash  1000 USD
  Income:Salary  -1000 USD

2023-02-01 * "buy"
  Assets:Stock  10 AAA {10 USD}
  Assets:Cash  -100 USD

2023-03-01 * "buy more"
  Assets:Stock  5 AAA {12 USD}
  Assets:Cash  -60 USD

2023-06-01 * "exchange"
  Assets:Euro  90 EUR @ 1.1 USD
  Assets:Cash  -99 USD

2024-01-05 * "lunch"
  Expenses:Food  20 USD
  Assets:Cash  -20 USD

2024-02-01 * "sell"
  Assets:Stock  -10 AAA {10 USD}
  Assets:Cash  100 USD
"#,
    );

    // OPEN: per-lot opening balances against Equity:Opening-Balances at cost, earnings moved to
    // Equity:Retained (option), conversions to Equity:Conversions:Previous
    let opened = run(
        &ledger,
        "SELECT date, flag, account, position, other_accounts FROM OPEN ON 2024-01-01 WHERE flag = 'S' AND account !~ 'Opening'",
    );
    assert_eq!(
        cells(&opened),
        vec![
            vec!["2023-12-31", "S", "Assets:Cash", "741 USD", "Equity:Opening-Balances"],
            vec!["2023-12-31", "S", "Assets:Euro", "90 EUR", "Equity:Opening-Balances"],
            // other_accounts are the accounts of the entry's other postings, so a second lot shows
            vec![
                "2023-12-31",
                "S",
                "Assets:Stock",
                "10 AAA {10 USD, 2023-02-01}",
                "Assets:Stock, Equity:Opening-Balances"
            ],
            vec![
                "2023-12-31",
                "S",
                "Assets:Stock",
                "5 AAA {12 USD, 2023-03-01}",
                "Assets:Stock, Equity:Opening-Balances"
            ],
            // the conversions follow the order in which their currencies first appeared
            vec![
                "2023-12-31",
                "S",
                "Equity:Conversions:Previous",
                "99 USD",
                "Equity:Conversions:Previous, Equity:Opening-Balances"
            ],
            vec![
                "2023-12-31",
                "S",
                "Equity:Conversions:Previous",
                "-90 EUR",
                "Equity:Conversions:Previous, Equity:Opening-Balances"
            ],
            vec!["2023-12-31", "S", "Equity:Retained", "-1000 USD", "Equity:Opening-Balances"],
        ]
    );

    // the sale after OPEN reduces the summarized lot
    let holdings = run(&ledger, "SELECT sum(position) FROM OPEN ON 2024-01-01 WHERE account = 'Assets:Stock'");
    assert_eq!(cells(&holdings), vec![vec!["5 AAA {12 USD, 2023-03-01}"]]);

    // CLOSE: the conversion entry of the period, priced at zero in the conversion currency
    let closed = run(
        &ledger,
        "SELECT date, flag, account, position, price, weight FROM CLOSE ON 2024-01-01 WHERE flag = 'C' ORDER BY currency",
    );
    assert_eq!(
        cells(&closed),
        vec![
            vec!["2023-12-31", "C", "Equity:Conversions:Current", "-90 EUR", "0 ZERO", "0 ZERO"],
            vec!["2023-12-31", "C", "Equity:Conversions:Current", "99 USD", "0 ZERO", "0 ZERO"],
        ]
    );

    // CLEAR: transfers dated like the last entry, to Equity:Earnings:Current
    let cleared = run(
        &ledger,
        "SELECT date, flag, account, position FROM CLEAR WHERE flag = 'T' ORDER BY account, number",
    );
    assert_eq!(
        cells(&cleared),
        vec![
            vec!["2024-02-01", "T", "Equity:Earnings:Current", "-1000 USD"],
            vec!["2024-02-01", "T", "Equity:Earnings:Current", "20 USD"],
            vec!["2024-02-01", "T", "Expenses:Food", "-20 USD"],
            vec!["2024-02-01", "T", "Income:Salary", "1000 USD"],
        ]
    );

    // the whole period balances by weight: the conversion postings weigh 0 ZERO
    let net = run(&ledger, "SELECT sum(weight) FROM OPEN ON 2024-01-01 CLOSE ON 2024-12-31 CLEAR");
    assert_eq!(net.rows, vec![vec![Value::Inventory(Default::default())]]);
}
