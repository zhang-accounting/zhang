//! Evaluator tests over a small ledger: row source, operators, NULL handling, grouping,
//! ordering, aggregates, parameters and error positions.

mod common;

use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ParamTypes, Params, Query, QueryError, QueryErrorKind, Value};

const LEDGER: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity EUR
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Assets:Broker
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel
1970-01-01 open Income:Salary
1970-01-01 open Income:Gains
1970-01-01 open Equity:Opening

2024-01-01 price EUR 1.10 USD
2024-02-15 price AAPL 120.00 USD
2024-03-01 price AAPL 150.00 USD

2024-01-01 * "Employer" "January salary" #work
  Assets:Bank    1000.00 USD
  Income:Salary

2024-01-05 * "午餐" "lunch" #food ^receipt-1
  category: "meal"
  Expenses:Food    12.50 USD
  Assets:Bank

2024-01-10 * "Lunch"
  Expenses:Food     7.50 USD
  Assets:Bank

2024-02-01 * "Broker" "buy"
  Assets:Broker     5 AAPL {100.00 USD}
  Assets:Bank    -500.00 USD

2024-02-10 * "Broker" "buy more"
  Assets:Broker     5 AAPL {110.00 USD}
  Assets:Bank    -550.00 USD

2024-03-05 * "Broker" "sell"
  Assets:Broker    -7 AAPL {} @ 150.00 USD
  Assets:Bank    1050.00 USD
  Income:Gains   -330.00 USD

2024-03-10 * "Trip" "hotel" #travel
  Expenses:Travel  100.00 EUR @@ 110.00 USD
  Assets:Bank     -110.00 USD

2024-04-02 balance Assets:Bank 2000.00 USD with pad Equity:Opening

2024-04-03 balance Assets:Bank 2000.00 USD
"#;

fn ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| common::load_text(LEDGER))
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 6, 30).unwrap()
}

fn try_query(query: &str) -> Result<Vec<Vec<String>>, QueryError> {
    let result = Query::compile(query)?.execute_at(ledger(), &Params::new(), today())?;
    Ok(result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect())
}

fn query(query: &str) -> Vec<Vec<String>> {
    try_query(query).unwrap_or_else(|err| panic!("{}: {}", query, err))
}

/// The single column of every row.
fn column(sql: &str) -> Vec<String> {
    query(sql).into_iter().map(|mut row| row.remove(0)).collect()
}

fn one(sql: &str) -> String {
    let rows = column(sql);
    assert_eq!(rows.len(), 1, "{}: {:?}", sql, rows);
    rows.into_iter().next().unwrap()
}

fn error(sql: &str) -> QueryError {
    match try_query(sql) {
        Ok(rows) => panic!("{} should fail but returned {:?}", sql, rows),
        Err(err) => err,
    }
}

#[test]
fn rows_include_pads_and_exclude_balance_assertions() {
    // 7 transactions with 15 postings, the sell split over two lots (+1), and the pad (+2)
    assert_eq!(one("SELECT count(*)"), "18");
    assert_eq!(column("SELECT DISTINCT flag ORDER BY flag"), vec!["*", "P"]);
    assert!(query("SELECT account WHERE flag = 'C'").is_empty());
    assert_eq!(
        query("SELECT account, position WHERE flag = 'P'"),
        vec![vec!["Assets:Bank", "1130.00 USD"], vec!["Equity:Opening", "-1130.00 USD"]]
    );
}

#[test]
fn rows_come_in_ledger_order() {
    assert_eq!(
        column("SELECT date WHERE account = 'Assets:Bank'"),
        vec![
            "2024-01-01",
            "2024-01-05",
            "2024-01-10",
            "2024-02-01",
            "2024-02-10",
            "2024-03-05",
            "2024-03-10",
            "2024-04-02"
        ]
    );
}

#[test]
fn entry_columns() {
    assert_eq!(
        query("SELECT date, year, month, day, flag, payee, narration, description, tags, links WHERE narration = 'lunch'")[0],
        vec!["2024-01-05", "2024", "1", "5", "*", "午餐", "lunch", "午餐 | lunch", "food", "receipt-1"]
    );
    // a single string is the narration
    assert_eq!(
        query("SELECT payee, narration, description WHERE narration = 'Lunch'")[0],
        vec!["NULL", "Lunch", "Lunch"]
    );
    assert_eq!(
        query("SELECT other_accounts WHERE narration = 'sell' AND account = 'Assets:Bank'")[0],
        vec!["Assets:Broker, Income:Gains"]
    );
    let ids = column("SELECT DISTINCT id");
    assert_eq!(ids.len(), 8);
    assert!(ids.iter().all(|id| id.len() == 36));
}

#[test]
fn lot_reductions_are_booked_against_open_lots() {
    assert_eq!(
        query("SELECT number, cost_number, cost_currency, cost_date, cost_label, price, weight WHERE account = 'Assets:Broker' AND number < 0"),
        vec![
            vec!["-5", "100.00", "USD", "2024-02-01", "NULL", "150.00 USD", "-500.00 USD"],
            vec!["-2", "110.00", "USD", "2024-02-10", "NULL", "150.00 USD", "-220.00 USD"],
        ]
    );
    assert_eq!(one("SELECT sum(position) WHERE account = 'Assets:Broker'"), "3 AAPL {110.00 USD, 2024-02-10}");
}

#[test]
fn price_and_weight_columns() {
    // @@ total price becomes a per-unit price
    assert_eq!(
        query("SELECT price, weight, cost_number, cost_label WHERE account = 'Expenses:Travel'")[0],
        vec!["1.1 USD", "110.000 USD", "NULL", ""]
    );
    assert_eq!(
        query("SELECT price, weight WHERE narration = 'lunch' AND account = 'Assets:Bank'")[0],
        vec!["NULL", "-12.50 USD"]
    );
}

#[test]
fn regex_operators() {
    assert_eq!(
        column("SELECT DISTINCT account WHERE account ~ 'expenses' ORDER BY account"),
        vec!["Expenses:Food", "Expenses:Travel"]
    );
    assert!(column("SELECT account WHERE account ?~ 'expenses'").is_empty());
    assert_eq!(column("SELECT DISTINCT account WHERE account ?~ 'Expenses:F'"), vec!["Expenses:Food"]);
    // NULL payee: neither ~ nor !~ match
    assert_eq!(one("SELECT count(*) WHERE payee ~ 'x' OR payee !~ 'x'"), "16");
    let err = error("SELECT * WHERE account ~ '('");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert_eq!(err.column, Some(26));
}

#[test]
fn in_operator_over_sets_and_lists() {
    assert_eq!(column("SELECT DISTINCT narration WHERE 'food' IN tags"), vec!["lunch"]);
    assert_eq!(column("SELECT DISTINCT narration WHERE 'receipt-1' IN links"), vec!["lunch"]);
    assert_eq!(one("SELECT count(*) WHERE 'food' NOT IN tags"), "16");
    assert_eq!(column("SELECT DISTINCT payee WHERE payee IN ('Trip')"), vec!["Trip"]);
    assert_eq!(
        column("SELECT DISTINCT payee WHERE payee IN ('Trip', 'Employer') ORDER BY payee"),
        vec!["Employer", "Trip"]
    );
    assert_eq!(one("SELECT count(*) WHERE payee NOT IN ('Trip', 'Employer')"), "12");
    assert_eq!(column("SELECT DISTINCT narration WHERE 'travel' IN (tags)"), vec!["hotel"]);
    assert_eq!(one("SELECT count(*) WHERE month IN (1, 2.0)"), "10");
}

#[test]
fn null_semantics() {
    assert_eq!(one("SELECT count(*) WHERE payee IS NULL"), "2");
    assert_eq!(one("SELECT count(*) WHERE payee IS NOT NULL"), "16");
    // comparisons with NULL are NULL, and NULL filters like FALSE
    assert_eq!(one("SELECT count(*) WHERE payee = 'Lunch' OR payee != 'Lunch'"), "16");
    assert_eq!(one("SELECT count(*) WHERE NOT (payee = 'x')"), "16");
    assert_eq!(
        query("SELECT NULL = 1, NULL IS NULL, TRUE AND NULL, FALSE AND NULL, TRUE OR NULL, FALSE OR NULL, NOT NULL LIMIT 1")[0],
        vec!["NULL", "TRUE", "NULL", "FALSE", "TRUE", "NULL", "NULL"]
    );
}

#[test]
fn arithmetic_and_literals() {
    assert_eq!(
        query("SELECT 1 + 2 * 3, 7 / 2, 1 / 3, 1 / 0, 10.00 / 4, -number, 'a' + 'b', 2024-01-31 + 1, 2024-03-01 - 2024-02-01 LIMIT 1")[0],
        vec![
            "7",
            "3.5",
            "0.3333333333333333333333333333",
            "NULL",
            "2.50",
            "-1000.00",
            "ab",
            "2024-02-01",
            "29"
        ]
    );
    // multiplication keeps the scale of its operands, also when one of them is 1
    assert_eq!(
        query("SELECT number * 1, 1 * number, number * 1.0, weight * 1 WHERE narration = 'January salary' AND account = 'Assets:Bank'")[0],
        vec!["1000.00", "1000.00", "1000.000", "1000.00 USD"]
    );
    assert_eq!(query("SELECT position LIMIT 0").len(), 0, "LIMIT 0 returns nothing");
    assert_eq!(one("SELECT count(*) WHERE date >= '2024-03-01'"), "8");
    assert_eq!(one("SELECT count(*) WHERE date >= 2024-03-01 AND date < 2024-04-01"), "6");
    let err = error("SELECT * WHERE date > 'March'");
    assert!(err.message.contains("not a valid date"), "{}", err);
    let err = error("SELECT 1 + 'a'");
    assert!(err.message.contains("not supported for (int, str)"), "{}", err);
}

#[test]
fn null_sorts_first_ascending_and_last_descending() {
    let ascending = column("SELECT DISTINCT payee ORDER BY payee");
    assert_eq!(ascending.first().map(String::as_str), Some("NULL"));
    let descending = column("SELECT DISTINCT payee ORDER BY payee DESC");
    assert_eq!(descending.last().map(String::as_str), Some("NULL"));
    assert_eq!(descending.first().map(String::as_str), Some("午餐"));
}

#[test]
fn order_by_multiple_keys_with_directions() {
    assert_eq!(
        query("SELECT account, number WHERE account ~ 'Expenses|Broker' ORDER BY account DESC, number ASC"),
        vec![
            vec!["Expenses:Travel", "100.00"],
            vec!["Expenses:Food", "7.50"],
            vec!["Expenses:Food", "12.50"],
            vec!["Assets:Broker", "-5"],
            vec!["Assets:Broker", "-2"],
            vec!["Assets:Broker", "5"],
            vec!["Assets:Broker", "5"],
        ]
    );
    // ORDER BY an expression that is not selected
    assert_eq!(
        column("SELECT narration WHERE account = 'Expenses:Food' ORDER BY number DESC"),
        vec!["lunch", "Lunch"]
    );
}

#[test]
fn distinct_and_limit() {
    assert_eq!(
        column("SELECT DISTINCT root(account, 1) ORDER BY 1"),
        vec!["Assets", "Equity", "Expenses", "Income"]
    );
    assert_eq!(column("SELECT DISTINCT root(account, 1) ORDER BY 1 LIMIT 2"), vec!["Assets", "Equity"]);
    assert_eq!(query("SELECT * LIMIT 3").len(), 3);
}

#[test]
fn wildcard_columns() {
    let result = Query::compile("SELECT * LIMIT 1").unwrap();
    assert_eq!(
        result.columns().iter().map(|it| it.name.as_str()).collect::<Vec<_>>(),
        vec!["date", "flag", "payee", "narration", "account", "position"]
    );
}

#[test]
fn group_by_index_alias_expression_and_implicit() {
    let expected = vec![vec!["Assets", "12"], vec!["Equity", "1"], vec!["Expenses", "3"], vec!["Income", "2"]];
    assert_eq!(query("SELECT root(account, 1), count(*) GROUP BY 1 ORDER BY 1"), expected);
    assert_eq!(query("SELECT root(account, 1) AS r, count(*) GROUP BY r ORDER BY r"), expected);
    assert_eq!(query("SELECT root(account, 1), count(*) GROUP BY root(account, 1) ORDER BY 1"), expected);
    // implicit GROUP BY of the non-aggregate targets
    assert_eq!(query("SELECT root(account, 1), count(*) ORDER BY 1"), expected);
    // GROUP BY an expression that is not selected
    assert_eq!(
        column("SELECT count(*) GROUP BY root(account, 1) ORDER BY count(*) DESC, 1"),
        vec!["12", "3", "2", "1"]
    );
    // constants need no GROUP BY
    assert_eq!(
        query("SELECT root(account, 1), count(*), 'x', 1 / 4 GROUP BY 1 ORDER BY 1")[0],
        vec!["Assets", "12", "x", "0.25"]
    );
    // a group key without aggregates de-duplicates
    assert_eq!(column("SELECT year GROUP BY year"), vec!["2024"]);
}

#[test]
fn grouping_errors() {
    let err = error("SELECT account, date, count(*) GROUP BY account");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert!(err.message.contains("'date' is missing"), "{}", err);
    assert_eq!((err.line, err.column), (Some(1), Some(17)));

    let err = error("SELECT account, count(*) GROUP BY account ORDER BY date");
    assert!(err.message.contains("'date' is missing"), "{}", err);
    assert_eq!(err.column, Some(52));

    let err = error("SELECT tags, count(*) GROUP BY tags");
    assert!(err.message.contains("cannot be grouped"), "{}", err);
    let err = error("SELECT account, count(*) GROUP BY 3");
    assert!(err.message.contains("out of range"), "{}", err);
    let err = error("SELECT account, count(*) GROUP BY 2");
    assert!(err.message.contains("aggregate"), "{}", err);
    let err = error("SELECT * WHERE count(*) > 1");
    assert!(err.message.contains("not allowed in WHERE"), "{}", err);
    let err = error("SELECT sum(count(*))");
    assert!(err.message.contains("not allowed in an aggregate argument"), "{}", err);
    let err = error("SELECT number + count(*)");
    assert!(err.message.contains("inside an aggregate"), "{}", err);
}

#[test]
fn aggregates() {
    assert_eq!(
        query(
            "SELECT count(*), count(payee), first(payee), last(payee), min(number), max(number), min(date), max(account), sum(1), sum(number) \
             WHERE account = 'Assets:Bank'"
        )[0],
        vec![
            "8",
            "7",
            "Employer",
            "Balance Pad",
            "-550.00",
            "1130.00",
            "2024-01-01",
            "Assets:Bank",
            "8",
            "2000.00"
        ]
    );
    // the weights of a balanced transaction cancel out into an empty inventory
    assert_eq!(one("SELECT sum(weight) WHERE narration = 'sell'"), "");
    assert_eq!(one("SELECT sum(position) WHERE account ~ '^Expenses'"), "100.00 EUR, 20.00 USD");
}

#[test]
#[should_panic(expected = "returned 0 rows")]
fn aggregates_over_zero_rows_return_zero_rows() {
    let rows = query("SELECT count(*) WHERE FALSE");
    panic!("returned {} rows", rows.len());
}

#[test]
fn from_is_anded_with_where() {
    assert_eq!(one("SELECT count(*) FROM month = 1 WHERE account ~ 'Food'"), "2");
    let err = error("SELECT * FROM postings");
    assert!(err.message.contains("FROM <expression>"), "{}", err);
}

#[test]
fn parameters() {
    let compiled = Query::compile_with_params(
        "SELECT count(*) WHERE account ~ $1 AND date >= :from",
        &ParamTypes::new().push(DataType::Str).bind("from", DataType::Date),
    )
    .unwrap();
    let run = |pattern: &str, from: (i32, u32, u32)| {
        let params = Params::new()
            .push(pattern)
            .bind("from", NaiveDate::from_ymd_opt(from.0, from.1, from.2).unwrap());
        compiled.execute_at(ledger(), &params, today()).unwrap().rows
    };
    assert_eq!(run("Food", (2024, 1, 1)), vec![vec![Value::Int(2)]]);
    assert_eq!(run("Food", (2024, 1, 6)), vec![vec![Value::Int(1)]]);

    let err = Query::compile("SELECT * WHERE account ~ $1").err().unwrap();
    assert_eq!(err.message, "parameter $1 is not bound");
    assert_eq!(err.column, Some(26));
    let err = compiled.execute_at(ledger(), &Params::new().push("Food"), today()).unwrap_err();
    assert!(err.message.contains(":from"), "{}", err);

    let result = zhang_query::execute_with_params(ledger(), "SELECT :n * 2", &Params::new().bind("n", 21)).unwrap();
    assert_eq!(result.rows[0][0], Value::Int(42));
}

#[test]
fn error_positions_count_characters() {
    let err = error("SELECT * WHERE payee = '午餐' AND nosuch = 1");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert_eq!(err.message, "unknown column 'nosuch'");
    assert_eq!((err.line, err.column), (Some(1), Some(33)));

    let err = error("SELECT\n  date,\n  '午餐' + 1");
    assert_eq!((err.line, err.column), (Some(3), Some(3)));

    let err = error("SELECT * WHERE payee = '午餐' AND x ~");
    assert_eq!(err.kind, QueryErrorKind::Parse);
    assert_eq!((err.line, err.column), (Some(1), Some(36)));

    let err = error("SELECT nosuch(1)");
    assert_eq!(err.message, "unknown function 'nosuch'");
    assert_eq!(err.column, Some(8));
    let err = error("SELECT root");
    assert!(err.message.contains("did you mean"), "{}", err);
}

#[test]
fn schema_lists_columns_and_functions() {
    let schema = zhang_query::schema();
    assert!(schema.columns.iter().any(|it| it.name == "position" && it.ty == DataType::Position));
    assert!(schema.functions.iter().any(|it| it.signature == "count(*) -> int"));
    assert!(schema.functions.iter().any(|it| it.signature == "sum(position) -> inventory"));
    assert!(schema.functions.iter().any(|it| it.signature == "first(any) -> any"));
    assert!(schema.functions.iter().all(|it| !it.description.is_empty()));
}
