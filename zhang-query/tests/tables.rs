//! `FROM #table`: the table registry, name resolution, per-table columns, the projector,
//! `explain()`, CSV export and the schema.

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

/// zhang extension: `time` and `timestamp` tell apart the quotes of one day. A price without a
/// time is at local midnight, and the quotes of a day come in time order whatever their written
/// order. Asia/Shanghai is UTC+8 without daylight saving, so 2024-01-15 00:00:00 +08:00 is
/// 1705248000 (19737 days of 86400 s since 1970-01-01, minus 8 hours).
#[test]
fn prices_have_the_time_and_timestamp_of_their_directive() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"
option "operating_currency" "USD"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity USD
1970-01-01 commodity BTC

2024-01-15 18:00 price BTC 42000 USD
2024-01-15 price BTC 40000 USD
2024-01-15 09:30:15 price BTC 41000 USD
"#,
    );
    let run = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql).unwrap().execute_at(&ledger, &Params::new(), today()).unwrap();
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    assert_eq!(
        run("SELECT date, time, timestamp, amount FROM #prices"),
        vec![
            vec!["2024-01-15", "00:00:00", "1705248000", "40000 USD"],
            vec!["2024-01-15", "09:30:15", "1705282215", "41000 USD"],
            vec!["2024-01-15", "18:00:00", "1705312800", "42000 USD"],
        ]
    );
    // the latest quote of each currency
    assert_eq!(
        run("SELECT currency, last(date), last(time), last(amount) FROM #prices GROUP BY currency"),
        vec![vec!["BTC", "2024-01-15", "18:00:00", "42000 USD"]]
    );
    // they are extensions: SELECT * stays beanquery's
    assert_eq!(run("SELECT * FROM #prices")[0], vec!["2024-01-15", "BTC", "40000 USD"]);
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
        "unknown column 'account' in #prices (its columns are date, currency, amount, meta, time, timestamp, id, type, filename, year, \
         month, day, seq, metas)"
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
         project: [currency, date] (2 of 14 columns)\n"
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
        count_total: false,
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
            ("meta", DataType::Str),
            ("time", DataType::Str),
            ("timestamp", DataType::Int),
            ("id", DataType::Str),
            ("type", DataType::Str),
            ("filename", DataType::Str),
            ("year", DataType::Int),
            ("month", DataType::Int),
            ("day", DataType::Int),
            ("seq", DataType::Int),
            ("metas", DataType::Metas)
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
            "commodities",
            "budgets",
            "budget_events",
            "budget_definitions",
            "errors"
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
    let ledger = zhang_testkit::ledger::fava_demo_ledger();
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
2024-02-03 note Assets:Bank "check the statement" #todo ^stmt
2024-02-04 document Assets:Bank "../docs/./statement.pdf"
2024-02-05 custom "fava-option" "language" "en"
2024-03-01 close Expenses:Food
"#;

fn directives_ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| zhang_testkit::ledger::load_text(DIRECTIVES))
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
            // the rejected transaction is not an entry; the pad's padding transaction comes before
            // the balance, which zhang checks after it, and the balance-check transactions are not
            // entries
            &[
                "2024-02-01",
                "transaction",
                "P",
                "Balance Pad",
                "pad Assets:Bank to Equity:Opening",
                "",
                "Assets:Bank, Equity:Opening"
            ],
            &["2024-02-01", "balance", "NULL", "NULL", "NULL", "NULL", "Assets:Bank, Equity:Opening"],
            &["2024-02-02", "balance", "NULL", "NULL", "NULL", "NULL", "Assets:Bank"],
            &["2024-02-03", "note", "NULL", "NULL", "NULL", "todo", "Assets:Bank"],
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
            // a balance with pad holds: it pads to its amount
            &["2024-02-01", "Assets:Bank", "100.00 USD", "NULL", "NULL"],
            // the balance (100.00 after the pad) minus the asserted 99.00
            &["2024-02-02", "Assets:Bank", "99.00 USD", "NULL", "1.00 USD"],
        ])
    );
    assert_eq!(on_directives("SELECT count(*) FROM #balances WHERE discrepancy IS NULL"), rows(&[&["1"]]));
}

#[test]
fn a_balance_discrepancy_is_measured_from_the_postings() {
    // an assertion moves no balance, failing or not: each one is measured from the postings, and a
    // transaction flagged `C` (beancount's conversions) is an ordinary transaction
    let ledger = zhang_testkit::ledger::load_text(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Equity:Opening\n\
         2024-01-01 * \"Salary\"\n  Assets:Bank 165 CNY\n  Equity:Opening\n\
         2024-01-02 balance Assets:Bank 200 CNY\n\
         2024-01-03 balance Assets:Bank 200 CNY\n\
         2024-01-04 balance Assets:Bank 165.004 ~ 0.01 CNY\n\
         2024-01-05 C \"Conversion\"\n  Assets:Bank 10 CNY\n  Equity:Opening\n",
    );
    let query = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql)
            .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
            .unwrap_or_else(|err| panic!("{}: {}", sql, err));
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    assert_eq!(
        query("SELECT date, discrepancy FROM #balances"),
        rows(&[&["2024-01-02", "-35 CNY"], &["2024-01-03", "-35 CNY"], &["2024-01-04", "NULL"]])
    );
    assert_eq!(
        query("SELECT flag, sum(position) WHERE account = 'Assets:Bank' GROUP BY flag ORDER BY flag"),
        rows(&[&["*", "165 CNY"], &["C", "10 CNY"]])
    );
}

#[test]
fn a_pad_is_an_entry_and_its_padding_a_transaction_on_its_date() {
    let ledger = zhang_testkit::ledger::load_text(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Assets:Cash\n1970-01-01 open Equity:Opening\n\
         2024-01-01 pad Assets:Bank Equity:Opening\n\
         2024-01-01 pad Assets:Cash Equity:Opening\n\
         2024-02-01 balance Assets:Bank 100 CNY\n",
    );
    let query = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql)
            .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
            .unwrap_or_else(|err| panic!("{}: {}", sql, err));
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    assert_eq!(
        query("SELECT date, type, flag, accounts FROM #entries WHERE type IN ('pad', 'transaction', 'balance')"),
        // the padding right after its pad, as in beancount
        rows(&[
            &["2024-01-01", "pad", "NULL", "Assets:Bank, Equity:Opening"],
            &["2024-01-01", "transaction", "P", "Assets:Bank, Equity:Opening"],
            &["2024-01-01", "pad", "NULL", "Assets:Cash, Equity:Opening"],
            &["2024-02-01", "balance", "NULL", "Assets:Bank"],
        ])
    );
    // the pad of Assets:Cash serves no assertion
    assert_eq!(
        query("SELECT kind, account, message FROM #errors"),
        rows(&[&["UnusedPad", "Assets:Cash", "Pad is not used by any later balance assertion of its account"]])
    );
}

#[test]
fn a_balance_with_pad_reports_the_discrepancy_a_later_pad_of_its_time_leaves() {
    let ledger = zhang_testkit::ledger::load_text(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Assets:Bank:Checking\n1970-01-01 open Equity:Opening\n\
         2024-01-02 * \"init\"\n  Assets:Bank 345 CNY\n  Assets:Bank:Checking 155 CNY\n  Equity:Opening\n\
         2024-01-10 balance Assets:Bank 500 CNY with pad Equity:Opening\n\
         2024-01-10 balance Assets:Bank:Checking 200 CNY with pad Equity:Opening\n",
    );
    let result = Query::compile("SELECT account, discrepancy FROM #balances")
        .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
        .unwrap();
    let rows_of = result
        .rows
        .iter()
        .map(|row| row.iter().map(Value::to_string).collect())
        .collect::<Vec<Vec<String>>>();
    assert_eq!(rows_of, rows(&[&["Assets:Bank", "45 CNY"], &["Assets:Bank:Checking", "NULL"]]));
}

/// The tables of one kind of directive are views of `#entries`: they have the columns every directive has there, with the
/// same values.
#[test]
fn directive_tables_have_the_columns_of_their_entries() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"
1970-01-01 commodity USD
  name: "US Dollar"
1970-01-01 open Assets:Bank
2024-01-01 price EUR 1.10 USD
  source: "ecb"
2024-01-02 note Assets:Bank "hi"
  by: "me"
2024-01-03 event "location" "Berlin"
2024-01-04 balance Assets:Bank 0 USD
"#,
    );
    let run = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql)
            .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
            .unwrap_or_else(|err| panic!("{}: {}", sql, err));
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    let shared = "id, filename, date, year, month, day, time, timestamp, seq, meta, metas";
    for (table, kind) in [
        ("prices", "price"),
        ("balances", "balance"),
        ("notes", "note"),
        ("events", "event"),
        ("commodities", "commodity"),
    ] {
        let rows = run(&format!("SELECT {shared} FROM #{table}"));
        assert_eq!(rows.len(), 1, "{table}");
        assert_eq!(rows, run(&format!("SELECT {shared} FROM #entries WHERE type = '{kind}'")), "{table}");
    }
    // `type` is the kind of directive, except on #events, where it is the kind of event
    assert_eq!(run("SELECT type FROM #prices"), rows(&[&["price"]]));
    assert_eq!(run("SELECT type FROM #events"), rows(&[&["location"]]));
}

#[test]
fn notes_documents_and_commodities() {
    assert_eq!(
        on_directives("SELECT * FROM #notes"),
        rows(&[&["2024-02-03", "Assets:Bank", "check the statement", "todo", "stmt"]])
    );
    // a relative document path is resolved against the directory of its ledger file, the root
    // here, which the ledger names by its path within it, as `zhang serve` does
    let documents = on_directives("SELECT filename FROM #documents");
    assert_eq!(documents[0][0], "../docs/statement.pdf");
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

/// `open.time` is the time of day written in the account's first `open`, `00:00:00` without one: with `open.date`,
/// the instant its lifecycle starts at, the one `account_status` compares with.
#[test]
fn open_time_is_the_time_written_in_the_open() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"
option "timezone" "Asia/Shanghai"
1970-01-01 open Assets:Plain
2024-01-05 09:30:00 open Assets:Timed
2024-01-05 09:30:00 open Assets:Reopened
2024-01-06 close Assets:Reopened
2024-01-07 15:00:00 open Assets:Reopened
2024-01-01 close Assets:NeverOpened
"#,
    );
    let run = |sql: &str| {
        let result = Query::compile(sql).unwrap().execute_at(&ledger, &Params::new(), today()).unwrap();
        result
            .rows
            .iter()
            .map(|row| row.iter().map(Value::to_string).collect())
            .collect::<Vec<Vec<String>>>()
    };
    assert_eq!(
        run("SELECT account, open.date, open.time, account_status(account, 2024-01-05, '09:30:00') FROM #accounts ORDER BY account"),
        rows(&[
            &["Assets:NeverOpened", "NULL", "NULL", "closed"],
            &["Assets:Plain", "1970-01-01", "00:00:00", "open"],
            &["Assets:Reopened", "2024-01-05", "09:30:00", "open"],
            &["Assets:Timed", "2024-01-05", "09:30:00", "open"],
        ])
    );
    // the accounts opened by an instant, by date and then by time: those a document dated then may name
    assert_eq!(
        run("SELECT account FROM #accounts WHERE open.date < 2024-01-05 OR (open.date = 2024-01-05 AND open.time <= '09:29:59') ORDER BY account"),
        rows(&[&["Assets:Plain"]])
    );
    assert_eq!(
        run("SELECT account FROM #accounts WHERE open.date < 2024-01-05 OR (open.date = 2024-01-05 AND open.time <= '09:30:00') ORDER BY account"),
        rows(&[&["Assets:Plain"], &["Assets:Reopened"], &["Assets:Timed"]])
    );
    let columns = Query::compile("SELECT open.time FROM #accounts").unwrap().columns();
    assert_eq!(columns[0].ty, DataType::Str);
}

/// `open.booking` is the method booking books the account with: the last `booking_method` value
/// of the latest `open` that has one, and NULL when the account books with the default, also
/// for a value that is not a method zhang books with.
#[test]
fn open_booking_is_the_method_booking_uses() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"
option "operating_currency" "USD"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Twice
  booking_method: "FIFO"
  booking_method: "LIFO"
1970-01-01 open Assets:Invalid
  booking_method: "XYZ"
1970-01-01 open Assets:Unsupported
  booking_method: "AVERAGE"
1970-01-01 open Assets:Reopened
  booking_method: "FIFO"
1970-01-01 open Assets:Plain

2024-01-01 * "buy"
  Assets:Twice   1 AAPL {100 USD}
  Assets:Cash
2024-01-02 * "buy"
  Assets:Twice   1 AAPL {120 USD}
  Assets:Cash
2024-01-03 * "sell"
  Assets:Twice  -1 AAPL {}
  Assets:Cash   120 USD
2024-02-01 close Assets:Reopened
2024-03-01 open Assets:Reopened
  booking_method: "LIFO"
"#,
    );
    let run = |sql: &str| {
        let result = Query::compile(sql).unwrap().execute_at(&ledger, &Params::new(), today()).unwrap();
        result
            .rows
            .iter()
            .map(|row| row.iter().map(Value::to_string).collect())
            .collect::<Vec<Vec<String>>>()
    };
    assert_eq!(
        run("SELECT account, open.booking FROM #accounts WHERE account != 'Assets:Cash' ORDER BY account"),
        rows(&[
            &["Assets:Invalid", "NULL"],
            &["Assets:Plain", "NULL"],
            &["Assets:Reopened", "LIFO"],
            &["Assets:Twice", "LIFO"],
            &["Assets:Unsupported", "NULL"],
        ])
    );
    // and the sale of Assets:Twice booked the newest lot, as LIFO does
    assert_eq!(run("SELECT cost_number WHERE account = 'Assets:Twice' AND number < 0"), rows(&[&["120"]]));
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
            "unknown attribute 'datum' of open; its attributes are date, account, currencies, booking, meta, time".to_owned()
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

    use super::today;

    fn ledger(name: &str) -> Ledger {
        match name {
            "fava" => zhang_testkit::ledger::fava_demo_ledger(),
            "extra" => zhang_testkit::ledger::load_ledger(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tables/ledger"), "main.zhang"),
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
            DataType::Interval => "interval",
            DataType::Metas => "metas",
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
            Value::Interval(it) => json!(it.to_string()),
            Value::Metas(it) => panic!("metadata pairs are not encoded: {:?}", it),
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

#[test]
fn the_paddings_of_a_pad_follow_it_each_with_its_own_id_as_beancount_orders_a_day() {
    // a pad between two transactions of its day, padding two commodities
    let ledger = zhang_testkit::ledger::load_text(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n1970-01-01 open Equity:Opening\n\
         2024-01-05 * \"before pad\"\n  Assets:Bank -10 CNY\n  Expenses:Food\n\
         2024-01-05 pad Assets:Bank Equity:Opening\n\
         2024-01-05 * \"after pad\"\n  Assets:Bank -20 CNY\n  Expenses:Food\n\
         2024-01-05 balance Assets:Bank -30 CNY\n\
         2024-01-06 balance Assets:Bank 100 CNY\n\
         2024-01-06 balance Assets:Bank 7 USD\n",
    );
    let query = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql)
            .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
            .unwrap_or_else(|err| panic!("{}: {}", sql, err));
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    // the day's balance first, then the rest in file order, each padding right after its pad, as bean-query lists them
    assert_eq!(
        query("SELECT seq, type, narration FROM #entries WHERE date = 2024-01-05"),
        rows(&[
            &["3", "balance", "NULL"],
            &["4", "transaction", "before pad"],
            &["5", "pad", "NULL"],
            &["6", "transaction", "pad Assets:Bank to Equity:Opening"],
            &["7", "transaction", "pad Assets:Bank to Equity:Opening"],
            &["8", "transaction", "after pad"],
        ])
    );
    // each padding is its own transaction: its id and seq are those of its postings (the running balance is over
    // the rows the query selects)
    let entries = query("SELECT seq, id FROM #transactions WHERE flag = 'P'");
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0][1], entries[1][1]);
    // the same rows of #entries, with the same ids and seqs: in the order zhang processes the day, right after
    // their pad
    assert_eq!(
        query("SELECT seq, id FROM #entries WHERE narration = 'pad Assets:Bank to Equity:Opening' ORDER BY seq"),
        entries
    );
    assert_eq!(
        query("SELECT seq, type, narration FROM #entries WHERE date = 2024-01-05 ORDER BY seq"),
        query("SELECT seq, type, narration FROM #entries WHERE date = 2024-01-05")
    );
    assert_eq!(
        query("SELECT seq, id, position, balance WHERE account = 'Assets:Bank' AND flag = 'P'"),
        vec![
            vec![entries[0][0].clone(), entries[0][1].clone(), "130 CNY".to_owned(), "130 CNY".to_owned()],
            vec![entries[1][0].clone(), entries[1][1].clone(), "7 USD".to_owned(), "130 CNY, 7 USD".to_owned()],
        ]
    );
    // the balance of the pad's day is checked before it, as in beancount; the next day's are padded
    assert_eq!(
        query("SELECT date, amount, discrepancy, actual, passed FROM #balances"),
        rows(&[
            &["2024-01-05", "-30 CNY", "30 CNY", "0 CNY", "FALSE"],
            &["2024-01-06", "100 CNY", "NULL", "100 CNY", "TRUE"],
            &["2024-01-06", "7 USD", "NULL", "7 USD", "TRUE"],
        ])
    );
}

#[test]
fn a_pad_is_processed_after_the_balance_entries_of_its_day() {
    // a pad written at 9:00, before a balance at noon: zhang processes it after every balance entry of its day, at
    // the time of the last one, with its padding right after it
    let ledger = zhang_testkit::ledger::load_text(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n1970-01-01 open Equity:Opening\n\
         2024-01-05 09:00:00 pad Assets:Bank Equity:Opening\n\
         2024-01-05 08:00:00 * \"breakfast\"\n  Assets:Bank -10 CNY\n  Expenses:Food\n\
         2024-01-05 12:00:00 balance Assets:Bank -10 CNY\n\
         2024-01-06 balance Assets:Bank 100 CNY\n",
    );
    let query = |sql: &str| -> Vec<Vec<String>> {
        let result = Query::compile(sql)
            .and_then(|query| query.execute_at(&ledger, &Params::new(), today()))
            .unwrap_or_else(|err| panic!("{}: {}", sql, err));
        result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
    };
    assert_eq!(
        query("SELECT seq, type, narration FROM #entries WHERE date = 2024-01-05 ORDER BY seq"),
        rows(&[
            &["3", "transaction", "breakfast"],
            &["4", "balance", "NULL"],
            &["5", "pad", "NULL"],
            &["6", "transaction", "pad Assets:Bank to Equity:Opening"],
        ])
    );
    // each assertion comes right after the postings its actual balance includes: the next day's after the padding
    assert_eq!(
        query("SELECT seq, actual, passed FROM #balances ORDER BY seq"),
        rows(&[&["4", "-10 CNY", "TRUE"], &["7", "100 CNY", "TRUE"]])
    );
    // the padding is one transaction, with one id and seq in #entries, #transactions and its postings
    let padding = query("SELECT seq, id FROM #transactions WHERE flag = 'P'");
    assert_eq!(padding.len(), 1);
    assert_eq!(padding[0][0], "6");
    assert_eq!(
        query("SELECT seq, id FROM #entries WHERE narration = 'pad Assets:Bank to Equity:Opening'"),
        padding
    );
    assert_eq!(query("SELECT DISTINCT seq, id FROM #postings WHERE flag = 'P'"), padding);
}
