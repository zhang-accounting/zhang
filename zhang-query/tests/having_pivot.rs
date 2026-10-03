//! Oracle tests of `HAVING` and `PIVOT BY`: runs the beanquery-generated fixtures in
//! `tests/having_pivot/cases` against the engine on the shared fava demo ledger, with the
//! comparison rules of the conformance suite (`tests/conformance/README.md`): column types by
//! position, decimals numerically, inventories and unordered results as multisets, errors by
//! class, CSV cells trimmed and compared numerically when they are numbers.
//!
//! Unlike the conformance suite, column **names** are compared too: the columns of a pivot
//! are named after the data, so their names are part of the result. Every computed target of
//! these fixtures has an alias, so names never depend on how an engine spells an expression.
//!
//! The fixtures come from `tests/having_pivot/generate.py`, which runs the official beanquery
//! with the conformance generator's validation (determinism, zhang's balance-check rows).

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_query::decimal::to_plain_string;
use zhang_query::{Amount, DataType, Params, Position, Query, QueryErrorKind, QueryResult, Value};

enum Expect {
    Rows,
    Error(String),
    Csv(Vec<String>),
}

struct Fixture {
    file: String,
    query: String,
    ordered: bool,
    expect: Expect,
    /// (name, type) of every column
    columns: Vec<(String, String)>,
    rows: Vec<Vec<Json>>,
}

fn load_fixtures() -> Vec<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/having_pivot/cases");
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
            let expect = match json["expect"].as_str().expect("expect") {
                "rows" => Expect::Rows,
                "error" => Expect::Error(json["error_class"].as_str().expect("error_class").to_owned()),
                "csv" => Expect::Csv(
                    json["csv"]
                        .as_array()
                        .expect("csv")
                        .iter()
                        .map(|line| line.as_str().expect("csv line").to_owned())
                        .collect(),
                ),
                other => panic!("{}: unknown expect {:?}", file, other),
            };
            Fixture {
                query: json["query"].as_str().expect("query").to_owned(),
                ordered: json["ordered"].as_bool().expect("ordered"),
                expect,
                columns: json["columns"]
                    .as_array()
                    .expect("columns")
                    .iter()
                    .map(|column| {
                        (
                            column["name"].as_str().expect("column name").to_owned(),
                            column["type"].as_str().expect("column type").to_owned(),
                        )
                    })
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
        QueryErrorKind::TooLarge => "too_large",
    }
}

fn compare_rows(mut expected: Vec<String>, mut actual: Vec<String>, ordered: bool) -> Option<String> {
    if !ordered {
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

/// The records of a CSV text without quoted fields (the fixtures have none), each cell
/// trimmed and, when it is a plain number, normalised.
fn csv_records(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            assert!(!line.contains('"'), "quoted CSV fields are not expected: {}", line);
            let cells = line
                .trim_end_matches('\r')
                .split(',')
                .map(|cell| {
                    let cell = cell.trim();
                    match BigDecimal::from_str(cell) {
                        Ok(number) if cell.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') => decimal(&number),
                        _ => Json::String(cell.to_owned()),
                    }
                })
                .collect();
            Json::Array(cells).to_string()
        })
        .collect()
}

/// `None` when the engine matches the fixture, otherwise what differs.
fn compare(fixture: &Fixture, outcome: Result<QueryResult, zhang_query::QueryError>) -> Option<String> {
    let result = match (outcome, &fixture.expect) {
        (Err(err), Expect::Error(class)) if error_class(err.kind) == class => return None,
        (Err(err), _) => return Some(format!("engine error: {}", err)),
        (Ok(result), Expect::Error(class)) => return Some(format!("expected a {} error, got {} rows", class, result.rows.len())),
        (Ok(result), Expect::Csv(lines)) => {
            let (expected, actual) = (csv_records(&lines.join("\n")), csv_records(&zhang_query::export::to_csv(&result)));
            if expected.first() != actual.first() {
                return Some(format!("CSV header: expected {:?}, got {:?}", expected.first(), actual.first()));
            }
            return compare_rows(expected[1..].to_vec(), actual[1..].to_vec(), fixture.ordered).map(|diff| format!("CSV {}", diff));
        }
        (Ok(result), Expect::Rows) => result,
    };
    let columns = result
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.ty.name().to_owned()))
        .collect::<Vec<_>>();
    if columns != fixture.columns {
        return Some(format!("columns: expected {:?}, got {:?}", fixture.columns, columns));
    }
    let expected = fixture
        .rows
        .iter()
        .map(|row| {
            let cells = row.iter().zip(&fixture.columns).map(|(cell, (_, ty))| fixture_cell(cell, ty)).collect();
            Json::Array(cells).to_string()
        })
        .collect::<Vec<_>>();
    let actual = result
        .rows
        .iter()
        .map(|row| Json::Array(row.iter().map(engine_cell).collect()).to_string())
        .collect::<Vec<_>>();
    compare_rows(expected, actual, fixture.ordered)
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
}

#[test]
fn having_and_pivot_match_beanquery() {
    let fixtures = load_fixtures();
    assert!(fixtures.len() >= 20, "fixtures missing from tests/having_pivot/cases");
    let ledger = common::fava_demo_ledger();
    let failures = fixtures
        .iter()
        .filter_map(|fixture| {
            let outcome = Query::compile(&fixture.query).and_then(|query| query.execute_at(&ledger, &Params::default(), today()));
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

fn run(query: &str) -> QueryResult {
    let ledger = common::fava_demo_ledger();
    Query::compile(query)
        .and_then(|query| query.execute_at(&ledger, &Params::default(), today()))
        .unwrap_or_else(|err| panic!("{}: {}", query, err))
}

fn compile_error(query: &str) -> zhang_query::QueryError {
    Query::compile(query).err().unwrap_or_else(|| panic!("{} compiles", query))
}

/// HAVING reads GROUP BY keys per group (beanquery reads an arbitrary posting there), and
/// columns outside an aggregate that are not keys are rejected.
#[test]
fn having_reads_group_keys_per_group() {
    let result = run("SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account \
         HAVING account ~ 'Rest|Coffee' AND count(*) > 10 ORDER BY account");
    let accounts = result.rows.iter().map(|row| row[0].to_string()).collect::<Vec<_>>();
    assert_eq!(accounts, vec!["Expenses:Food:Coffee", "Expenses:Food:Restaurant"]);

    // a key that is not selected, and an expression equal to a key
    let result = run("SELECT count(*) AS n WHERE account ~ '^Expenses' GROUP BY root(account, 2), year \
         HAVING root(account, 2) = 'Expenses:Home' AND year = 2016 AND count(*) > 0");
    assert_eq!(result.rows, vec![vec![Value::Int(48)]]);

    let err = compile_error("SELECT account, count(*) GROUP BY account HAVING payee = 'x' AND count(*) > 0");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert!(err.message.contains("'payee' must be a GROUP BY key"), "{}", err.message);
    assert_eq!(err.column, Some(50));
    let err = compile_error("SELECT account, count(*) GROUP BY account HAVING entry_meta('x') = 'y' AND count(*) > 0");
    assert!(err.message.contains("entry_meta(...)"), "{}", err.message);
    let err = compile_error("SELECT account, count(*) GROUP BY account HAVING str(balance) = '' AND count(*) > 0");
    assert!(err.message.contains("'balance' must be a GROUP BY key"), "{}", err.message);
}

#[test]
fn having_errors_explain_the_rule() {
    let err = compile_error("SELECT account, sum(number) AS total GROUP BY account HAVING total > 10");
    assert!(err.message.contains("HAVING cannot use target names"), "{}", err.message);
    let err = compile_error("SELECT account, count(*) GROUP BY account HAVING sum(number)");
    assert!(err.message.contains("HAVING expects a boolean expression, got decimal"), "{}", err.message);
    let err = compile_error("SELECT account, count(*) GROUP BY account HAVING account = 'x'");
    assert!(err.message.contains("HAVING must use an aggregate function"), "{}", err.message);
    let err = compile_error("SELECT account, sum(position) GROUP BY account HAVING sum(position) > 10");
    assert!(err.message.contains("cannot compare inventory with int: "), "{}", err.message);
    assert!(err.message.contains("number(only('USD', sum(position)))"), "{}", err.message);
    let err = compile_error("SELECT count(*) HAVING count(*) > 1");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(17)));
    let err = compile_error("SELECT account, count(*) GROUP BY account ORDER BY account HAVING count(*) > 1");
    assert!(err.message.contains("HAVING must follow GROUP BY"), "{}", err.message);
}

/// HAVING works with the deferred running balance and with parameters.
#[test]
fn having_with_balance_aggregates_and_parameters() {
    let ledger = common::fava_demo_ledger();
    let query = Query::compile_with_params(
        "SELECT account, last(balance) AS balance WHERE account ~ '^Assets:US:BofA' GROUP BY account \
         HAVING count(*) > :min AND last(balance) IS NOT NULL ORDER BY account",
        &zhang_query::ParamTypes::new().bind("min", DataType::Int),
    )
    .unwrap();
    let explain = query.explain();
    assert!(explain.contains("having: ((agg#1 > :min) AND (target#1 IS NOT NULL))\n"), "{}", explain);
    assert!(explain.contains("balance: deferred agg#0\n"), "{}", explain);
    let result = query.execute_at(&ledger, &Params::new().bind("min", 10), today()).unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Str("Assets:US:BofA:Checking".into()));
    let none = query.execute_at(&ledger, &Params::new().bind("min", 100_000), today()).unwrap();
    assert!(none.rows.is_empty());
}

/// The columns of a pivot are typed and named after the data, and encode to CSV with
/// numberify splitting the pivoted inventory columns.
#[test]
fn pivot_columns_are_typed_and_exported() {
    let query = "SELECT account, year, sum(position) AS total, count(*) AS n \
                 WHERE account ~ '^Expenses:Food:(Alcohol|Coffee)' GROUP BY 1, 2 PIVOT BY account, year";
    let result = run(query);
    let columns = result.columns.iter().map(|it| (it.name.as_str(), it.ty)).collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec![
            ("account/year", DataType::Str),
            ("2015/total", DataType::Inventory),
            ("2015/n", DataType::Int),
            ("2016/total", DataType::Inventory),
            ("2016/n", DataType::Int),
            ("2017/total", DataType::Inventory),
            ("2017/n", DataType::Int),
        ]
    );
    // the static columns are those before the pivot
    let compiled = Query::compile(query).unwrap();
    assert_eq!(compiled.columns().len(), 4);
    assert!(compiled.explain().contains("pivot by: 0 (rows), 1 (columns)\n"), "{}", compiled.explain());
    // Alcohol has no 2015 and 2017 postings: NULL cells, which numberify leaves empty
    assert_eq!(result.rows[0][1], Value::Null);
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "account/year,2015/total (USD),2015/n,2016/total (USD),2016/n,2017/total (USD),2017/n\r\n\
         Expenses:Food:Alcohol,,,63.58,7,,\r\n\
         Expenses:Food:Coffee,21.93,4,29.96,5,25.09,4\r\n"
    );
}

#[test]
fn pivot_errors_explain_the_rule() {
    let err = compile_error("SELECT account, year, number PIVOT BY account, year");
    assert!(err.message.contains("PIVOT BY needs an aggregate query"), "{}", err.message);
    let err = compile_error("SELECT account, year, sum(position) AS total GROUP BY 1, 2 PIVOT BY total, account");
    assert!(err.message.contains("values of type inventory cannot be pivoted"), "{}", err.message);
    let err = compile_error("SELECT account, year, count(*) AS n GROUP BY 1, 2 PIVOT BY account, n");
    assert!(err.message.contains("the second PIVOT BY column must be a GROUP BY key"), "{}", err.message);
    assert_eq!(err.column, Some(69));
    let err = compile_error("SELECT account, year, count(*) AS n GROUP BY 1, 2 PIVOT BY account");
    assert_eq!(err.kind, QueryErrorKind::Parse);
    let err = compile_error("SELECT account, year, count(*) AS n GROUP BY 1, 2 PIVOT BY account, sum(number)");
    assert_eq!(err.kind, QueryErrorKind::Parse);
    let err = compile_error("SELECT account, year, count(*) AS n GROUP BY 1, 2 LIMIT 1 PIVOT BY account, year");
    assert!(err.message.contains("PIVOT BY must come before LIMIT"), "{}", err.message);
}

/// A pivot is charged to the result budget before it is built: rows × columns cells, even
/// when most of them are NULL.
#[test]
fn pivot_respects_the_result_budget() {
    let ledger = common::fava_demo_ledger();
    // about 1,000 dates × 60 accounts: 60,000 cells from about 3,000 groups
    let query = Query::compile("SELECT date, account, count(*) AS n GROUP BY 1, 2 PIVOT BY date, account").unwrap();
    let options = |limit: u64| zhang_query::ExecuteOptions {
        today: Some(today()),
        timeout: None,
        max_result_values: Some(limit),
    };
    let err = query.execute_with_options(&ledger, &Params::new(), &options(30_000)).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge);
    let result = query.execute_with_options(&ledger, &Params::new(), &options(1_000_000)).unwrap();
    assert!(
        result.columns.len() > 50 && result.rows.len() > 500,
        "{} × {}",
        result.rows.len(),
        result.columns.len()
    );
    assert!(result.rows.iter().all(|row| row.len() == result.columns.len()));
}

/// Column names repeat target names that can be as long as the query: they are charged to
/// the budget before they are built.
#[test]
fn pivot_column_names_count_towards_the_result_budget() {
    let alias = |c: char| c.to_string().repeat(31_000);
    let sql = format!("SELECT 'r' AS r, id, 1 AS {}, 2 AS {} GROUP BY r, id PIVOT BY r, id", alias('a'), alias('b'));
    assert!(sql.len() < zhang_query::MAX_QUERY_LENGTH);
    let ledger = common::fava_demo_ledger();
    // a thousand ids, two 31 KB names each: about 1,000,000 values of names for 2,000 cells
    let err = Query::compile(&sql).unwrap().execute_at(&ledger, &Params::default(), today()).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", err);
}

#[test]
fn explain_shows_having_and_pivot() {
    let query = Query::compile(
        "SELECT year, root(account, 2) AS category, sum(number) AS total WHERE account ~ '^Expenses' GROUP BY 1, 2 \
         HAVING sum(number) > 500 + 500 AND count(*) > 1 AND NOT NOT TRUE ORDER BY 3 DESC PIVOT BY category, year LIMIT 8",
    )
    .unwrap();
    // the aggregate target is reused, constants are folded, and LIMIT applies after HAVING
    assert_eq!(
        query.explain(),
        "target 0: year = year : int\n\
         target 1: category = root(account, 2) : str\n\
         target 2: total = agg#0 : decimal\n\
         agg#0: sum(number)\n\
         agg#1: count(*)\n\
         filter: (account ~ /^Expenses/i)\n\
         group by: [0, 1]\n\
         having: ((target#2 > 1000) AND (agg#1 > 1))\n\
         order by: 2 DESC\n\
         limit: 8\n\
         pivot by: 1 (rows), 0 (columns)\n\
         project: [account, number, year] (3 of 31 columns)\n"
    );
    // without ORDER BY, LIMIT only aggregates the first groups unless HAVING may drop some
    let limit = |sql: &str| {
        let explain = Query::compile(sql).unwrap().explain();
        explain.lines().find(|line| line.starts_with("limit:")).unwrap().to_owned()
    };
    assert_eq!(limit("SELECT account, count(*) GROUP BY account LIMIT 2"), "limit: 2 (first groups only)");
    assert_eq!(limit("SELECT account, count(*) GROUP BY account HAVING count(*) > 9 LIMIT 2"), "limit: 2");
    // a HAVING that always holds is dropped
    let always = Query::compile("SELECT account, count(*) GROUP BY account HAVING count(*) > 9 OR TRUE LIMIT 2").unwrap();
    assert!(!always.explain().contains("having:"), "{}", always.explain());
    assert_eq!(
        limit("SELECT account, count(*) GROUP BY account HAVING count(*) > 9 OR TRUE LIMIT 2"),
        "limit: 2 (first groups only)"
    );
    let never = run("SELECT account, count(*) GROUP BY account HAVING count(*) > 9 AND FALSE");
    assert!(never.rows.is_empty());
}

/// HAVING compiles and evaluates nested conditions up to the nesting limit on the 2 MiB stack
/// of tokio's blocking pool, with key references at the leaves.
#[test]
fn deep_having_runs_on_a_small_stack() {
    let depth = zhang_query::MAX_DEPTH - 8;
    let queries = [
        format!(
            "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account HAVING {}count(*) > 300",
            "NOT NOT ".repeat(depth / 2)
        ),
        format!(
            "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account \
             HAVING {}account{} ~ 'Rest' AND count(*) > 0",
            "root(".repeat(depth - 2),
            ", 9)".repeat(depth - 2)
        ),
    ];
    for sql in queries {
        let rows = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || run(&sql).rows)
            .unwrap()
            .join()
            .expect("the query thread crashed");
        assert_eq!(rows, vec![vec![Value::from("Expenses:Food:Restaurant"), Value::Int(370)]]);
    }
}
