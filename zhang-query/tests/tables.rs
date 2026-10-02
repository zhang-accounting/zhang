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
    assert_eq!(
        err.message,
        "unknown column 'account' in #prices (its columns are date, currency, amount, meta)"
    );
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
         project: [currency, date] (2 of 4 columns)\n"
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
        vec![
            ("date", DataType::Date),
            ("currency", DataType::Str),
            ("amount", DataType::Amount),
            ("meta", DataType::Str)
        ]
    );
    let names = schema.tables.iter().map(|table| table.name).collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "postings",
            "entries",
            "transactions",
            "prices",
            "balances",
            "notes",
            "events",
            "documents",
            "accounts",
            "commodities"
        ]
    );
    for table in &schema.tables {
        assert!(!table.description.is_empty(), "{}", table.name);
        // the projector keeps one bit per column, and names are unique within a table
        assert!(table.columns.len() <= 64, "{}", table.name);
        let mut names = table.columns.iter().map(|column| column.name.to_ascii_lowercase()).collect::<Vec<_>>();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), table.columns.len(), "{}", table.name);
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

// ---------------------------------------------------------------------------------------
// zhang's directives in the beanquery tables

/// Budgets, a `balance ... with pad`, a failing balance assertion, a rejected transaction,
/// a relative document path, metadata and a closed account.
const DIRECTIVES: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
  precision: "2"

1970-01-01 open Assets:Bank USD
  booking_method: "FIFO"
1970-01-01 open Expenses:Food
  budget: food
  note: "say \"hi\""
1970-01-01 open Equity:Opening

2024-01-01 budget food USD
  alias: "Eating"
2024-01-02 budget-add food 100.00 USD

2024-01-05 * "Cafe" "lunch" #food ^receipt
  Expenses:Food    12.50 USD
  Assets:Bank

2024-01-06 * "Broken" "two implicit postings"
  Expenses:Food
  Assets:Bank

2024-02-01 balance Assets:Bank 100.00 USD with pad Equity:Opening
2024-02-02 balance Assets:Bank 99.00 USD
2024-02-03 note Assets:Bank "check the statement"
2024-02-04 document Assets:Bank "../docs/./statement.pdf"
2024-02-05 custom "fava-option" "language" "en"
2024-03-01 close Expenses:Food
"#;

fn directives_ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| common::load_text(DIRECTIVES))
}

fn on_directives(sql: &str) -> Vec<Vec<String>> {
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(directives_ledger(), &Params::new(), today()))
        .unwrap_or_else(|err| panic!("{}: {}", sql, err));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

fn rows(expected: &[&[&str]]) -> Vec<Vec<String>> {
    expected.iter().map(|row| row.iter().map(|it| (*it).to_owned()).collect()).collect()
}

#[test]
fn entries_hold_every_directive_in_beancount_order() {
    assert_eq!(
        on_directives("SELECT date, type, flag, payee, narration, tags, accounts FROM #entries"),
        rows(&[
            // open sorts before commodity on the same day, as in beancount
            &["1970-01-01", "open", "NULL", "NULL", "NULL", "NULL", "Assets:Bank"],
            &["1970-01-01", "open", "NULL", "NULL", "NULL", "NULL", "Expenses:Food"],
            &["1970-01-01", "open", "NULL", "NULL", "NULL", "NULL", "Equity:Opening"],
            &["1970-01-01", "commodity", "NULL", "NULL", "NULL", "NULL", ""],
            &["2024-01-01", "budget", "NULL", "NULL", "NULL", "NULL", ""],
            &["2024-01-02", "budget-add", "NULL", "NULL", "NULL", "NULL", ""],
            &["2024-01-05", "transaction", "*", "Cafe", "lunch", "food", "Assets:Bank, Expenses:Food"],
            // the rejected transaction is not an entry; the pad is a balance entry followed by
            // its padding transaction, and the balance-check transactions are not entries
            &["2024-02-01", "balance", "NULL", "NULL", "NULL", "NULL", "Assets:Bank, Equity:Opening"],
            &[
                "2024-02-01",
                "transaction",
                "P",
                "Balance Pad",
                "pad Assets:Bank to Equity:Opening",
                "",
                "Assets:Bank, Equity:Opening"
            ],
            &["2024-02-02", "balance", "NULL", "NULL", "NULL", "NULL", "Assets:Bank"],
            &["2024-02-03", "note", "NULL", "NULL", "NULL", "", "Assets:Bank"],
            &["2024-02-04", "document", "NULL", "NULL", "NULL", "", "Assets:Bank"],
            &["2024-02-05", "custom", "NULL", "NULL", "NULL", "NULL", ""],
            &["2024-03-01", "close", "NULL", "NULL", "NULL", "NULL", "Expenses:Food"],
        ])
    );
    // a transaction entry has the id of its postings; every id is unique
    let entry_ids = on_directives("SELECT id FROM #entries WHERE type = 'transaction' ORDER BY id");
    assert_eq!(entry_ids, on_directives("SELECT DISTINCT id FROM #postings ORDER BY id"));
    let ids = on_directives("SELECT DISTINCT id FROM #entries");
    assert_eq!(ids.len(), 14);
    // the default SELECT * of #entries
    let columns = Query::compile("SELECT * FROM #entries").unwrap().columns();
    let names = columns.iter().map(|it| it.name.as_str()).collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "id",
            "type",
            "filename",
            "date",
            "year",
            "month",
            "day",
            "flag",
            "payee",
            "narration",
            "description",
            "tags",
            "links",
            "meta",
            "accounts"
        ]
    );
    let filenames = on_directives("SELECT DISTINCT filename FROM #entries");
    assert_eq!(filenames.len(), 1);
    assert!(filenames[0][0].ends_with("main.zhang"), "{:?}", filenames);
}

#[test]
fn transactions_are_the_stored_transactions() {
    assert_eq!(
        on_directives("SELECT * FROM #transactions"),
        rows(&[
            &["2024-01-05", "*", "Cafe", "lunch", "food", "receipt", "Assets:Bank, Expenses:Food"],
            &[
                "2024-02-01",
                "P",
                "Balance Pad",
                "pad Assets:Bank to Equity:Opening",
                "",
                "",
                "Assets:Bank, Equity:Opening"
            ],
        ])
    );
}

#[test]
fn balances_report_the_discrepancy_zhang_found() {
    assert_eq!(
        on_directives("SELECT * FROM #balances"),
        rows(&[
            // a balance with pad always holds
            &["2024-02-01", "Assets:Bank", "100.00 USD", "NULL", "NULL"],
            // the balance (100.00 after the pad) minus the asserted 99.00
            &["2024-02-02", "Assets:Bank", "99.00 USD", "NULL", "1.00 USD"],
        ])
    );
    assert_eq!(on_directives("SELECT count(*) FROM #balances WHERE discrepancy IS NULL"), rows(&[&["1"]]));
}

#[test]
fn notes_documents_and_commodities() {
    assert_eq!(
        on_directives("SELECT * FROM #notes"),
        rows(&[&["2024-02-03", "Assets:Bank", "check the statement", "", ""]])
    );
    // a relative document path is resolved against the directory of its ledger file
    let documents = on_directives("SELECT filename FROM #documents");
    let filename = &documents[0][0];
    assert!(filename.ends_with("/docs/statement.pdf") && !filename.contains(".."), "{}", filename);
    assert_eq!(on_directives("SELECT * FROM #commodities"), rows(&[&["precision: \"2\"", "1970-01-01", "USD"]]));
}

#[test]
fn accounts_expose_open_and_close_as_structures() {
    assert_eq!(
        on_directives("SELECT * FROM #accounts"),
        rows(&[
            &["Assets:Bank", "1970-01-01", "NULL"],
            &["Expenses:Food", "1970-01-01", "2024-03-01"],
            &["Equity:Opening", "1970-01-01", "NULL"],
        ])
    );
    assert_eq!(
        on_directives(
            "SELECT account, open.date, open.account, open.currencies, open.booking, close.date, close.account FROM #accounts \
             WHERE close IS NOT NULL OR open.booking = 'FIFO' ORDER BY open.date, account"
        ),
        rows(&[
            &["Assets:Bank", "1970-01-01", "Assets:Bank", "USD", "FIFO", "NULL", "NULL"],
            &["Expenses:Food", "1970-01-01", "Expenses:Food", "NULL", "NULL", "2024-03-01", "Expenses:Food"],
        ])
    );
    // metadata of the open directive, as text and by key
    assert_eq!(
        on_directives("SELECT open.meta, close.meta, meta('budget'), entry_meta('note') FROM #accounts WHERE account = 'Expenses:Food'"),
        rows(&[&["budget: \"food\", note: \"say \\\"hi\\\"\"", "", "food", "say \"hi\""]])
    );
    let types = Query::compile("SELECT open, open.date, open.currencies, close.meta FROM #accounts")
        .unwrap()
        .columns();
    let types = types.iter().map(|it| it.ty).collect::<Vec<_>>();
    assert_eq!(types, [DataType::Date, DataType::Date, DataType::Set, DataType::Str]);
}

#[test]
fn attribute_errors_point_at_the_attribute() {
    let attribute_error = |sql: &str| match Query::compile(sql) {
        Ok(_) => panic!("{} should not compile", sql),
        Err(err) => (err.kind, err.column, err.message),
    };
    assert_eq!(
        attribute_error("SELECT open.datum FROM #accounts"),
        (
            QueryErrorKind::Compile,
            Some(13),
            "unknown attribute 'datum' of open; its attributes are date, account, currencies, booking, meta".to_owned()
        )
    );
    assert_eq!(
        attribute_error("SELECT account.name FROM #accounts"),
        (
            QueryErrorKind::Compile,
            Some(16),
            "account is a str and has no attributes, so account.name does not exist".to_owned()
        )
    );
    assert_eq!(attribute_error("SELECT x.date FROM #accounts").1, Some(8),);
    assert_eq!(
        attribute_error("SELECT open.date"),
        (QueryErrorKind::Compile, Some(8), "unknown column 'open'".to_owned())
    );
    // a parse error for a dangling dot
    let err = Query::compile("SELECT open. FROM #accounts").err().unwrap();
    assert_eq!(err.kind, QueryErrorKind::Parse);
}

#[test]
fn having_and_pivot_by_work_over_tables() {
    assert_eq!(
        on_directives(
            "SELECT type, year(date) AS y, count(*) AS n FROM #entries WHERE type IN ('open', 'balance', 'transaction') \
             GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY type, y"
        ),
        rows(&[&["balance", "NULL", "2"], &["open", "3", "NULL"], &["transaction", "NULL", "2"]])
    );
    let result = Query::compile("SELECT type, year(date) AS y, count(*) AS n FROM #entries GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY type, y")
        .unwrap()
        .execute_at(directives_ledger(), &Params::new(), today())
        .unwrap();
    let names = result.columns.iter().map(|it| it.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["type/y", "1970", "2024"]);
}

// ---------------------------------------------------------------------------------------
// The beanquery oracle: tests/tables/oracle.json, generated by tests/tables/generate.py.

mod oracle {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use serde_json::{json, Value as Json};
    use zhang_core::ledger::Ledger;
    use zhang_query::{DataType, Params, Query, Value};

    use super::{common, today};

    fn ledger(name: &str) -> Ledger {
        match name {
            "fava" => common::fava_demo_ledger(),
            "extra" => common::load_ledger(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tables/ledger"), "main.zhang"),
            other => panic!("unknown oracle ledger {}", other),
        }
    }

    fn type_name(ty: DataType) -> &'static str {
        match ty {
            DataType::Null => "null",
            DataType::Bool => "bool",
            DataType::Int => "int",
            DataType::Decimal => "decimal",
            DataType::Str => "str",
            DataType::Date => "date",
            DataType::Set => "set",
            DataType::Amount => "amount",
            DataType::Position => "position",
            DataType::Inventory => "inventory",
        }
    }

    /// A decimal written without trailing zeros, so that numbers compare by value.
    fn number(text: &str) -> Json {
        let number = BigDecimal::from_str(text).unwrap_or_else(|_| panic!("not a number: {}", text)).normalized();
        Json::String(zhang_query::decimal::to_plain_string(&number))
    }

    fn amount(number_text: &str, currency: &str) -> Json {
        json!({"number": number(number_text), "currency": currency})
    }

    /// An engine value in the oracle's encoding, with normalized numbers.
    fn encode(value: &Value) -> Json {
        match value {
            Value::Null => Json::Null,
            Value::Bool(it) => json!(it),
            Value::Int(it) => json!(it),
            Value::Decimal(it) => number(&it.to_string()),
            Value::Str(it) => json!(it),
            Value::Date(it) => json!(it.format("%Y-%m-%d").to_string()),
            Value::Set(it) => json!(it.iter().collect::<Vec<_>>()),
            Value::Amount(it) => amount(&it.number.to_string(), &it.commodity),
            Value::Position(it) => panic!("positions are not encoded: {:?}", it),
            Value::Inventory(it) => {
                let mut positions = it
                    .positions()
                    .map(|position| {
                        assert!(position.cost.is_none(), "inventories at cost are not encoded");
                        amount(&position.units.number.to_string(), &position.units.commodity)
                    })
                    .collect::<Vec<_>>();
                positions.sort_by_key(|it| it["currency"].as_str().unwrap_or_default().to_owned());
                Json::Array(positions)
            }
        }
    }

    /// An oracle cell with normalized numbers.
    fn normalize(ty: &str, cell: &Json) -> Json {
        match (ty, cell) {
            (_, Json::Null) => Json::Null,
            ("decimal", Json::String(text)) => number(text),
            ("amount", cell) => amount(cell["number"].as_str().unwrap(), cell["currency"].as_str().unwrap()),
            ("inventory", Json::Array(positions)) => Json::Array(
                positions
                    .iter()
                    .map(|it| amount(it["number"].as_str().unwrap(), it["currency"].as_str().unwrap()))
                    .collect(),
            ),
            _ => cell.clone(),
        }
    }

    #[test]
    fn tables_match_beanquery() {
        let oracle: Json = serde_json::from_str(include_str!("tables/oracle.json")).unwrap();
        let mut ledgers: HashMap<String, Ledger> = HashMap::new();
        let cases = oracle["cases"].as_array().unwrap();
        let mut failures = vec![];
        for case in cases {
            let sql = case["query"].as_str().unwrap();
            let name = case["ledger"].as_str().unwrap();
            let ledger = ledgers.entry(name.to_owned()).or_insert_with(|| ledger(name));
            let result = match Query::compile(sql).and_then(|query| query.execute_at(ledger, &Params::new(), today())) {
                Ok(result) => result,
                Err(err) => {
                    failures.push(format!("{}: {}", sql, err));
                    continue;
                }
            };
            let expected_columns = case["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|it| (it["name"].as_str().unwrap().to_owned(), it["type"].as_str().unwrap().to_owned()))
                .collect::<Vec<_>>();
            let columns = result
                .columns
                .iter()
                .map(|it| (it.name.clone(), type_name(it.ty).to_owned()))
                .collect::<Vec<_>>();
            if columns != expected_columns {
                failures.push(format!("{}: columns {:?}, expected {:?}", sql, columns, expected_columns));
                continue;
            }
            let mut expected = case["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    let cells = row.as_array().unwrap().iter().zip(&expected_columns);
                    Json::Array(cells.map(|(cell, (_, ty))| normalize(ty, cell)).collect())
                })
                .collect::<Vec<_>>();
            let mut rows = result.rows.iter().map(|row| Json::Array(row.iter().map(encode).collect())).collect::<Vec<_>>();
            if !case["ordered"].as_bool().unwrap() {
                expected.sort_by_key(Json::to_string);
                rows.sort_by_key(Json::to_string);
            }
            if rows != expected {
                let first = rows.iter().zip(&expected).position(|(a, b)| a != b).unwrap_or(rows.len().min(expected.len()));
                failures.push(format!(
                    "{}: {} rows, expected {}; first difference at row {}: {:?} vs expected {:?}",
                    sql,
                    rows.len(),
                    expected.len(),
                    first,
                    rows.get(first),
                    expected.get(first)
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} oracle cases differ:\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
        assert!(cases.len() >= 30);
    }
}
