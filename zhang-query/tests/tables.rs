//! `FROM #table`: the table registry, name resolution, per-table columns, the projector,
//! `explain()`, CSV export and the schema.

mod common;

use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, Params, Query, QueryError, QueryErrorKind, Value};

const LEDGER: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity EUR
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
1970-01-01 open Equity:Opening

2024-01-01 price EUR 1.10 USD
  source: "ecb"
2024-02-15 price AAPL 120.00 USD
2024-03-01 price AAPL 150.00 USD
2024-03-01 price EUR 1.08 USD

2024-01-05 * "Cafe" "lunch"
  Expenses:Food    12.50 USD
  Assets:Bank
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

fn error(sql: &str) -> QueryError {
    match try_query(sql) {
        Ok(rows) => panic!("{} should fail but returned {:?}", sql, rows),
        Err(err) => err,
    }
}

#[test]
fn prices_rows_come_in_ledger_order() {
    assert_eq!(
        query("SELECT * FROM #prices"),
        vec![
            vec!["2024-01-01", "EUR", "1.10 USD"],
            vec!["2024-02-15", "AAPL", "120.00 USD"],
            vec!["2024-03-01", "AAPL", "150.00 USD"],
            vec!["2024-03-01", "EUR", "1.08 USD"],
        ]
    );
    let columns = Query::compile("SELECT * FROM #prices").unwrap().columns();
    let columns = columns.iter().map(|column| (column.name.as_str(), column.ty)).collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec![("date", DataType::Date), ("currency", DataType::Str), ("amount", DataType::Amount)]
    );
}

#[test]
fn table_queries_filter_group_order_and_limit() {
    assert_eq!(
        query("SELECT currency, count(*) AS n, last(amount), max(number(amount)) FROM #prices WHERE date >= 2024-01-01 GROUP BY currency ORDER BY n DESC, currency"),
        vec![vec!["AAPL", "2", "150.00 USD", "150.00"], vec!["EUR", "2", "1.08 USD", "1.10"]]
    );
    assert_eq!(query("SELECT DISTINCT currency FROM prices ORDER BY currency DESC LIMIT 1"), vec![vec!["EUR"]]);
    // functions work on the columns of any table, by type
    assert_eq!(
        query("SELECT year(date), currency(amount), amount * 2 FROM #prices WHERE currency = 'EUR' AND month(date) = 3"),
        vec![vec!["2024", "USD", "2.16 USD"]]
    );
    // metadata functions read the row's own directive
    assert_eq!(
        query("SELECT currency, meta('source'), entry_meta('source'), any_meta('source') FROM #prices WHERE meta('source') IS NOT NULL"),
        vec![vec!["EUR", "ecb", "ecb", "ecb"]]
    );
    // the postings table can be named explicitly
    assert_eq!(query("SELECT count(*) FROM #postings"), query("SELECT count(*)"));
}

#[test]
fn table_names_are_case_sensitive_and_resolved_at_compile_time() {
    let err = error("SELECT * FROM #nosuch WHERE TRUE");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert_eq!((err.line, err.column), (Some(1), Some(15)));
    assert!(
        err.message.starts_with("unknown table '#nosuch'; the tables are #postings, "),
        "{}",
        err.message
    );
    let err = error("SELECT * FROM #Prices");
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert!(err.message.contains("unknown table '#Prices'"), "{}", err.message);
    // a bare name that is not a postings column names a table
    let err = error("SELECT * FROM nosuch");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(15)));
    assert!(err.message.contains("unknown column or table 'nosuch'"), "{}", err.message);
}

#[test]
fn columns_belong_to_their_table() {
    let err = error("SELECT account FROM #prices");
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(8)));
    assert_eq!(err.message, "unknown column 'account' in #prices (its columns are date, currency, amount)");
    let err = error("SELECT * FROM #prices WHERE balance IS NULL");
    assert!(err.message.contains("unknown column 'balance' in #prices"), "{}", err.message);
    // the postings table keeps its own message
    assert_eq!(error("SELECT amount").message, "unknown column 'amount'");
}

#[test]
fn explain_names_the_table_and_projects_its_columns() {
    let query = Query::compile("SELECT currency, count(*) FROM #prices WHERE year(date) = 2024 GROUP BY currency").unwrap();
    assert_eq!(query.table(), "prices");
    assert_eq!(query.referenced_columns(), vec!["currency", "date"]);
    assert_eq!(
        query.explain(),
        "table: #prices\n\
         target 0: currency = currency : str\n\
         target 1: count(*) = agg#0 : int\n\
         agg#0: count(*)\n\
         filter: (year(date) = 2024)\n\
         group by: [0]\n\
         project: [currency, date] (2 of 3 columns)\n"
    );
    // the default table is not named
    let postings = Query::compile("SELECT count(*) FROM #postings").unwrap();
    assert_eq!(postings.table(), "postings");
    assert!(!postings.explain().contains("table:"), "{}", postings.explain());
}

#[test]
fn table_results_respect_the_budget_and_export_to_csv() {
    let compiled = Query::compile("SELECT date, currency, amount FROM #prices ORDER BY date, currency").unwrap();
    let options = |limit| ExecuteOptions {
        today: Some(today()),
        timeout: None,
        max_result_values: Some(limit),
    };
    // 4 rows of 3 values
    assert!(compiled.execute_with_options(ledger(), &Params::new(), &options(12)).is_ok());
    let err = compiled.execute_with_options(ledger(), &Params::new(), &options(11)).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge);

    let result = compiled.execute_at(ledger(), &Params::new(), today()).unwrap();
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "date,currency,amount (USD)\r\n\
         2024-01-01,EUR,1.10\r\n\
         2024-02-15,AAPL,120.00\r\n\
         2024-03-01,AAPL,150.00\r\n\
         2024-03-01,EUR,1.08\r\n"
    );
}

#[test]
fn the_schema_describes_every_table() {
    let schema = zhang_query::schema();
    assert_eq!(schema.tables[0].name, "postings");
    assert_eq!(schema.tables[0].columns, schema.columns);
    let prices = schema.tables.iter().find(|table| table.name == "prices").unwrap();
    let columns = prices.columns.iter().map(|column| (column.name, column.ty)).collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec![("date", DataType::Date), ("currency", DataType::Str), ("amount", DataType::Amount)]
    );
    for table in &schema.tables {
        assert!(!table.description.is_empty(), "{}", table.name);
        // every documented table can be queried, and SELECT * reads its own columns
        let query = Query::compile(&format!("SELECT * FROM #{}", table.name)).unwrap_or_else(|err| panic!("{}: {}", table.name, err));
        for column in query.columns() {
            assert!(table.columns.iter().any(|it| it.name == column.name), "{}.{}", table.name, column.name);
        }
        query.execute_at(ledger(), &Params::new(), today()).unwrap();
    }
}

#[test]
fn the_fava_demo_ledger_prices_match_the_store() {
    let ledger = common::fava_demo_ledger();
    let run = |sql: &str| Query::compile(sql).unwrap().execute_at(&ledger, &Params::new(), today()).unwrap().rows;
    assert_eq!(run("SELECT count(*) FROM #prices"), vec![vec![Value::Int(846)]]);
    assert_eq!(run("SELECT DISTINCT currency FROM #prices").len(), 6);
}
