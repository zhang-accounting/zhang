//! The BALANCES and JOURNAL statements, the `AT` clause and the running `balance` column.
//!
//! The oracle tests compare whole results with beanquery's over
//! `integration-tests/fava-demo-ledger` (`tests/golden/statements.json`, regenerated with
//! `tests/golden/statements.py`). Numbers are compared numerically and inventory positions as
//! sets; column types must match exactly and column names up to the spelling documented in
//! [`zhang_name`].

mod common;

use std::str::FromStr;
use std::sync::OnceLock;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Params, Position, Query, QueryError, QueryErrorKind, Value};

fn fava_demo() -> &'static Ledger {
    static LEDGER: OnceLock<Ledger> = OnceLock::new();
    LEDGER.get_or_init(common::fava_demo_ledger)
}

fn decimal_json(value: &BigDecimal) -> Json {
    Json::String(zhang_query::decimal::to_plain_string(value))
}

fn position_json(position: &Position) -> Json {
    json!({
        "units": {"number": decimal_json(&position.units.number), "currency": position.units.commodity},
        "cost": position.cost.as_ref().map(|cost| json!({
            "number": decimal_json(&cost.number),
            "currency": cost.currency,
            "date": cost.date.map(|date| date.to_string()),
            "label": cost.label,
        })),
    })
}

/// The `POST /api/query` cell encoding.
fn cell_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(it) => json!(it),
        Value::Int(it) => json!(it),
        Value::Decimal(it) => decimal_json(it),
        Value::Str(it) => json!(it),
        Value::Date(it) => json!(it.to_string()),
        Value::Set(it) => json!(it),
        Value::Amount(it) => json!({"number": decimal_json(&it.number), "currency": it.commodity}),
        Value::Position(it) => position_json(it),
        Value::Inventory(it) => json!({ "positions": it.positions().map(|it| position_json(&it)).collect::<Vec<_>>() }),
    }
}

/// A form in which equal results are equal JSON: numbers are normalized (`4.00` = `4`) and
/// the positions of an inventory are sorted. Applied to both sides alike.
fn canonical(json: &Json) -> Json {
    match json {
        Json::String(text) => match BigDecimal::from_str(text) {
            Ok(number) => Json::String(number.normalized().to_string()),
            Err(_) => json.clone(),
        },
        Json::Array(items) => Json::Array(items.iter().map(canonical).collect()),
        Json::Object(object) if object.contains_key("positions") => {
            let mut positions = object["positions"].as_array().unwrap().iter().map(canonical).collect::<Vec<_>>();
            positions.sort_by_key(|it| it.to_string());
            json!({ "positions": positions })
        }
        Json::Object(object) => Json::Object(object.iter().map(|(key, value)| (key.clone(), canonical(value))).collect()),
        other => other.clone(),
    }
}

/// The name zhang gives a column beanquery calls `name`: zhang names the targets of BALANCES
/// and JOURNAL like the equivalent hand-written SELECT, in lower case and without the empty
/// `AT` function (beanquery: `SUM((position))`, `MAXWIDTH(payee, 48)`).
fn zhang_name(name: &str) -> String {
    name.to_lowercase().replace("((position))", "(position)")
}

fn oracle_case(index: usize) {
    let cases: Vec<Json> = serde_json::from_str(include_str!("golden/statements.json")).unwrap();
    let case = &cases[index];
    let sql = case["query"].as_str().unwrap();
    let query = Query::compile(sql).unwrap_or_else(|err| panic!("{}: {}", sql, err));
    let result = query.execute(fava_demo(), &Params::new()).unwrap_or_else(|err| panic!("{}: {}", sql, err));

    let expected_columns = case["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (zhang_name(it["name"].as_str().unwrap()), it["type"].as_str().unwrap().to_owned()))
        .collect::<Vec<_>>();
    let columns = result.columns.iter().map(|it| (it.name.clone(), it.ty.name().to_owned())).collect::<Vec<_>>();
    assert_eq!(columns, expected_columns, "columns of {}", sql);

    let expected = case["rows"].as_array().unwrap();
    assert_eq!(result.rows.len(), expected.len(), "row count of {}", sql);
    for (idx, (expected, actual)) in expected.iter().zip(&result.rows).enumerate() {
        let actual = Json::Array(actual.iter().map(cell_json).collect());
        assert_eq!(canonical(&actual), canonical(expected), "{}: row {}", sql, idx);
    }
}

#[test]
fn oracle_balances() {
    oracle_case(0);
}

#[test]
fn oracle_balances_at_cost_from_where() {
    oracle_case(1);
}

#[test]
fn oracle_balances_at_value() {
    oracle_case(2);
}

#[test]
fn oracle_journal_from() {
    oracle_case(3);
}

#[test]
fn oracle_journal_collapses_whitespace_like_maxwidth() {
    oracle_case(4);
}

#[test]
fn oracle_journal_at_cost_over_lots() {
    oracle_case(5);
}

#[test]
fn oracle_journal_at_units_double_quoted() {
    oracle_case(6);
}

#[test]
fn oracle_balance_is_accumulated_before_order_by() {
    oracle_case(7);
}

#[test]
fn oracle_balance_in_an_aggregate() {
    oracle_case(8);
}

#[test]
fn explain_shows_the_desugared_statements() {
    let balances = Query::compile("BALANCES AT cost FROM year = 2016 WHERE account ~ '^Assets'").unwrap();
    assert_eq!(
        balances.explain(),
        "target 0: account = account : str\n\
         target 1: sum(cost(position)) = agg#0 : inventory\n\
         target 2: account_sortkey(account) = account_sortkey(account) : str (hidden)\n\
         agg#0: sum(cost(position))\n\
         filter: ((year = 2016) AND (account ~ /^Assets/i))\n\
         group by: [0, 2]\n\
         order by: 2 ASC\n\
         project: [account, position, year] (3 of 31 columns)\n"
    );
    let journal = Query::compile("JOURNAL 'Checking' AT units FROM year = 2016").unwrap();
    assert_eq!(
        journal.explain(),
        "target 0: date = date : date\n\
         target 1: flag = flag : str\n\
         target 2: maxwidth(payee, 48) = maxwidth(payee, 48) : str\n\
         target 3: maxwidth(narration, 80) = maxwidth(narration, 80) : str\n\
         target 4: account = account : str\n\
         target 5: units(position) = units(position) : amount\n\
         target 6: units(balance) = units(balance) : inventory\n\
         filter: ((year = 2016) AND (account ~ /Checking/i))\n\
         rewrite: units(balance) -> running units\n\
         balance: deferred targets [6]\n\
         project: [account, balance, date, flag, narration, payee, position, year] (8 of 31 columns)\n"
    );
}

/// The statements return exactly what the SELECT they stand for returns.
#[test]
fn statements_equal_their_select() {
    let pairs = [
        (
            "BALANCES",
            "SELECT account, sum(position) GROUP BY account, account_sortkey(account) ORDER BY account_sortkey(account)",
        ),
        (
            "balances at units from year = 2016 where account ~ 'Expenses';",
            "SELECT account, sum(units(position)) FROM year = 2016 WHERE account ~ 'Expenses' \
             GROUP BY account, account_sortkey(account) ORDER BY account_sortkey(account)",
        ),
        (
            "JOURNAL 'Cash' FROM month = 3",
            "SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account, position, balance \
             FROM month = 3 WHERE account ~ 'Cash'",
        ),
        (
            "JOURNAL AT value FROM year = 2017 AND account ~ 'GLD'",
            "SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account, value(position), value(balance) \
             FROM year = 2017 AND account ~ 'GLD'",
        ),
    ];
    let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for (statement, select) in pairs {
        let run = |sql: &str| {
            let query = Query::compile(sql).unwrap_or_else(|err| panic!("{}: {}", sql, err));
            let result = query.execute_at(fava_demo(), &Params::new(), today).unwrap();
            (result.columns, result.rows)
        };
        let (statement_columns, statement_rows) = run(statement);
        let (select_columns, select_rows) = run(select);
        assert!(!statement_rows.is_empty(), "{}", statement);
        assert_eq!(statement_rows, select_rows, "{}", statement);
        assert_eq!(statement_columns, select_columns, "{}", statement);
    }
}

const LEDGER: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Assets:Broker
1970-01-01 open Liabilities:Card
1970-01-01 open Expenses:Food
1970-01-01 open Income:Salary
1970-01-01 open Equity:Opening

2024-03-01 price AAPL 150.00 USD

2024-01-01 * "Employer" "January salary"
  Assets:Bank    1000.00 USD
  Income:Salary

2024-01-05 * "Cafe"    "lunch   with   a   long   narration"
  Expenses:Food    12.50 USD
  Liabilities:Card

2024-02-01 * "Broker" "buy"
  Assets:Broker     5 AAPL {100.00 USD}
  Assets:Bank    -500.00 USD

2024-02-10 * "Broker" "buy more"
  Assets:Broker     5 AAPL {110.00 USD}
  Assets:Bank    -550.00 USD

2024-03-05 * "Broker" "sell"
  Assets:Broker    -7 AAPL {} @ 150.00 USD
  Assets:Bank    1050.00 USD
  Income:Salary  -330.00 USD

2024-04-02 balance Assets:Bank 1500.00 USD with pad Equity:Opening
"#;

fn ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| common::load_text(LEDGER))
}

fn try_rows(sql: &str) -> Result<Vec<Vec<String>>, QueryError> {
    let result = Query::compile(sql)?.execute_at(ledger(), &Params::new(), NaiveDate::from_ymd_opt(2024, 6, 30).unwrap())?;
    Ok(result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| match value {
                    Value::Inventory(inventory) => format!("({})", inventory),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect())
}

fn rows(sql: &str) -> Vec<Vec<String>> {
    try_rows(sql).unwrap_or_else(|err| panic!("{}: {}", sql, err))
}

fn error(sql: &str) -> QueryError {
    match try_rows(sql) {
        Ok(rows) => panic!("{} should fail but returned {:?}", sql, rows),
        Err(err) => err,
    }
}

#[test]
fn balances_orders_accounts_by_type_then_name() {
    assert_eq!(
        rows("BALANCES"),
        vec![
            vec!["Assets:Bank", "(1500.00 USD)"],
            vec!["Assets:Broker", "(3 AAPL {110.00 USD, 2024-02-10})"],
            vec!["Liabilities:Card", "(-12.50 USD)"],
            vec!["Equity:Opening", "(-500.00 USD)"],
            vec!["Income:Salary", "(-1330.00 USD)"],
            vec!["Expenses:Food", "(12.50 USD)"],
        ]
    );
    assert_eq!(rows("BALANCES AT cost WHERE account ~ 'Broker'"), vec![vec!["Assets:Broker", "(330.00 USD)"]]);
    assert_eq!(
        rows("BALANCES AT value FROM date < 2024-03-01 WHERE account ~ 'Broker'"),
        vec![vec!["Assets:Broker", "(1500.00 USD)"]]
    );
    // the AT function is applied to every position, then summed
    assert_eq!(rows("BALANCES AT units WHERE account ~ 'Broker'"), vec![vec!["Assets:Broker", "(3 AAPL)"]]);
}

#[test]
fn journal_lists_postings_with_their_running_balance() {
    assert_eq!(
        rows("JOURNAL 'Bank'"),
        vec![
            vec!["2024-01-01", "*", "Employer", "January salary", "Assets:Bank", "1000.00 USD", "(1000.00 USD)"],
            vec!["2024-02-01", "*", "Broker", "buy", "Assets:Bank", "-500.00 USD", "(500.00 USD)"],
            vec!["2024-02-10", "*", "Broker", "buy more", "Assets:Bank", "-550.00 USD", "(-50.00 USD)"],
            vec!["2024-03-05", "*", "Broker", "sell", "Assets:Bank", "1050.00 USD", "(1000.00 USD)"],
            vec![
                "2024-04-02",
                "P",
                "Balance Pad",
                "pad Assets:Bank to Equity:Opening",
                "Assets:Bank",
                "500.00 USD",
                "(1500.00 USD)"
            ],
        ]
    );
    // the balance sums every account the pattern matches, lots included
    let broker = rows("JOURNAL 'broker' AT cost");
    assert_eq!(
        broker.iter().map(|row| (row[5].as_str(), row[6].as_str())).collect::<Vec<_>>(),
        vec![
            ("500.00 USD", "(500.00 USD)"),
            ("550.00 USD", "(1050.00 USD)"),
            ("-500.00 USD", "(550.00 USD)"),
            ("-220.00 USD", "(330.00 USD)"),
        ]
    );
    // payee and narration go through maxwidth(): whitespace is collapsed, long text is cut
    assert_eq!(rows("JOURNAL 'Food'")[0][3], "lunch with a long narration");
    assert_eq!(rows("SELECT maxwidth(narration, 20) WHERE account ~ 'Food'"), vec![vec!["lunch with a [...]"]]);
    // FROM filters the postings; the pattern can be a parameter
    let query = Query::compile_with_params(
        "JOURNAL :account FROM year = 2024 AND month = 2",
        &zhang_query::ParamTypes::new().bind("account", DataType::Str),
    )
    .unwrap();
    let result = query
        .execute_at(
            ledger(),
            &Params::new().bind("account", "^Assets:Bank$"),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
        )
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    // every posting; the sale reduces two lots, so it is split into two rows
    assert_eq!(rows("JOURNAL").len(), 14);
    assert_eq!(rows("journal \"food\";").len(), 1);
}

#[test]
fn balance_column_in_select() {
    // the rows WHERE selects, in ledger order
    assert_eq!(
        rows("SELECT date, position, balance WHERE account ~ 'Bank'"),
        vec![
            vec!["2024-01-01", "1000.00 USD", "(1000.00 USD)"],
            vec!["2024-02-01", "-500.00 USD", "(500.00 USD)"],
            vec!["2024-02-10", "-550.00 USD", "(-50.00 USD)"],
            vec!["2024-03-05", "1050.00 USD", "(1000.00 USD)"],
            vec!["2024-04-02", "500.00 USD", "(1500.00 USD)"],
        ]
    );
    // accumulated before ORDER BY, DISTINCT and LIMIT, as in beanquery
    assert_eq!(
        rows("SELECT date, balance WHERE account ~ 'Bank' ORDER BY date DESC LIMIT 2"),
        vec![vec!["2024-04-02", "(1500.00 USD)"], vec!["2024-03-05", "(1000.00 USD)"]]
    );
    assert_eq!(
        rows("SELECT date, balance WHERE account ~ 'Bank' LIMIT 2"),
        vec![vec!["2024-01-01", "(1000.00 USD)"], vec!["2024-02-01", "(500.00 USD)"]]
    );
    // FROM filters like WHERE; lots that net to zero leave the inventory
    assert_eq!(
        rows("SELECT balance FROM month >= 2 WHERE account ~ 'Bank'"),
        vec![vec!["(-500.00 USD)"], vec!["(-1050.00 USD)"], vec!["()"], vec!["(500.00 USD)"]]
    );
    // one balance per row, however often it is read
    assert_eq!(
        rows("SELECT balance, units(balance), str(balance) WHERE account ~ 'Food'"),
        vec![vec!["(12.50 USD)", "(12.50 USD)", "(12.50 USD)"]]
    );
    // aggregates see the running balance of every row that passed the filter, across groups
    assert_eq!(rows("SELECT last(balance), count(*) WHERE account ~ 'Bank'"), vec![vec!["(1500.00 USD)", "5"]]);
    assert_eq!(
        rows("SELECT account, last(balance) WHERE account ~ 'Assets' GROUP BY account ORDER BY account"),
        vec![
            vec!["Assets:Bank", "(3 AAPL {110.00 USD, 2024-02-10}, 1500.00 USD)"],
            vec!["Assets:Broker", "(3 AAPL {110.00 USD, 2024-02-10}, -50.00 USD)"],
        ]
    );
}

#[test]
fn balance_is_never_folded_or_filtered_on() {
    let query = Query::compile("SELECT units(balance), balance WHERE account ~ 'Bank'").unwrap();
    assert!(
        query.explain().contains("target 0: units(balance) = units(balance) : inventory"),
        "{}",
        query.explain()
    );
    assert_eq!(query.referenced_columns(), vec!["account", "balance"]);
    let schema = zhang_query::schema();
    assert!(schema.columns.iter().any(|it| it.name == "balance" && it.ty == DataType::Inventory));

    for (sql, column) in [
        ("SELECT date WHERE balance IS NOT NULL", 19),
        ("SELECT date FROM str(balance) = '' WHERE TRUE", 22),
        ("BALANCES WHERE units(balance) IS NULL", 22),
    ] {
        let err = error(sql);
        assert_eq!(err.kind, QueryErrorKind::Compile, "{}", sql);
        assert!(err.message.contains("balance cannot be used in"), "{}: {}", sql, err);
        assert_eq!(err.column, Some(column), "{}", sql);
    }
    let err = error("SELECT account, balance, count(*) GROUP BY account");
    assert!(err.message.contains("'balance' is missing"), "{}", err);
}

#[test]
fn statement_errors_point_at_what_was_written() {
    let err = error("BALANCES AT nosuch");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(13)));
    assert!(err.message.contains("unknown function 'nosuch'"), "{}", err);
    // an AT function sum() cannot add up
    let err = error("BALANCES AT str");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(13)));
    let err = error("JOURNAL 'x' AT year");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(16)));
    let err = error("JOURNAL '(' ");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(9)));
    assert!(err.message.contains("regular expression"), "{}", err);

    for (sql, column, message) in [
        ("BALANCES AT", 12, "expected a function name after AT"),
        ("BALANCES AT WHERE TRUE", 13, "expected a function name after AT"),
        ("JOURNAL 'x' WHERE TRUE", 13, "JOURNAL has no WHERE clause"),
        ("BALANCES ORDER BY account", 10, "unexpected 'ORDER'"),
        ("JOURNAL account", 9, "unexpected 'account'"),
        ("BALANCES FROM OPEN ON 2024", 23, "expected a date after OPEN ON"),
        ("JOURNAL 'x' FROM CLEAR OPEN ON 2024-01-01", 24, "in this order"),
        ("PRINT", 1, "PRINT statements are not supported yet"),
        ("UPDATE x", 1, "expected SELECT, BALANCES or JOURNAL"),
    ] {
        let err = error(sql);
        assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(column)), "{}: {}", sql, err);
        assert!(err.message.contains(message), "{}: {}", sql, err);
    }
}
