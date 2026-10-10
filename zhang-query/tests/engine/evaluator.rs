//! Evaluator tests over a small ledger: row source, operators, NULL handling, grouping,
//! ordering, aggregates, parameters and error positions.

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
    LEDGER_CELL.get_or_init(|| zhang_testkit::ledger::load_text(LEDGER))
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
    // a transaction without strings has an empty narration, like in beancount
    let ledger = zhang_testkit::ledger::load_text("1970-01-01 open Assets:A\n1970-01-01 open Income:B\n2024-01-01 *\n  Assets:A 1 USD\n  Income:B\n");
    let result = Query::compile("SELECT payee, narration, description")
        .unwrap()
        .execute_at(&ledger, &Params::new(), today())
        .unwrap();
    assert_eq!(result.rows[0], vec![Value::Null, Value::from(""), Value::from("")]);
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

/// Rows are matched to their parsed directives by source file and offset: transactions at the
/// same offset of two files keep their own price annotations and metadata.
#[test]
fn directives_are_matched_by_file_and_offset() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let transaction = |name: &str, price: &str| format!("2024-01-01 * \"{name}\"\n  k: \"{name}\"\n  Assets:A  1 EUR @ {price} USD\n  Assets:B\n");
    let opens = "1970-01-01 open Assets:A\n1970-01-01 open Assets:B\n";
    std::fs::write(
        dir.join("main.zhang"),
        format!("{}{opens}include \"other.zhang\"\n", transaction("main", "1.10")),
    )
    .unwrap();
    std::fs::write(dir.join("other.zhang"), transaction("other", "2.20")).unwrap();
    let ledger = zhang_testkit::ledger::load_ledger(dir, "main.zhang");
    let result = Query::compile("SELECT narration, price, entry_meta('k') WHERE account = 'Assets:A' ORDER BY narration")
        .unwrap()
        .execute_at(&ledger, &Params::new(), today())
        .unwrap();
    let rows = result
        .rows
        .iter()
        .map(|row| row.iter().map(Value::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(rows, vec![vec!["main", "1.10 USD", "main"], vec!["other", "2.20 USD", "other"]]);
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
    // `?~` is case-sensitive and takes the pattern on the left, like beanquery
    assert!(column("SELECT account WHERE 'expenses' ?~ account").is_empty());
    assert_eq!(column("SELECT DISTINCT account WHERE 'Expenses:F' ?~ account"), vec!["Expenses:Food"]);
    assert!(column("SELECT account WHERE account ?~ 'Expenses'").is_empty());
    let err = error("SELECT * WHERE '(' ?~ account");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(16)));
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

/// One text-to-date rule: a string compared with a date reads exactly the texts `date(text)`
/// reads, as the same day, and a bare date literal is a day of the same calendar.
#[test]
fn a_string_compared_with_a_date_is_read_as_date_reads_it() {
    let texts = [
        "2024-01-05",
        "2024-1-5",
        "2024-01- 5",
        " 2024-01-05",
        "+2024-01-05",
        "2024-01-05 ",
        "24-01-05",
        "0024-01-05",
        "2024-02-30",
        "20240105",
    ];
    for text in texts {
        let by_function = query(&format!("SELECT date('{}') LIMIT 1", text))[0][0].clone();
        let source = format!("SELECT count(*) WHERE date = '{}'", text);
        match Query::compile(&source) {
            Ok(_) if by_function == "NULL" => panic!("{:?}: date() reads no date, but the comparison compiles", text),
            Ok(_) => {
                let expected = one(&format!("SELECT count(*) WHERE date = date('{}')", text));
                assert_eq!(one(&source), expected, "{:?}", text);
            }
            Err(err) => {
                assert_eq!(
                    by_function, "NULL",
                    "{:?}: date() reads {}, but the comparison fails: {}",
                    text, by_function, err
                );
                assert!(err.message.contains("not a valid date"), "{:?}: {}", text, err);
            }
        }
    }
    assert_eq!(one("SELECT count(*) WHERE date = '2024-1-5'"), "2");
    assert_eq!(one("SELECT count(*) WHERE date IN ('2024-1-5', '2024-01-10')"), "4");
    // a bare literal in the year 0 is no date, as date() reads it
    let err = error("SELECT count(*) WHERE date > 0000-01-01");
    assert!(err.message.contains("invalid date literal"), "{}", err);
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
fn aggregates_over_zero_rows_return_their_initial_values() {
    assert_eq!(
        query(
            "SELECT count(*), count(payee), sum(day), sum(number), sum(weight), sum(position), \
             first(payee), last(payee), min(number), max(date), first(balance), last(balance) \
             FROM #postings WHERE account = 'Assets:DoesNotExist'"
        ),
        vec![vec!["0", "0", "0", "0", "", "", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL"]]
    );
    assert_eq!(one("SELECT count(*) + 1 WHERE FALSE"), "1");
}

#[test]
fn empty_aggregate_groups_are_only_created_without_group_keys() {
    for sql in [
        "SELECT count(*) WHERE FALSE GROUP BY account",
        "SELECT account, count(*) WHERE FALSE",
        "SELECT 'constant', count(*) WHERE FALSE",
        "SELECT account WHERE FALSE",
        "SELECT first(balance) WHERE FALSE GROUP BY account HAVING count(*) = 0",
        "SELECT last(balance) WHERE FALSE GROUP BY account HAVING last(balance) IS NULL",
    ] {
        assert!(query(sql).is_empty(), "{}", sql);
    }
    let empty = zhang_testkit::ledger::load_text("");
    for table in ["postings", "transactions", "prices", "balances"] {
        let sql = format!("SELECT count(*) FROM #{table}");
        let result = Query::compile(&sql).unwrap().execute_at(&empty, &Params::new(), today()).unwrap();
        assert_eq!(result.rows, vec![vec![Value::Int(0)]], "{}", sql);
    }
}

#[test]
fn from_is_anded_with_where() {
    assert_eq!(one("SELECT count(*) FROM month = 1 WHERE account ~ 'Food'"), "2");
    // FROM names the default table explicitly, with or without '#'
    assert_eq!(query("SELECT * FROM postings"), query("SELECT *"));
    assert_eq!(
        query("SELECT * FROM #postings WHERE account ~ 'Food'"),
        query("SELECT * WHERE account ~ 'Food'")
    );
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
fn compiled_queries_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Query>();
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

/// Run `f` on a thread with the default 2 MiB stack, as tokio's blocking pool does.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .expect("the query thread crashed")
}

fn run_on_small_stack(sql: String, params: Params) -> Result<Vec<Vec<Value>>, QueryError> {
    on_small_stack(move || {
        let compiled = Query::compile_with_params(&sql, &params.types())?;
        Ok(compiled.execute_at(ledger(), &params, today())?.rows)
    })
}

#[test]
fn deeply_nested_queries_are_rejected_without_crashing() {
    let max = zhang_query::MAX_DEPTH;
    for sql in [
        format!("SELECT {}1{}", "(".repeat(10_000), ")".repeat(10_000)),
        format!("SELECT count(*) WHERE {}TRUE", "NOT ".repeat(5_000)),
        format!("SELECT {}1", "- ".repeat(10_000)),
        format!("SELECT {}1{}", "root(".repeat(5_000), ")".repeat(5_000)),
        format!("SELECT 1 IN {}1{}", "(".repeat(5_000), ")".repeat(5_000)),
        format!("SELECT {}1{}", "(".repeat(max + 1), ")".repeat(max + 1)),
    ] {
        let err = run_on_small_stack(sql.clone(), Params::new()).expect_err(&sql[..40]);
        assert_eq!(err.kind, QueryErrorKind::Parse, "{}", err);
        assert!(err.message.contains("nested too deeply"), "{}", err);
        assert!(err.line.is_some() && err.column.is_some());
    }
}

#[test]
fn long_dotted_names_are_rejected_without_crashing() {
    let max = zhang_query::MAX_NAME_PARTS;
    for dots in [max, 10_000, 32_000] {
        let sql = format!("SELECT a{} FROM #accounts", ".a".repeat(dots));
        assert!(sql.len() <= zhang_query::MAX_QUERY_LENGTH);
        let err = run_on_small_stack(sql, Params::new()).unwrap_err();
        assert_eq!(err.kind, QueryErrorKind::Parse, "{}", err);
        // at the dot that starts the part after the last accepted one
        assert_eq!((err.line, err.column), (Some(1), Some(8 + 2 * max - 1)), "{}", err);
        assert!(err.message.contains("at most 8 parts"), "{}", err);
    }
    // the most parts a name may have is an unknown column, not a crash
    let sql = format!("SELECT a{} FROM #accounts", ".a".repeat(max - 1));
    let err = run_on_small_stack(sql, Params::new()).unwrap_err();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(8)), "{}", err);
}

#[test]
fn nesting_up_to_the_limit_still_runs_on_a_small_stack() {
    let max = zhang_query::MAX_DEPTH;
    let depth = max - 8;
    let rows = run_on_small_stack(format!("SELECT {}1{} LIMIT 1", "(".repeat(depth), ")".repeat(depth)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(1)]]);
    let rows = run_on_small_stack(format!("SELECT count(*) WHERE {}TRUE", "NOT NOT ".repeat(depth / 2)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(18)]]);
    let rows = run_on_small_stack(format!("SELECT 0{} LIMIT 1", " - 1".repeat(depth)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(-(depth as i64))]]);
    let rows = run_on_small_stack(
        format!("SELECT {}account{} LIMIT 1", "root(".repeat(depth), ", 9)".repeat(depth)),
        Params::new(),
    )
    .unwrap();
    assert_eq!(rows, vec![vec![Value::from("Assets:Bank")]]);
}

#[test]
fn long_operator_chains_are_flat() {
    // 10,000 chained ORs: a balanced tree, evaluated on a small stack
    let sql = format!("SELECT count(*) WHERE {}account = 'Assets:Bank'", ":n OR ".repeat(9_999));
    assert!(sql.len() < zhang_query::MAX_QUERY_LENGTH);
    let rows = run_on_small_stack(sql, Params::new().bind("n", false)).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(8)]]);

    // a realistic list of accounts
    let accounts = (0..2_000).map(|idx| format!("account = 'Expenses:X{}'", idx)).collect::<Vec<_>>().join(" OR ");
    let rows = run_on_small_stack(format!("SELECT count(*) WHERE {} OR account = 'Expenses:Food'", accounts), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(2)]]);

    let rows = run_on_small_stack(format!("SELECT 0{} LIMIT 1", " + 1".repeat(10_000)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(10_000)]]);
    // non-associative chains are flat too, and evaluate left to right
    let rows = run_on_small_stack(format!("SELECT 1{} LIMIT 1", " - 1".repeat(10_000)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(-9_999)]]);
    let rows = run_on_small_stack(format!("SELECT 2{} LIMIT 1", " * 2 / 2".repeat(5_000)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Decimal(2.into())]]);
    let rows = run_on_small_stack(format!("SELECT count(*) WHERE TRUE{}", " AND TRUE".repeat(5_000)), Params::new()).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(18)]]);
}

#[test]
fn overlong_queries_are_rejected() {
    let sql = format!("SELECT count(*) WHERE {}TRUE", "TRUE OR ".repeat(9_000));
    let err = run_on_small_stack(sql, Params::new()).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Parse);
    assert!(err.message.contains("too long"), "{}", err);
}

#[test]
fn constant_patterns_and_parameters_compile_their_regex_once() {
    // folded at compile time: a static regex
    assert_eq!(one("SELECT count(*) WHERE account ~ ('^Expenses' + ':Food')"), "2");
    // invalid constant patterns are reported at compile time, at the pattern
    let err = error("SELECT * WHERE account ~ ('(' + '')");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    // a bound parameter is compiled once per execution
    let rows = run_on_small_stack("SELECT count(*) WHERE account ~ :p".to_owned(), Params::new().bind("p", "^Expenses")).unwrap();
    assert_eq!(rows, vec![vec![Value::Int(3)]]);
    // per-row patterns work and are cached
    assert_eq!(one("SELECT count(*) WHERE account ~ (root(account, 1) + ':')"), "18");
    // oversized patterns are rejected instead of compiled
    let err = try_query("SELECT count(*) WHERE account ~ (narration + 'x{1000}{1000}')").unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Eval);
    assert!(err.message.contains("regular expression"), "{}", err);
    let err = error("SELECT count(*) WHERE account ~ 'x{1000}{1000}'");
    assert!(err.message.contains("size limit"), "{}", err);
}

#[test]
fn executions_stop_at_their_deadline() {
    let compiled = Query::compile("SELECT account, count(*) GROUP BY account").unwrap();
    let options = zhang_query::ExecuteOptions {
        today: Some(today()),
        timeout: Some(std::time::Duration::ZERO),
        ..Default::default()
    };
    let err = compiled.execute_with_options(ledger(), &Params::new(), &options).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Timeout);
    assert!(err.message.contains("time limit"), "{}", err);
    assert_eq!((err.line, err.column), (None, None));

    let options = zhang_query::ExecuteOptions {
        today: Some(today()),
        timeout: Some(std::time::Duration::from_secs(60)),
        ..Default::default()
    };
    assert_eq!(compiled.execute_with_options(ledger(), &Params::new(), &options).unwrap().rows.len(), 7);
}

#[test]
fn metadata_functions_are_row_dependent_when_grouping() {
    let err = error("SELECT account, entry_meta('category'), count(*) GROUP BY account");
    assert!(err.message.contains("is missing"), "{}", err);
    assert_eq!(err.column, Some(17));
    let err = error("SELECT entry_meta('category'), count(*) GROUP BY account");
    assert!(err.message.contains("is missing"), "{}", err);
    assert_eq!(
        query("SELECT entry_meta('category') AS c, count(*) GROUP BY c ORDER BY c"),
        vec![vec!["NULL", "16"], vec!["meal", "2"]]
    );
    assert_eq!(one("SELECT count(entry_meta('category'))"), "2");
}

#[test]
fn explain_shows_the_optimized_plan() {
    let query =
        Query::compile("SELECT account, number * 2 AS double FROM year = 2024 WHERE account ~ ('^Ex' + 'penses') AND NOT NOT TRUE ORDER BY 2 DESC LIMIT 5")
            .unwrap();
    assert_eq!(
        query.explain(),
        "target 0: account = account : str\n\
         target 1: double = (number * 2) : decimal\n\
         filter: ((year = 2024) AND (account ~ /^Expenses/i))\n\
         order by: 1 DESC\n\
         limit: 5 (top-k while scanning)\n\
         late targets: [0] (built for the kept rows only)\n\
         project: [account, number, year] (3 of 36 columns)\n"
    );
    let grouped = Query::compile("SELECT root(account, 1) AS r, sum(position) WHERE TRUE OR payee IS NULL GROUP BY r").unwrap();
    assert_eq!(
        grouped.explain(),
        "target 0: r = root(account, 1) : str\n\
         target 1: sum(position) = agg#0 : inventory\n\
         agg#0: sum(position)\n\
         group by: [0]\n\
         project: [account, position] (2 of 36 columns)\n"
    );
    // a thousand ORs are one flat node
    let sql = format!("SELECT count(*) WHERE {}account = 'x'", "account = 'y' OR ".repeat(999));
    let explain = Query::compile(&sql).unwrap().explain();
    assert_eq!(explain.matches(" OR ").count(), 999);
    assert!(explain.contains("filter: ((account = 'y') OR "));
}

#[test]
fn explain_shows_the_projected_columns() {
    let projected = |sql: &str| {
        let explain = Query::compile(sql).unwrap().explain();
        explain.lines().last().unwrap().to_owned()
    };
    assert_eq!(projected("SELECT count(*)"), "project: [] (0 of 36 columns)");
    assert_eq!(
        projected("SELECT *"),
        "project: [account, date, flag, narration, payee, position] (6 of 36 columns)"
    );
    // the filter, GROUP BY / ORDER BY keys and aggregate arguments are projected too
    assert_eq!(
        projected("SELECT payee, count(*) FROM year = 2024 WHERE 'x' IN tags GROUP BY payee, month ORDER BY max(cost_date)"),
        "project: [cost_date, month, payee, tags, year] (5 of 36 columns)"
    );
    // a column only an optimized-away filter read is pruned
    assert_eq!(projected("SELECT account WHERE TRUE OR payee IS NULL"), "project: [account] (1 of 36 columns)");
    let query = Query::compile("SELECT date, sum(weight) WHERE account ~ 'Expenses' GROUP BY date").unwrap();
    assert_eq!(
        query.explain().lines().last().unwrap(),
        format!(
            "project: [{}] ({} of 36 columns)",
            query.referenced_columns().join(", "),
            query.referenced_columns().len()
        )
    );
}

#[test]
fn plans_expose_the_columns_they_read() {
    let query = Query::compile("SELECT account, sum(position) WHERE year = 2024 AND 'x' IN tags GROUP BY account ORDER BY count(*)").unwrap();
    assert_eq!(query.referenced_columns(), vec!["account", "position", "tags", "year"]);
    let query = Query::compile("SELECT count(*)").unwrap();
    assert!(query.referenced_columns().is_empty());
}
