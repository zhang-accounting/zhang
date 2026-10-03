//! Acceptance tests of the wave 1 query-engine features of issue #479 (moving the server's
//! read endpoints onto the engine), written from the spec alone (track Y): the columns of
//! track D1, the tables of track D2 and the language and functions of track L.
//!
//! Everything here goes through BQL text and the public API, so the file compiles before the
//! features exist and each feature's tests fail on their own until it lands. The tests of the
//! new Rust API (`ExecuteOptions::count_total`, `QueryResult::total`, `Position::convert`,
//! `Inventory::convert`, `PriceMap::for_ledger`) are in `server_features_api.rs`, which does
//! not compile until that API exists.
//!
//! # Expected values
//!
//! - **beanquery functions** (the date functions, `interval`, `open_meta`, `commodity_meta`,
//!   `open_date`, `close_date`): `server_features/oracle/oracle.json`, written by beanquery
//!   0.2.0 over `server_features/oracle/main.zhang`. Regenerate with
//!   `/tmp/bqvenv/bin/python zhang-query/tests/server_features/oracle/generate.py` and check
//!   with `generate.py --check`.
//!   Where the lead ruled that zhang deliberately differs from beanquery (`date_bin` starts
//!   bin k at origin + k × stride, computed from the origin, and a date on a bin start begins
//!   that bin; `interval` accepts weeks), the oracle keeps beanquery's rows, generate.py marks
//!   the case `accepted_deviation`, and `ACCEPTED_DEVIATIONS` below holds the rows zhang must
//!   return, as in `conformance.rs`.
//! - **zhang extensions**: derived by hand from the small ledgers under `server_features/`;
//!   every expected value is explained next to it, so a reviewer can re-derive it from the
//!   ledger text.
//!
//! Amounts compare by value: `80.00 CNY` and `80 CNY` are the same cell (see [`normalize`]).
//!
//! # Readings of the spec
//!
//! Where the spec leaves a choice, the test picks the reading most useful to an end user and
//! says so in a comment that starts with `READING:`. The lead rules on them; change the test
//! with the ruling, not the other way round.
//!
//! # Performance smoke tests
//!
//! The `perf_*` tests are ignored by default. Run them in release mode:
//! `cargo test --release -p zhang-query --test server_features -- --ignored --nocapture`.
//! The big synthetic ledger is read from `ZHANG_QUERY_BIG_LEDGER` (default `/tmp/qm-b/big`).

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::Value as Json;
use zhang_core::ledger::Ledger;
use zhang_query::decimal::to_plain_string;
use zhang_query::{DataType, ExecuteOptions, ParamTypes, Params, Query, QueryError, QueryErrorKind, QueryResult, Value};

// =======================================================================================
// helpers

fn fixture_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/server_features").join(name)
}

macro_rules! fixture_ledger {
    ($function:ident, $name:literal) => {
        fn $function() -> &'static Ledger {
            static CELL: OnceLock<Ledger> = OnceLock::new();
            CELL.get_or_init(|| common::load_ledger(fixture_dir($name), "main.zhang"))
        }
    };
}

fixture_ledger!(journal, "journal");
fixture_ledger!(assertions, "assertions");
fixture_ledger!(documents, "documents");
fixture_ledger!(errors_ledger, "errors");
fixture_ledger!(budgets, "budgets");
fixture_ledger!(oracle_ledger, "oracle");

/// `today()` of every execution: after every entry of every fixture.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
}

fn options() -> ExecuteOptions {
    ExecuteOptions {
        today: Some(today()),
        ..ExecuteOptions::default()
    }
}

/// Compile `sql` with the types of `params` and execute it.
fn try_query(ledger: &Ledger, sql: &str, params: &Params) -> Result<QueryResult, QueryError> {
    Query::compile_with_params(sql, &params.types())?.execute_with_options(ledger, params, &options())
}

fn query_with(ledger: &Ledger, sql: &str, params: &Params) -> Vec<Vec<String>> {
    cells(&try_query(ledger, sql, params).unwrap_or_else(|err| panic!("{}\n  failed: {:?} {}", sql, err.kind, err.message)))
}

fn query(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    query_with(ledger, sql, &Params::new())
}

/// [`query`] with `today()` on another day.
fn query_on(ledger: &Ledger, today: NaiveDate, sql: &str) -> Vec<Vec<String>> {
    let options = ExecuteOptions {
        today: Some(today),
        ..ExecuteOptions::default()
    };
    let result = Query::compile(sql).and_then(|query| query.execute_with_options(ledger, &Params::new(), &options));
    cells(&result.unwrap_or_else(|err| panic!("{}\n  failed: {:?} {}", sql, err.kind, err.message)))
}

/// The error of a query that must fail.
fn query_error(ledger: &Ledger, sql: &str, params: &Params) -> QueryError {
    match try_query(ledger, sql, params) {
        Ok(result) => panic!("{}\n  should fail but returned {:?}", sql, cells(&result)),
        Err(err) => err,
    }
}

/// The cells of a result as text, amounts normalized.
fn cells(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .rows
        .iter()
        .map(|row| row.iter().map(|value| normalize(&value.to_string())).collect())
        .collect()
}

/// A cell that is exactly an amount (`<decimal> <CURRENCY>`) with its number normalized, so
/// that amounts compare by value whatever the scale (`80.00 CNY` is `80 CNY`). Other cells are
/// left as they are.
fn normalize(cell: &str) -> String {
    if let Some((number, currency)) = cell.split_once(' ') {
        let is_currency = !currency.is_empty() && currency.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if is_currency {
            if let Ok(number) = BigDecimal::from_str(number) {
                return format!("{} {}", to_plain_string(&number.normalized()), currency);
            }
        }
    }
    cell.to_owned()
}

fn rows(expected: &[&[&str]]) -> Vec<Vec<String>> {
    expected.iter().map(|row| row.iter().map(|cell| normalize(cell)).collect()).collect()
}

fn owned(expected: Vec<Vec<String>>) -> Vec<Vec<String>> {
    expected.into_iter().map(|row| row.iter().map(|cell| normalize(cell)).collect()).collect()
}

/// The single cell of a one-row, one-column result.
fn scalar(ledger: &Ledger, sql: &str) -> String {
    let result = query(ledger, sql);
    assert_eq!(result.len(), 1, "{}: {:?}", sql, result);
    assert_eq!(result[0].len(), 1, "{}: {:?}", sql, result);
    result[0][0].clone()
}

fn set(items: &[&str]) -> Value {
    Value::Set(items.iter().map(|it| (*it).to_owned()).collect::<BTreeSet<_>>())
}

/// The id of the transaction with this narration, from the `id` of its postings (which
/// exists before track D1 adds `#transactions.id`, so the D2 tests do not depend on it).
fn transaction_id(ledger: &Ledger, narration: &str) -> String {
    let sql = format!("SELECT DISTINCT id FROM #postings WHERE narration = '{}'", narration);
    scalar(ledger, &sql)
}

// =======================================================================================
// schema and documentation: every new column and function is in the registry-driven schema
// (`GET /api/query/schema`) with a description, and in both language references

fn column_doc(table: &str, column: &str) -> Option<(&'static str, &'static str)> {
    let schema = zhang_query::schema();
    let table = schema.tables.iter().find(|it| it.name == table)?;
    table.columns.iter().find(|it| it.name == column).map(|it| (it.ty.name(), it.description))
}

fn assert_columns(expected: &[(&str, &str, &str)]) {
    let mut missing = vec![];
    for (table, column, ty) in expected {
        match column_doc(table, column) {
            None => missing.push(format!("#{}.{} is missing", table, column)),
            Some((actual, description)) => {
                if !actual.eq_ignore_ascii_case(ty) {
                    missing.push(format!("#{}.{} is {} instead of {}", table, column, actual, ty));
                }
                if description.trim().is_empty() {
                    missing.push(format!("#{}.{} has no description", table, column));
                }
            }
        }
    }
    assert!(missing.is_empty(), "schema:\n  {}", missing.join("\n  "));
}

fn table_columns(table: &str) -> Vec<(String, String)> {
    let schema = zhang_query::schema();
    let table = schema
        .tables
        .iter()
        .find(|it| it.name == table)
        .unwrap_or_else(|| panic!("no table #{}", table));
    assert!(!table.description.trim().is_empty(), "#{} has no description", table.name);
    table.columns.iter().map(|it| (it.name.to_owned(), it.ty.name().to_owned())).collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
}

#[test]
fn d1_schema_has_the_new_columns() {
    assert_columns(&[
        ("postings", "time", "str"),
        ("postings", "timestamp", "int"),
        ("postings", "seq", "int"),
        ("postings", "posting_index", "int"),
        ("postings", "account_balance", "inventory"),
        ("postings", "balanced", "bool"),
        ("postings", "errors", "set"),
        ("postings", "metas", "metas"),
        ("postings", "entry_metas", "metas"),
        ("transactions", "time", "str"),
        ("transactions", "timestamp", "int"),
        ("transactions", "seq", "int"),
        ("transactions", "id", "str"),
        ("transactions", "balanced", "bool"),
        ("transactions", "errors", "set"),
        ("transactions", "metas", "metas"),
        ("entries", "time", "str"),
        ("entries", "timestamp", "int"),
        ("entries", "seq", "int"),
        ("entries", "metas", "metas"),
    ]);
    // the default table is #postings: its columns are also the schema's top-level columns
    let schema = zhang_query::schema();
    for column in [
        "time",
        "timestamp",
        "seq",
        "posting_index",
        "account_balance",
        "balanced",
        "errors",
        "metas",
        "entry_metas",
    ] {
        assert!(schema.columns.iter().any(|it| it.name == column), "schema.columns lacks {}", column);
    }
}

#[test]
fn d2_schema_has_the_new_columns_and_tables() {
    assert_columns(&[
        ("balances", "actual", "amount"),
        ("balances", "passed", "bool"),
        ("errors", "id", "str"),
        ("errors", "span_start", "int"),
        ("errors", "span_end", "int"),
    ]);
    // the new #documents columns come after the existing ones, so beanquery's SELECT * stays a prefix
    assert_eq!(
        table_columns("documents"),
        pairs(&[
            ("date", "date"),
            ("account", "str"),
            ("filename", "str"),
            ("tags", "set"),
            ("links", "set"),
            ("meta", "str"),
            ("source", "str"),
            ("path", "str"),
            ("transaction_id", "str"),
        ])
    );
    let wildcard = Query::compile("SELECT * FROM #documents").unwrap().columns();
    let wildcard = wildcard.iter().map(|it| it.name.as_str()).collect::<Vec<_>>();
    assert_eq!(&wildcard[..5], ["date", "account", "filename", "tags", "links"]);
    // #budgets keeps its columns and their order
    assert_eq!(
        table_columns("budgets"),
        pairs(&[
            ("name", "str"),
            ("alias", "str"),
            ("category", "str"),
            ("currency", "str"),
            ("date", "date"),
            ("year", "int"),
            ("month", "int"),
            ("assigned", "amount"),
            ("added", "amount"),
            ("activity", "amount"),
            ("available", "amount"),
            ("accounts", "set"),
            ("closed", "bool"),
        ])
    );
    // the new #budget_events table, with its columns in the spec's order
    assert_eq!(
        table_columns("budget_events"),
        pairs(&[
            ("name", "str"),
            ("date", "date"),
            ("time", "str"),
            ("timestamp", "int"),
            ("type", "str"),
            ("amount", "amount"),
        ])
    );
}

#[test]
fn l_schema_has_the_new_functions() {
    let schema = zhang_query::schema();
    let overloads = |name: &str| schema.functions.iter().filter(|it| it.name == name && !it.aggregate).collect::<Vec<_>>();
    let mut problems = vec![];
    // zhang extensions, with the signatures of the spec
    for (name, signature) in [
        ("icontains", "icontains(str, str) -> bool"),
        ("any_icontains", "any_icontains(set, str) -> bool"),
        ("intersects", "intersects(set, set) -> bool"),
        ("under", "under(str, str) -> bool"),
        ("meta_values", "meta_values(str) -> set"),
        ("entry_meta_values", "entry_meta_values(str) -> set"),
        // beanquery 0.2.0 signatures
        ("date_trunc", "date_trunc(str, date) -> date"),
        ("date_add", "date_add(date, int) -> date"),
        ("date_diff", "date_diff(date, date) -> int"),
        ("date_part", "date_part(str, date) -> int"),
        ("date", "date(int, int, int) -> date"),
        ("open_date", "open_date(str) -> date"),
        ("close_date", "close_date(str) -> date"),
    ] {
        let found = overloads(name);
        if !found.iter().any(|it| it.signature == signature) {
            problems.push(format!(
                "no overload {} (found {:?})",
                signature,
                found.iter().map(|it| &it.signature).collect::<Vec<_>>()
            ));
        }
        if found.iter().any(|it| it.description.trim().is_empty()) {
            problems.push(format!("{} has an overload without a description", name));
        }
    }
    // beanquery functions whose result type is beanquery's own (interval, the dynamically typed
    // metadata): present and described
    for name in ["date_bin", "interval", "open_meta", "commodity_meta"] {
        let found = overloads(name);
        if found.is_empty() {
            problems.push(format!("no function {}", name));
        }
        if found.iter().any(|it| it.description.trim().is_empty()) {
            problems.push(format!("{} has an overload without a description", name));
        }
    }
    assert!(problems.is_empty(), "functions:\n  {}", problems.join("\n  "));
}

/// Every name in `names` is mentioned in the English and the Chinese query-language reference.
/// The references list a column as `` `name` `` in a table row and a function by its signature,
/// `name(...)`.
fn assert_documented(names: &[&str]) {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/src/content/docs");
    for file in ["user-guide/query-language.md", "zh-cn/user-guide/query-language.md"] {
        let text = std::fs::read_to_string(docs.join(file)).unwrap_or_else(|err| panic!("{}: {}", file, err));
        let missing = names.iter().filter(|name| !text.contains(*name)).collect::<Vec<_>>();
        assert!(missing.is_empty(), "{} does not mention {:?}", file, missing);
    }
}

#[test]
fn d1_new_columns_are_documented_in_both_references() {
    assert_documented(&[
        "`time`",
        "`timestamp`",
        "`seq`",
        "`posting_index`",
        "`account_balance`",
        "`balanced`",
        "`errors`",
    ]);
}

#[test]
fn d2_new_columns_and_tables_are_documented_in_both_references() {
    assert_documented(&[
        "`actual`",
        "`passed`",
        "`source`",
        "`path`",
        "`transaction_id`",
        "`span_start`",
        "`span_end`",
        "#budget_events",
        "transfer_in",
        "transfer_out",
    ]);
}

#[test]
fn b_case_the_budget_definitions_and_date_bounds_are_documented_in_both_references() {
    assert_documented(&["### CASE", "CASE WHEN", "#budget_definitions", "`close`", "yearmonth(date) = :month"]);
}

#[test]
fn l_new_syntax_and_functions_are_documented_in_both_references() {
    assert_documented(&[
        "OFFSET",
        "icontains(",
        "any_icontains(",
        "intersects(",
        "under(",
        "date_trunc(",
        "date_add(",
        "date_diff(",
        "date_part(",
        "date_bin(",
        "interval(",
        "open_meta(",
        "commodity_meta(",
        "open_date(",
        "close_date(",
        "meta_values(",
        "entry_meta_values(",
        "`metas`",
        "`entry_metas`",
    ]);
}

// =======================================================================================
// track D1: time, timestamp, seq, id, posting_index, account_balance, balanced, errors
// (journal/main.zhang, ledger timezone Asia/Shanghai = UTC+8 without daylight saving)

/// Unix timestamps of the journal ledger, local time minus 8 hours:
///
/// - 2024-01-01T00:00:00Z is 1704067200 (19723 days of 86400 s since 1970-01-01), so
///   2024-01-15T00:00:00Z is 1704067200 + 14 × 86400 = 1705276800;
/// - 2024-01-15 00:00:00 +08:00 (no time written) is 1705276800 - 28800 = 1705248000;
/// - 2024-01-15 09:05 +08:00 (`09:05`, seconds 0) is 1705248000 + 9 × 3600 + 5 × 60 = 1705280700;
/// - 2024-01-15 10:30:00 +08:00 is 1705248000 + 10 × 3600 + 30 × 60 = 1705285800;
/// - 2023-12-31 00:00:00 +08:00 is 1705248000 - 15 × 86400 = 1703952000;
/// - 1970-01-01 00:00:00 +08:00 is -28800.
#[test]
fn d1_time_and_timestamp_of_entries_with_and_without_a_time() {
    assert_eq!(
        query(journal(), "SELECT narration, time, timestamp FROM #transactions WHERE date <= 2024-01-15"),
        rows(&[
            &["December salary", "00:00:00", "1703952000"],
            &["Groceries", "00:00:00", "1705248000"],
            &["Bread", "09:05:00", "1705280700"],
            &["Morning coffee", "10:30:00", "1705285800"],
        ])
    );
    // every posting has its transaction's time and timestamp
    assert_eq!(
        query(journal(), "SELECT narration, account, time, timestamp FROM #postings WHERE date = 2024-01-15"),
        rows(&[
            &["Groceries", "Expenses:Food", "00:00:00", "1705248000"],
            &["Groceries", "Assets:Bank", "00:00:00", "1705248000"],
            &["Bread", "Expenses:Food", "09:05:00", "1705280700"],
            &["Bread", "Assets:Bank", "09:05:00", "1705280700"],
            &["Morning coffee", "Expenses:Food", "10:30:00", "1705285800"],
            &["Morning coffee", "Assets:Bank", "10:30:00", "1705285800"],
        ])
    );
    // #entries: the same for transactions, and an `open` without a time is at local midnight
    assert_eq!(
        query(
            journal(),
            "SELECT type, narration, time, timestamp FROM #entries WHERE date = 2024-01-15 OR (type = 'open' AND 'Assets:Bank' IN accounts)"
        ),
        rows(&[
            &["open", "NULL", "00:00:00", "-28800"],
            &["transaction", "Groceries", "00:00:00", "1705248000"],
            &["transaction", "Bread", "09:05:00", "1705280700"],
            &["transaction", "Morning coffee", "10:30:00", "1705285800"],
        ])
    );
    // timestamps order entries the way the ledger does
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE date = 2024-01-15 ORDER BY timestamp DESC"),
        rows(&[&["Morning coffee"], &["Bread"], &["Groceries"]])
    );
}

/// The 27 entries of the journal ledger in #entries order: the 9 opens in written order, the 2
/// commodities (an open sorts before a commodity of the same day), then the dated entries. The
/// three transactions of 2024-01-15 are ordered by time: Groceries (no time, 00:00:00), Bread
/// (09:05), Morning coffee (10:30), although Morning coffee is written first.
const JOURNAL_ENTRIES: &[&[&str]] = &[
    &["0", "open", "1970-01-01", "NULL", "Assets:Bank"],
    &["1", "open", "1970-01-01", "NULL", "Assets:Bank:Savings"],
    &["2", "open", "1970-01-01", "NULL", "Assets:BankCard"],
    &["3", "open", "1970-01-01", "NULL", "Assets:Broker"],
    &["4", "open", "1970-01-01", "NULL", "Assets:Old"],
    &["5", "open", "1970-01-01", "NULL", "Expenses:Food"],
    &["6", "open", "1970-01-01", "NULL", "Expenses:Fun"],
    &["7", "open", "1970-01-01", "NULL", "Income:Salary"],
    &["8", "open", "1970-01-01", "NULL", "Income:Gains"],
    &["9", "commodity", "1970-01-01", "NULL", ""],
    &["10", "commodity", "1970-01-01", "NULL", ""],
    &["11", "transaction", "2023-12-31", "December salary", "Assets:Bank, Income:Salary"],
    &["12", "transaction", "2024-01-15", "Groceries", "Assets:Bank, Expenses:Food"],
    &["13", "transaction", "2024-01-15", "Bread", "Assets:Bank, Expenses:Food"],
    &["14", "transaction", "2024-01-15", "Morning coffee", "Assets:Bank, Expenses:Food"],
    &["15", "transaction", "2024-01-16", "Movie night", "Assets:Bank, Expenses:Food, Expenses:Fun"],
    &["16", "transaction", "2024-01-20", "Move to savings", "Assets:Bank, Assets:Bank:Savings"],
    &["17", "transaction", "2024-01-21", "Card payment", "Assets:BankCard, Expenses:Fun"],
    &["18", "transaction", "2024-02-01", "Buy AAPL", "Assets:Bank, Assets:Broker"],
    &["19", "transaction", "2024-02-02", "Buy AAPL again", "Assets:Bank, Assets:Broker"],
    &["20", "transaction", "2024-03-01", "Sell AAPL", "Assets:Bank, Assets:Broker, Income:Gains"],
    &["21", "close", "2024-03-02", "NULL", "Assets:Old"],
    &["22", "transaction", "2024-03-05", "Unbalanced", "Assets:Bank, Expenses:Food"],
    &["23", "transaction", "2024-03-06", "Closed account", "Assets:Old, Expenses:Food"],
    &["24", "transaction", "2024-03-07", "Closed and unbalanced", "Assets:Old, Expenses:Food"],
    &["25", "transaction", "2024-03-10", "Only a narration", "Assets:Bank, Expenses:Food"],
    &["26", "transaction", "2024-03-11", "ÄPFEL vom Markt", "Assets:Bank, Expenses:Food"],
];

/// READING: `seq` is the 0-based row number of the entry in the unfiltered `#entries` table, so
/// the seqs of `#entries` are exactly 0..n-1 and a transaction's seq counts the opens,
/// commodities and other entries before it.
#[test]
fn d1_seq_is_the_position_in_the_entries_order() {
    assert_eq!(
        query(journal(), "SELECT seq, type, date, narration, accounts FROM #entries"),
        rows(JOURNAL_ENTRIES)
    );
    // a transaction has its entry's seq in #transactions and on every one of its postings
    let transactions = JOURNAL_ENTRIES
        .iter()
        .filter(|row| row[1] == "transaction")
        .map(|row| vec![row[0].to_owned(), row[3].to_owned()])
        .collect::<Vec<_>>();
    assert_eq!(query(journal(), "SELECT seq, narration FROM #transactions"), owned(transactions.clone()));
    assert_eq!(
        query(journal(), "SELECT DISTINCT seq, narration FROM #postings ORDER BY seq"),
        owned(transactions)
    );
    // the postings of a transaction share its seq: 2 postings each, 3 for Movie night, and 4
    // for Sell AAPL, whose sale is booked as two lots
    assert_eq!(
        query(journal(), "SELECT seq, count(*) FROM #postings GROUP BY seq ORDER BY seq"),
        rows(&[
            &["11", "2"],
            &["12", "2"],
            &["13", "2"],
            &["14", "2"],
            &["15", "3"],
            &["16", "2"],
            &["17", "2"],
            &["18", "2"],
            &["19", "2"],
            &["20", "4"],
            &["22", "2"],
            &["23", "2"],
            &["24", "2"],
            &["25", "2"],
            &["26", "2"],
        ])
    );
    // ORDER BY seq DESC is newest first, also within a day
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions ORDER BY seq DESC LIMIT 4"),
        rows(&[&["ÄPFEL vom Markt"], &["Only a narration"], &["Closed and unbalanced"], &["Closed account"]])
    );
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE date = 2024-01-15 ORDER BY seq DESC"),
        rows(&[&["Morning coffee"], &["Bread"], &["Groceries"]])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT DISTINCT narration, seq FROM #postings WHERE date = 2024-01-15 ORDER BY seq DESC"
        ),
        rows(&[&["Morning coffee", "14"], &["Bread", "13"], &["Groceries", "12"]])
    );
}

/// The assertions ledger has balance entries and the padding transaction of a
/// `balance ... with pad` (flag P): 5 opens (seq 0-4), 4 commodities (5-8), Opening (9),
/// the balances of 01-02 to 01-06 (10-15), on 01-10 the balance (16) before the Market
/// transaction of the same day (17), the balance of 01-11 (18), on 01-15 the balance (19) and
/// its padding transaction (20), Buy (21), the balance of 01-21 (22). The correcting
/// transactions zhang inserts after balance checks are not entries and have no seq.
#[test]
fn d1_seq_counts_balances_and_padding_transactions() {
    assert_eq!(
        query(assertions(), "SELECT seq FROM #entries"),
        (0..23).map(|seq| vec![seq.to_string()]).collect::<Vec<_>>()
    );
    assert_eq!(
        query(
            assertions(),
            "SELECT seq, type, flag, narration FROM #entries WHERE date >= 2024-01-10 AND date <= 2024-01-15"
        ),
        rows(&[
            &["16", "balance", "NULL", "NULL"],
            &["17", "transaction", "*", "Market"],
            &["18", "balance", "NULL", "NULL"],
            &["19", "balance", "NULL", "NULL"],
            &["20", "transaction", "P", "pad Assets:Cash to Equity:Opening"],
        ])
    );
    assert_eq!(
        query(assertions(), "SELECT DISTINCT seq, flag FROM #postings WHERE date = 2024-01-15"),
        rows(&[&["20", "P"]])
    );
}

#[test]
fn d1_transactions_have_the_id_of_their_postings_and_of_the_store() {
    // the same id in #transactions, on every posting and in #entries
    let transactions = query(journal(), "SELECT seq, narration, id FROM #transactions");
    assert_eq!(transactions.len(), 15);
    assert_eq!(query(journal(), "SELECT DISTINCT seq, narration, id FROM #postings ORDER BY seq"), transactions);
    assert_eq!(
        query(journal(), "SELECT seq, narration, id FROM #entries WHERE type = 'transaction'"),
        transactions
    );
    // the ids are distinct, and they are the store's transaction ids (what /api/journals returns)
    let ids = transactions.iter().map(|row| row[2].clone()).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 15);
    let store = journal().store.read().unwrap();
    let store_ids = store.transactions.keys().map(|id| id.to_string()).collect::<BTreeSet<_>>();
    assert!(ids.is_subset(&store_ids), "{:?} is not in the store's {:?}", ids, store_ids);
    drop(store);
    // the txn_id an error records is the transaction's id
    assert_eq!(
        query(
            journal(),
            "SELECT meta('txn_id') FROM #errors WHERE kind = 'UnbalancedTransaction' AND date = 2024-03-05"
        ),
        vec![vec![transaction_id(journal(), "Unbalanced")]]
    );
    // the id filters a transaction's postings
    let id = transaction_id(journal(), "Movie night");
    assert_eq!(
        query_with(
            journal(),
            "SELECT account FROM #postings WHERE id = :id",
            &Params::new().bind("id", id.as_str())
        ),
        rows(&[&["Assets:Bank"], &["Expenses:Fun"], &["Expenses:Food"]])
    );
}

/// `posting_index` is the index of the posting as written: in Movie night the implicit
/// posting is written first, and the sale of Sell AAPL (written second) is booked as two lots,
/// -5 from the lot of 2024-02-01 and -2 from the lot of 2024-02-02 (FIFO), which share index 1.
#[test]
fn d1_posting_index_is_the_written_index_shared_by_lot_splits() {
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account, posting_index, position FROM #postings WHERE narration IN ('Movie night', 'Sell AAPL')"
        ),
        rows(&[
            &["Movie night", "Assets:Bank", "0", "-95 CNY"],
            &["Movie night", "Expenses:Fun", "1", "80 CNY"],
            &["Movie night", "Expenses:Food", "2", "15 CNY"],
            &["Sell AAPL", "Assets:Bank", "0", "840 CNY"],
            &["Sell AAPL", "Assets:Broker", "1", "-5 AAPL {100 CNY, 2024-02-01}"],
            &["Sell AAPL", "Assets:Broker", "1", "-2 AAPL {100 CNY, 2024-02-02}"],
            &["Sell AAPL", "Income:Gains", "2", "-140 CNY"],
        ])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT posting_index, count(*), sum(number) FROM #postings WHERE narration = 'Sell AAPL' GROUP BY posting_index ORDER BY posting_index"
        ),
        rows(&[&["0", "1", "840"], &["1", "2", "-7"], &["2", "1", "-140"]])
    );
    // the first posting of every transaction
    assert_eq!(query(journal(), "SELECT count(*) FROM #postings WHERE posting_index = 0"), rows(&[&["15"]]));
}

/// The postings of Assets:Bank in ledger order, and its balance right after each:
///
/// | narration        | position | account_balance |
/// |------------------|---------:|----------------:|
/// | December salary  |     1000 |            1000 |
/// | Groceries        |      -50 |             950 |
/// | Bread            |      -20 |             930 |
/// | Morning coffee   |      -30 |             900 |
/// | Movie night      |      -95 |             805 | (the implicit posting: -(80 + 15))
/// | Move to savings  |     -100 |             705 |
/// | Buy AAPL         |     -500 |             205 |
/// | Buy AAPL again   |     -500 |            -295 |
/// | Sell AAPL        |      840 |             545 |
/// | Unbalanced       |       -9 |             536 |
/// | Only a narration |       -1 |             535 |
/// | ÄPFEL vom Markt  |       -2 |             533 |
#[test]
fn d1_account_balance_runs_over_every_posting_of_the_account_whatever_the_where() {
    // hiding Bread: account_balance still counts its -20; `balance` runs over the selected rows
    assert_eq!(
        query(
            journal(),
            "SELECT narration, position, account_balance, balance WHERE account = 'Assets:Bank' AND narration != 'Bread'"
        ),
        rows(&[
            &["December salary", "1000 CNY", "1000 CNY", "1000 CNY"],
            &["Groceries", "-50 CNY", "950 CNY", "950 CNY"],
            &["Morning coffee", "-30 CNY", "900 CNY", "920 CNY"],
            &["Movie night", "-95 CNY", "805 CNY", "825 CNY"],
            &["Move to savings", "-100 CNY", "705 CNY", "725 CNY"],
            &["Buy AAPL", "-500 CNY", "205 CNY", "225 CNY"],
            &["Buy AAPL again", "-500 CNY", "-295 CNY", "-275 CNY"],
            &["Sell AAPL", "840 CNY", "545 CNY", "565 CNY"],
            &["Unbalanced", "-9 CNY", "536 CNY", "556 CNY"],
            &["Only a narration", "-1 CNY", "535 CNY", "555 CNY"],
            &["ÄPFEL vom Markt", "-2 CNY", "533 CNY", "553 CNY"],
        ])
    );
    // a date range, as an account page with a period asks: the balances include everything before
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account_balance WHERE account = 'Assets:Bank' AND date >= 2024-03-01"
        ),
        rows(&[
            &["Sell AAPL", "545 CNY"],
            &["Unbalanced", "536 CNY"],
            &["Only a narration", "535 CNY"],
            &["ÄPFEL vom Markt", "533 CNY"],
        ])
    );
    // ORDER BY and LIMIT do not change it: the newest two postings
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account_balance WHERE account = 'Assets:Bank' ORDER BY seq DESC LIMIT 2"
        ),
        rows(&[&["ÄPFEL vom Markt", "533 CNY"], &["Only a narration", "535 CNY"]])
    );
    // nor does a FROM filter without OPEN/CLOSE/CLEAR: the 2023 salary still counts
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account_balance FROM year = 2024 WHERE account = 'Assets:Bank' LIMIT 2"
        ),
        rows(&[&["Groceries", "950 CNY"], &["Bread", "930 CNY"]])
    );
    // the same account given as a parameter
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration, account_balance WHERE account = :account AND narration = 'Movie night'",
            &Params::new().bind("account", "Assets:Bank")
        ),
        rows(&[&["Movie night", "805 CNY"]])
    );
}

/// Each row's account_balance is the balance of that row's own account: in Movie night,
/// Assets:Bank is at 805 (above), Expenses:Fun at 80 (its first posting), and Expenses:Food at
/// 30 + 50 + 20 + 15 = 115 (Morning coffee, Groceries, Bread, then this posting). A child
/// account is its own account: Move to savings puts Assets:Bank:Savings at 100.
#[test]
fn d1_account_balance_is_the_balance_of_the_postings_own_account() {
    assert_eq!(
        query(journal(), "SELECT account, account_balance WHERE narration = 'Movie night'"),
        rows(&[&["Assets:Bank", "805 CNY"], &["Expenses:Fun", "80 CNY"], &["Expenses:Food", "115 CNY"]])
    );
    assert_eq!(
        query(journal(), "SELECT account, account_balance WHERE narration = 'Move to savings'"),
        rows(&[&["Assets:Bank:Savings", "100 CNY"], &["Assets:Bank", "705 CNY"]])
    );
    // a WHERE on another account does not leak into this one
    assert_eq!(
        query(journal(), "SELECT account, account_balance WHERE account = 'Expenses:Fun'"),
        rows(&[&["Expenses:Fun", "80 CNY"], &["Expenses:Fun", "90 CNY"]])
    );
}

/// Assets:Broker holds lots: 5 AAPL bought 2024-02-01, 5 more 2024-02-02, then the FIFO sale
/// of 7 takes the first lot (-5) and 2 of the second, leaving 3 AAPL of 2024-02-02.
#[test]
fn d1_account_balance_follows_lot_splits() {
    assert_eq!(
        query(journal(), "SELECT position, account_balance WHERE account = 'Assets:Broker'"),
        rows(&[
            &["5 AAPL {100 CNY, 2024-02-01}", "5 AAPL {100 CNY, 2024-02-01}"],
            &["5 AAPL {100 CNY, 2024-02-02}", "5 AAPL {100 CNY, 2024-02-01}, 5 AAPL {100 CNY, 2024-02-02}"],
            &["-5 AAPL {100 CNY, 2024-02-01}", "5 AAPL {100 CNY, 2024-02-02}"],
            &["-2 AAPL {100 CNY, 2024-02-02}", "3 AAPL {100 CNY, 2024-02-02}"],
        ])
    );
    // hiding the first purchase still counts its lot
    assert_eq!(
        query(
            journal(),
            "SELECT narration, units(account_balance) WHERE account = 'Assets:Broker' AND narration != 'Buy AAPL'"
        ),
        rows(&[&["Buy AAPL again", "10 AAPL"], &["Sell AAPL", "5 AAPL"], &["Sell AAPL", "3 AAPL"]])
    );
}

/// With `FROM OPEN ON 2024-02-01` the rows before the period are replaced by opening balances:
/// Assets:Bank opens with one summarized posting of 705 (its balance at the end of January),
/// and the 115 that Expenses:Food spent in January (Morning coffee 30, Groceries 50, Bread 20,
/// Movie night 15) is moved to the earnings equity account, so Expenses:Food starts the period
/// at zero. account_balance runs over these transformed rows, whatever the WHERE hides.
#[test]
fn d1_account_balance_runs_over_the_period_transformed_rows() {
    // the opening posting (flag S) is hidden, and still counted
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account_balance, balance FROM OPEN ON 2024-02-01 WHERE account = 'Assets:Bank' AND flag != 'S'"
        ),
        rows(&[
            &["Buy AAPL", "205 CNY", "-500 CNY"],
            &["Buy AAPL again", "-295 CNY", "-1000 CNY"],
            &["Sell AAPL", "545 CNY", "-160 CNY"],
            &["Unbalanced", "536 CNY", "-169 CNY"],
            &["Only a narration", "535 CNY", "-170 CNY"],
            &["ÄPFEL vom Markt", "533 CNY", "-172 CNY"],
        ])
    );
    // Expenses:Food from zero: Unbalanced 10 (hidden), Closed account 5, Closed and unbalanced 7,
    // Only a narration 1, ÄPFEL vom Markt 2
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account_balance FROM OPEN ON 2024-02-01 WHERE account = 'Expenses:Food' AND narration != 'Unbalanced'"
        ),
        rows(&[
            &["Closed account", "15 CNY"],
            &["Closed and unbalanced", "22 CNY"],
            &["Only a narration", "23 CNY"],
            &["ÄPFEL vom Markt", "25 CNY"],
        ])
    );
}

/// The store recorded UnbalancedTransaction for Unbalanced (10 vs -9), AccountClosed for
/// Closed account (Assets:Old is closed on 2024-03-02), and both for Closed and unbalanced
/// (7 vs -6 on the closed account). The other transactions have no error.
#[test]
fn d1_balanced_and_errors_of_transactions_and_postings() {
    assert_eq!(
        query(journal(), "SELECT narration, balanced, errors FROM #transactions WHERE date >= 2024-03-01"),
        rows(&[
            &["Sell AAPL", "TRUE", ""],
            &["Unbalanced", "FALSE", "UnbalancedTransaction"],
            &["Closed account", "TRUE", "AccountClosed"],
            &["Closed and unbalanced", "FALSE", "AccountClosed, UnbalancedTransaction"],
            &["Only a narration", "TRUE", ""],
            &["ÄPFEL vom Markt", "TRUE", ""],
        ])
    );
    // every posting carries its transaction's values
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account, balanced, errors FROM #postings WHERE date >= 2024-03-05 AND date <= 2024-03-07"
        ),
        rows(&[
            &["Unbalanced", "Expenses:Food", "FALSE", "UnbalancedTransaction"],
            &["Unbalanced", "Assets:Bank", "FALSE", "UnbalancedTransaction"],
            &["Closed account", "Expenses:Food", "TRUE", "AccountClosed"],
            &["Closed account", "Assets:Old", "TRUE", "AccountClosed"],
            &["Closed and unbalanced", "Expenses:Food", "FALSE", "AccountClosed, UnbalancedTransaction"],
            &["Closed and unbalanced", "Assets:Old", "FALSE", "AccountClosed, UnbalancedTransaction"],
        ])
    );
    // the set and the flag filter and count
    assert_eq!(query(journal(), "SELECT count(*) FROM #transactions WHERE NOT balanced"), rows(&[&["2"]]));
    assert_eq!(query(journal(), "SELECT count(*) FROM #transactions WHERE balanced"), rows(&[&["13"]]));
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE 'AccountClosed' IN errors"),
        rows(&[&["Closed account"], &["Closed and unbalanced"]])
    );
    // an error-free transaction has an empty set, not NULL
    assert_eq!(
        query(journal(), "SELECT count(*) FROM #transactions WHERE length(errors) = 0 AND errors IS NOT NULL"),
        rows(&[&["12"]])
    );
    // the error kinds are those of #errors.kind
    assert_eq!(
        query(journal(), "SELECT DISTINCT kind FROM #errors ORDER BY kind"),
        rows(&[&["AccountClosed"], &["UnbalancedTransaction"]])
    );
    // a posting to an account that does not exist is an error of a balanced transaction
    assert_eq!(
        query(errors_ledger(), "SELECT narration, balanced, errors FROM #transactions"),
        rows(&[&["typo", "TRUE", "AccountDoesNotExist"], &["unbalanced", "FALSE", "UnbalancedTransaction"]])
    );
}

/// The per-ledger dataset cache never serves stale rows: a reloaded ledger is queried afresh,
/// also by a query compiled before the reload, and two ledgers loaded from the same text do
/// not share rows.
#[test]
fn d1_a_reloaded_ledger_is_queried_afresh() {
    const HEADER: &str = "option \"timezone\" \"Asia/Shanghai\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Bank\n1970-01-01 open Income:Salary\n\
                          2024-01-01 * \"Employer\" \"January\"\n  Assets:Bank 100 CNY\n  Income:Salary -100 CNY\n";
    let write = |dir: &std::path::Path, text: &str| std::fs::write(dir.join("main.zhang"), text).unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write(first.path(), HEADER);
    write(second.path(), HEADER);
    let mut ledger = common::load_ledger(first.path().to_path_buf(), "main.zhang");
    let other = common::load_ledger(second.path().to_path_buf(), "main.zhang");

    let compiled = Query::compile("SELECT narration, position, balance, account_balance, seq FROM #postings WHERE account = 'Assets:Bank'").unwrap();
    let run = |ledger: &Ledger| cells(&compiled.execute_with_options(ledger, &Params::new(), &options()).unwrap());
    // 3 opens/commodities before the transaction: open, open, commodity, then January (seq 3)
    let before = rows(&[&["January", "100 CNY", "100 CNY", "100 CNY", "3"]]);
    assert_eq!(run(&ledger), before);
    assert_eq!(run(&ledger), before, "an unchanged ledger gives the same rows again");
    assert_eq!(run(&other), before);

    write(
        first.path(),
        &format!(
            "{}2024-02-01 * \"Employer\" \"February\"\n  Assets:Bank 150 CNY\n  Income:Salary -150 CNY\n",
            HEADER
        ),
    );
    ledger.reload().unwrap();
    assert_eq!(
        run(&ledger),
        rows(&[
            &["January", "100 CNY", "100 CNY", "100 CNY", "3"],
            &["February", "150 CNY", "250 CNY", "250 CNY", "4"]
        ])
    );
    assert_eq!(
        query(&ledger, "SELECT narration, id FROM #transactions").len(),
        2,
        "the transactions table of the reloaded ledger"
    );
    // the ids of the new rows are the reloaded store's transaction ids
    let store_ids = ledger
        .store
        .read()
        .unwrap()
        .transactions
        .keys()
        .map(|id| id.to_string())
        .collect::<BTreeSet<_>>();
    for row in query(&ledger, "SELECT id FROM #transactions") {
        assert!(store_ids.contains(&row[0]), "{} is not a transaction of the reloaded store", row[0]);
    }
    // the other ledger is unchanged
    assert_eq!(run(&other), before);
}

/// Account-scoped scans restrict booking to the accounts of a WHERE conjunct
/// `account = ...` or `account IN ...`; the rows, running balances, lots and aggregates are those
/// of a full scan, written here with regular expressions that cannot be scoped.
#[test]
fn d1_account_scoped_queries_give_the_results_of_a_full_scan() {
    const COLUMNS: &str = "date, narration, account, position, cost_number, cost_date, price, weight, balance, other_accounts";
    for account in ["Assets:Bank", "Assets:Broker", "Expenses:Food", "Assets:Bank:Savings", "Assets:Nope"] {
        let full = query(journal(), &format!("SELECT {} WHERE account ~ '^{}$'", COLUMNS, account));
        assert_eq!(
            query(journal(), &format!("SELECT {} WHERE account = '{}'", COLUMNS, account)),
            full,
            "{}",
            account
        );
        assert_eq!(
            query_with(
                journal(),
                &format!("SELECT {} WHERE account = :account", COLUMNS),
                &Params::new().bind("account", account)
            ),
            full,
            ":account = {}",
            account
        );
        assert_eq!(
            query(journal(), &format!("SELECT {} WHERE '{}' = account", COLUMNS, account)),
            full,
            "'{}' = account",
            account
        );
    }
    let full = query(journal(), &format!("SELECT {} WHERE account ~ '^(Assets:Bank|Assets:Broker)$'", COLUMNS));
    assert_eq!(
        query(journal(), &format!("SELECT {} WHERE account IN ('Assets:Bank', 'Assets:Broker')", COLUMNS)),
        full
    );
    assert_eq!(
        query_with(
            journal(),
            &format!("SELECT {} WHERE account IN :accounts", COLUMNS),
            &Params::new().bind("accounts", set(&["Assets:Bank", "Assets:Broker"]))
        ),
        full
    );
    // a conjunct among others, and a disjunction, which cannot be scoped
    assert_eq!(
        query(
            journal(),
            &format!("SELECT {} WHERE date >= 2024-02-01 AND account = 'Assets:Bank' AND number < 0", COLUMNS)
        ),
        query(
            journal(),
            &format!("SELECT {} WHERE date >= 2024-02-01 AND account ~ '^Assets:Bank$' AND number < 0", COLUMNS)
        )
    );
    assert_eq!(
        query(
            journal(),
            &format!("SELECT {} WHERE account = 'Assets:Bank' OR narration = 'Card payment'", COLUMNS)
        ),
        query(
            journal(),
            &format!("SELECT {} WHERE account ~ '^Assets:Bank$' OR narration = 'Card payment'", COLUMNS)
        )
    );
    // aggregates over the lots of a scoped account
    assert_eq!(
        query(
            journal(),
            "SELECT account, sum(position), sum(cost(position)), count(*) WHERE account = 'Assets:Broker' GROUP BY account"
        ),
        rows(&[&["Assets:Broker", "3 AAPL {100 CNY, 2024-02-02}", "300 CNY", "4"]])
    );
    // account_balance in a scoped IN: Movie night (Expenses:Fun 80), Move to savings
    // (Assets:Bank:Savings 100), Card payment (Expenses:Fun 80 + 10 = 90)
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration, account, account_balance WHERE account IN :accounts",
            &Params::new().bind("accounts", set(&["Assets:Bank:Savings", "Expenses:Fun"]))
        ),
        rows(&[
            &["Movie night", "Expenses:Fun", "80 CNY"],
            &["Move to savings", "Assets:Bank:Savings", "100 CNY"],
            &["Card payment", "Expenses:Fun", "90 CNY"],
        ])
    );
}

// =======================================================================================
// track D2: #balances.actual and passed (assertions/main.zhang)

/// The true balance of each assertion, the sum of every posting before it (a transaction of the
/// assertion's own day comes after it, as beancount checks at the start of the day), and
/// whether `|actual - amount| <= tolerance` (no tolerance: 0). zhang's correcting transactions
/// (flag C) are not counted, so a failed assertion does not move the balance of later ones.
///
/// - Assets:Bank holds 10.004 CNY and 20 USD from Opening (2024-01-01).
/// - 01-02 `10 ~ 0.01`: |10.004 - 10| = 0.004 <= 0.01: passed (within the tolerance).
/// - 01-03 `10.014 ~ 0.01`: |10.004 - 10.014| = 0.01, exactly the tolerance: passed.
/// - 01-04 `10.015 ~ 0.01`: 0.011 > 0.01: failed.
/// - 01-05 `10`: 0.004 > 0: failed.
/// - 01-06 `20 USD`: the account's USD only: passed. `0 EUR`, a currency the account never
///   held: 0 EUR, passed.
/// - 01-10 `10.004`: the Market transaction of 01-10 (-4) is not counted yet: passed.
/// - 01-11 `6.004`: 10.004 - 4: passed.
/// - 01-15 Assets:Cash `100 with pad`: the padding transaction fills the account to 100 before
///   the check. READING: a `balance ... with pad` counts its own padding transaction, so it
///   always passes, as zhang's balance-check stage sees it.
/// - 01-21 Assets:Broker `10 AAPL`: the units of the lot bought at cost on 01-20: passed.
const ASSERTIONS: &[&[&str]] = &[
    &["2024-01-02", "Assets:Bank", "10 CNY", "0.01", "10.004 CNY", "TRUE"],
    &["2024-01-03", "Assets:Bank", "10.014 CNY", "0.01", "10.004 CNY", "TRUE"],
    &["2024-01-04", "Assets:Bank", "10.015 CNY", "0.01", "10.004 CNY", "FALSE"],
    &["2024-01-05", "Assets:Bank", "10 CNY", "NULL", "10.004 CNY", "FALSE"],
    &["2024-01-06", "Assets:Bank", "20 USD", "NULL", "20 USD", "TRUE"],
    &["2024-01-06", "Assets:Bank", "0 EUR", "NULL", "0 EUR", "TRUE"],
    &["2024-01-10", "Assets:Bank", "10.004 CNY", "NULL", "10.004 CNY", "TRUE"],
    &["2024-01-11", "Assets:Bank", "6.004 CNY", "NULL", "6.004 CNY", "TRUE"],
    &["2024-01-15", "Assets:Cash", "100 CNY", "NULL", "100 CNY", "TRUE"],
    &["2024-01-21", "Assets:Broker", "10 AAPL", "NULL", "10 AAPL", "TRUE"],
];

#[test]
fn d2_balances_report_the_true_balance_and_whether_the_assertion_passed() {
    assert_eq!(
        query(assertions(), "SELECT date, account, amount, tolerance, actual, passed FROM #balances"),
        rows(ASSERTIONS)
    );
    // the failed assertions, as an errors page or an account page lists them
    assert_eq!(
        query(assertions(), "SELECT date, amount, actual FROM #balances WHERE NOT passed"),
        rows(&[&["2024-01-04", "10.015 CNY", "10.004 CNY"], &["2024-01-05", "10 CNY", "10.004 CNY"]])
    );
    // actual is an amount in the assertion's currency
    assert_eq!(
        query(assertions(), "SELECT DISTINCT currency(actual) = currency(amount) FROM #balances"),
        rows(&[&["TRUE"]])
    );
    // the assertions that pass only thanks to their tolerance
    assert_eq!(
        query(assertions(), "SELECT date FROM #balances WHERE passed AND number(actual) != number(amount)"),
        rows(&[&["2024-01-02"], &["2024-01-03"]])
    );
}

// =======================================================================================
// track D2: #documents (documents/main.zhang and documents/sub/more.zhang)

/// Rows: the document directives first, in ledger order (the one of sub/more.zhang is dated
/// 2024-01-05, before the one of main.zhang), then one row per `document:` value of a
/// transaction and of each of its postings. `account` is NULL for a transaction's documents
/// and the posting's account for a posting's; `date` is the transaction's date.
///
/// READING: `path` is the document path as written, which zhang keeps relative to the ledger
/// root and the download endpoint `/api/documents/:b64` joins to the root (it is the `path`
/// of `GET /api/documents` today). For the directive of sub/more.zhang it is therefore
/// `docs/contract.pdf`, while beancount's `filename` resolves it against the declaring file's
/// directory, `<root>/sub/docs/contract.pdf`.
///
/// READING: the metadata rows of a transaction follow the transaction's own values, then each
/// posting's in written order, and transactions come in ledger order.
#[test]
fn d2_documents_list_directives_then_transaction_and_posting_metadata() {
    let receipts = transaction_id(documents(), "Receipts");
    let sub = transaction_id(documents(), "Receipt in a sub file");
    let (receipts, sub) = (receipts.as_str(), sub.as_str());
    assert_eq!(
        query(documents(), "SELECT date, account, source, path, transaction_id FROM #documents"),
        rows(&[
            &["2024-01-05", "Assets:Bank", "directive", "docs/contract.pdf", "NULL"],
            &["2024-01-10", "Assets:Bank", "directive", "statements/2024-01.pdf", "NULL"],
            &["2024-01-11", "NULL", "transaction", "attachments/receipt-1.pdf", receipts],
            &["2024-01-11", "NULL", "transaction", "attachments/receipt-2.pdf", receipts],
            &["2024-01-11", "Expenses:Food", "posting", "attachments/item-1.jpg", receipts],
            &["2024-01-11", "Expenses:Food", "posting", "attachments/item-2.jpg", receipts],
            &["2024-01-13", "Assets:Bank", "posting", "attachments/card-slip.pdf", sub],
        ])
    );

    // filename keeps beancount's resolution, relative to the declaring file (the ledger root
    // is canonicalized when the ledger is loaded)
    let root = fixture_dir("documents").canonicalize().unwrap();
    let absolute = |relative: &str| root.join(relative).to_string_lossy().into_owned();
    assert_eq!(
        query(documents(), "SELECT filename, path FROM #documents WHERE source = 'directive'"),
        vec![
            vec![absolute("sub/docs/contract.pdf"), "docs/contract.pdf".to_owned()],
            vec![absolute("statements/2024-01.pdf"), "statements/2024-01.pdf".to_owned()],
        ]
    );
}

/// READING: a metadata row's `filename` is resolved like a directive's, against the directory
/// of the file holding the transaction; main.zhang is at the root. (The row of
/// sub/more.zhang is left out: whether `filename` should then resolve against `sub/` or keep
/// zhang's root-relative convention is for the lead to rule.)
#[test]
fn d2_document_metadata_rows_resolve_filename_like_directives() {
    let root = fixture_dir("documents").canonicalize().unwrap();
    let absolute = |relative: &str| root.join(relative).to_string_lossy().into_owned();
    let receipts = transaction_id(documents(), "Receipts");
    assert_eq!(
        query_with(
            documents(),
            "SELECT filename FROM #documents WHERE transaction_id = :id",
            &Params::new().bind("id", receipts.as_str())
        ),
        vec![
            vec![absolute("attachments/receipt-1.pdf")],
            vec![absolute("attachments/receipt-2.pdf")],
            vec![absolute("attachments/item-1.jpg")],
            vec![absolute("attachments/item-2.jpg")],
        ]
    );
}

/// `path` and `transaction_id` are exactly what the store knows of every document, the data
/// behind `GET /api/documents` and the download endpoint.
#[test]
fn d2_documents_paths_are_the_store_document_paths() {
    let mut table = query(documents(), "SELECT path, transaction_id FROM #documents");
    table.sort();
    let store = documents().store.read().unwrap();
    let mut known = store
        .documents
        .iter()
        .map(|document| vec![normalize(&document.path), document.document_type.as_trx().unwrap_or_else(|| "NULL".to_owned())])
        .collect::<Vec<_>>();
    known.sort();
    assert_eq!(table, known);
}

#[test]
fn d2_documents_group_filter_and_stay_empty_without_documents() {
    assert_eq!(
        query(documents(), "SELECT source, count(*) FROM #documents GROUP BY source ORDER BY source"),
        rows(&[&["directive", "2"], &["posting", "3"], &["transaction", "2"]])
    );
    // the documents of an account: its directives and its postings' documents
    assert_eq!(
        query(documents(), "SELECT path FROM #documents WHERE account = 'Assets:Bank'"),
        rows(&[&["docs/contract.pdf"], &["statements/2024-01.pdf"], &["attachments/card-slip.pdf"]])
    );
    // a ledger without documents has an empty table; like beanquery (conformance case
    // 031_aggregate_over_no_rows), an aggregate over no rows gives no row
    assert_eq!(query(journal(), "SELECT count(*) FROM #documents"), rows(&[]));
    assert_eq!(query(journal(), "SELECT path FROM #documents"), rows(&[]));
}

// =======================================================================================
// track D2: #errors ids and spans (errors/main.zhang, errors/more.zhang)

/// The three errors, by file then position: the second `operating_currency` option of
/// main.zhang (MultipleOperatingCurrencyDetect), the typo transaction at the very start of
/// more.zhang (AccountDoesNotExist) and the unbalanced one after it. `span_start` is the byte
/// offset of the directive in its own file, and the bytes `span_start..span_end` are the
/// directive's text (`source`, which drops the trailing line break).
#[test]
fn d2_errors_have_the_store_id_and_the_byte_span_of_their_directive() {
    let dir = fixture_dir("errors");
    let main = std::fs::read_to_string(dir.join("main.zhang")).unwrap();
    let more = std::fs::read_to_string(dir.join("more.zhang")).unwrap();
    let result = query(errors_ledger(), "SELECT file, kind, id, span_start, span_end, source FROM #errors");
    assert_eq!(result.len(), 3, "{:?}", result);
    let expected = [
        (
            "main.zhang",
            "MultipleOperatingCurrencyDetect",
            &main,
            main.find("option \"operating_currency\" \"USD\"").unwrap(),
        ),
        ("more.zhang", "AccountDoesNotExist", &more, 0),
        (
            "more.zhang",
            "UnbalancedTransaction",
            &more,
            more.find("2024-02-02 * \"Shop\" \"unbalanced\"").unwrap(),
        ),
    ];
    let store = errors_ledger().store.read().unwrap();
    for (row, (file, kind, text, start)) in result.iter().zip(expected) {
        assert_eq!((row[0].as_str(), row[1].as_str()), (file, kind), "{:?}", row);
        // the store's id of the error of this kind at this position
        let store_id = store
            .errors
            .iter()
            .find(|error| error.error_type.to_string() == kind && error.span.as_ref().map(|span| span.start) == Some(start))
            .map(|error| error.id.clone())
            .unwrap_or_else(|| panic!("the store has no {} error at {}", kind, start));
        assert_eq!(row[2], store_id, "id of {}", kind);
        assert_eq!(row[3], start.to_string(), "span_start of {}", kind);
        let end = row[4].parse::<usize>().unwrap_or_else(|_| panic!("span_end of {} is {}", kind, row[4]));
        assert!(start < end && end <= text.len(), "{}: {}..{} in {} bytes", kind, start, end, text.len());
        assert_eq!(text[start..end].trim_end(), row[5], "the bytes of the span of {}", kind);
    }
    drop(store);
    // the id of a transaction's error matches what zhang recorded with it
    assert_eq!(
        query(errors_ledger(), "SELECT id = meta('txn_id') FROM #errors WHERE kind = 'UnbalancedTransaction'"),
        rows(&[&["TRUE"]])
    );
    // spans are integers to compute with, e.g. the length of the directive
    assert_eq!(
        query(errors_ledger(), "SELECT span_end - span_start > 0 FROM #errors WHERE file = 'more.zhang'"),
        rows(&[&["TRUE"], &["TRUE"]])
    );
    // a ledger without errors has no rows
    assert_eq!(query(documents(), "SELECT id, span_start, span_end FROM #errors"), rows(&[]));
}

/// Two errors of one directive (Closed and unbalanced) share the directive's span; each row
/// has the id the store gave that error.
#[test]
fn d2_errors_of_one_directive_share_its_span() {
    let rows_ = query(
        journal(),
        "SELECT kind, id, span_start, span_end FROM #errors WHERE date = 2024-03-07 ORDER BY kind",
    );
    assert_eq!(rows_.len(), 2, "{:?}", rows_);
    assert_eq!((rows_[0][0].as_str(), rows_[1][0].as_str()), ("AccountClosed", "UnbalancedTransaction"));
    assert_eq!((&rows_[0][2], &rows_[0][3]), (&rows_[1][2], &rows_[1][3]));
    let store = journal().store.read().unwrap();
    for row in &rows_ {
        let start = row[2].parse::<usize>().unwrap();
        assert!(
            store
                .errors
                .iter()
                .any(|error| error.error_type.to_string() == row[0] && error.id == row[1] && error.span.as_ref().map(|span| span.start) == Some(start)),
            "{:?} is not a store error",
            row
        );
    }
}

// =======================================================================================
// track D2: #budgets and #budget_events (budgets/main.zhang)

/// Activity converts every posting to the budget's commodity at the posting's date. The only
/// prices are CNY in USD (the inverse direction): 0.125 USD from 2024-01-01, so 8 CNY per USD,
/// and 0.1 USD from 2024-03-01, so 10 CNY per USD. Months are those of the ledger dates, in the
/// ledger's timezone.
///
/// food (Expenses:Food):
/// - 2024-01: assigned 0 + 1000 = 1000, added 1000, activity 100 CNY + 10 USD × 8 = 180,
///   available 820;
/// - 2024-02: assigned 820 - 200 (transferred to travel) = 620, added -200, activity 29 (the
///   leap day, the month's last day), available 591;
/// - 2024-03: assigned 591, added 0, activity 1 USD × 10 (2024-03-01 00:30, the price of that
///   day applies; it is still February in UTC) + 10 USD × 10 = 110, available 481;
/// - 2024-04: assigned 481, added 0, activity 50, available 431.
///
/// travel (Expenses:Travel, no spending): 500 in January; 50 added at 2024-02-01 00:30 (a
/// February budget-add, although it is 2024-01-31 in UTC) and 200 transferred in, so assigned
/// 500 + 250 = 750 from February. It is closed on 2024-03-20, so `closed` is FALSE in January
/// and February and TRUE from March on. Both series run through April, the ledger's last month
/// with a transaction.
///
/// The budget-add to `vacation`, a budget that does not exist, is an error and adds nothing.
#[test]
fn d2_budgets_convert_activity_at_the_posting_date_and_close_per_month() {
    // in April 2024, the fixture's last month: a budget's months also run through the current one
    let april = NaiveDate::from_ymd_opt(2024, 4, 30).unwrap();
    assert_eq!(
        query_on(
            budgets(),
            april,
            "SELECT name, date, assigned, added, activity, available, closed FROM #budgets"
        ),
        rows(&[
            &["food", "2024-01-01", "1000 CNY", "1000 CNY", "180 CNY", "820 CNY", "FALSE"],
            &["food", "2024-02-01", "620 CNY", "-200 CNY", "29 CNY", "591 CNY", "FALSE"],
            &["food", "2024-03-01", "591 CNY", "0 CNY", "110 CNY", "481 CNY", "FALSE"],
            &["food", "2024-04-01", "481 CNY", "0 CNY", "50 CNY", "431 CNY", "FALSE"],
            &["travel", "2024-01-01", "500 CNY", "500 CNY", "0 CNY", "500 CNY", "FALSE"],
            &["travel", "2024-02-01", "750 CNY", "250 CNY", "0 CNY", "750 CNY", "FALSE"],
            &["travel", "2024-03-01", "750 CNY", "0 CNY", "0 CNY", "750 CNY", "TRUE"],
            &["travel", "2024-04-01", "750 CNY", "0 CNY", "0 CNY", "750 CNY", "TRUE"],
        ])
    );
    // the current month of each budget, as the spec suggests for consumers
    assert_eq!(
        query_on(
            budgets(),
            april,
            "SELECT name, last(available), last(closed) FROM #budgets GROUP BY name ORDER BY name"
        ),
        rows(&[&["food", "431 CNY", "FALSE"], &["travel", "750 CNY", "TRUE"]])
    );
    // the open budgets of a month
    assert_eq!(
        query_on(budgets(), april, "SELECT name FROM #budgets WHERE date = 2024-02-01 AND NOT closed"),
        rows(&[&["food"], &["travel"]])
    );
    assert_eq!(
        query_on(budgets(), april, "SELECT name FROM #budgets WHERE date = 2024-03-01 AND NOT closed"),
        rows(&[&["food"]])
    );
    // the activity is in the budget's commodity and adds up: 180 + 29 + 110 + 50
    assert_eq!(
        query_on(budgets(), april, "SELECT name, sum(activity) FROM #budgets WHERE name = 'food' GROUP BY name"),
        rows(&[&["food", "369 CNY"]])
    );
    assert_eq!(
        query_on(budgets(), april, "SELECT DISTINCT name, currency(activity) FROM #budgets ORDER BY name"),
        rows(&[&["food", "CNY"], &["travel", "CNY"]])
    );
    // only defined budgets have rows
    assert_eq!(query(budgets(), "SELECT DISTINCT name FROM #budgets"), rows(&[&["food"], &["travel"]]));
}

/// One row per effect of a budget directive, in ledger order. The ledger's timezone is UTC+8:
/// - 2024-01-01 00:00:00 +08:00 is 1704067200 - 28800 = 1704038400, and its 09:30:00 is
///   1704038400 + 34200 = 1704072600;
/// - 2024-02-01 00:30:00 +08:00 is 31 days later plus 1800 s: 1704038400 + 2678400 + 1800 =
///   1706718600 (2024-01-31T16:30:00Z; the row's date is still the ledger date 2024-02-01);
/// - 2024-02-15 is 45 days after 2024-01-01: 1704038400 + 3888000 = 1707926400;
/// - 2024-03-20 is 79 days after 2024-01-01: 1704038400 + 6825600 = 1710864000.
///
/// The `budget` definitions are not effects. The transfer of 200 from food to travel takes
/// 200 out of food and puts 200 into travel. READING: the transfer's two rows come in the
/// order the directive names them (from, then to), and a `close` has a NULL amount since it
/// moves no money. READING: the budget-add to the undefined `vacation` budget, an error, has
/// no effect and so no row.
#[test]
fn d2_budget_events_list_every_effect_with_a_signed_amount() {
    assert_eq!(
        query(budgets(), "SELECT name, date, time, timestamp, type, amount FROM #budget_events"),
        rows(&[
            &["food", "2024-01-01", "00:00:00", "1704038400", "assign", "1000 CNY"],
            &["travel", "2024-01-01", "09:30:00", "1704072600", "assign", "500 CNY"],
            &["travel", "2024-02-01", "00:30:00", "1706718600", "assign", "50 CNY"],
            &["food", "2024-02-15", "00:00:00", "1707926400", "transfer_out", "-200 CNY"],
            &["travel", "2024-02-15", "00:00:00", "1707926400", "transfer_in", "200 CNY"],
            &["travel", "2024-03-20", "00:00:00", "1710864000", "close", "NULL"],
        ])
    );
    // the signed amounts add up to what #budgets says was added: food 1000 - 200, travel
    // 500 + 50 + 200
    let events = query(budgets(), "SELECT name, sum(amount) FROM #budget_events GROUP BY name ORDER BY name");
    assert_eq!(events, rows(&[&["food", "800 CNY"], &["travel", "750 CNY"]]));
    assert_eq!(events, query(budgets(), "SELECT name, sum(added) FROM #budgets GROUP BY name ORDER BY name"));
    assert_eq!(
        query(budgets(), "SELECT type, count(*) FROM #budget_events GROUP BY type ORDER BY type"),
        rows(&[&["assign", "3"], &["close", "1"], &["transfer_in", "1"], &["transfer_out", "1"]])
    );
    // a budget's history, newest first
    assert_eq!(
        query(
            budgets(),
            "SELECT date, type, amount FROM #budget_events WHERE name = 'travel' ORDER BY timestamp DESC"
        ),
        rows(&[
            &["2024-03-20", "close", "NULL"],
            &["2024-02-15", "transfer_in", "200 CNY"],
            &["2024-02-01", "assign", "50 CNY"],
            &["2024-01-01", "assign", "500 CNY"],
        ])
    );
    // the events of a month, by ledger date
    assert_eq!(
        query(budgets(), "SELECT name, type FROM #budget_events WHERE year(date) = 2024 AND month(date) = 2"),
        rows(&[&["travel", "assign"], &["food", "transfer_out"], &["travel", "transfer_in"]])
    );
}

// =======================================================================================
// track L: LIMIT / OFFSET with parameters. #transactions of the journal ledger, in the table's
// (ledger) order: December salary, Groceries, Bread, Morning coffee, Movie night, Move to
// savings, Card payment, Buy AAPL, Buy AAPL again, Sell AAPL, Unbalanced, Closed account,
// Closed and unbalanced, Only a narration, ÄPFEL vom Markt: 15 rows. The tests of track L do
// not use the columns of track D1, so that they run on the track L branch alone.

const IN_LEDGER_ORDER: &str = "SELECT narration FROM #transactions";

/// The same rows, by an ORDER BY: date, then narration (the three transactions of 2024-01-15
/// come Bread, Groceries, Morning coffee).
const BY_DATE: &str = "SELECT narration FROM #transactions ORDER BY date, narration";

fn narrations(names: &[&str]) -> Vec<Vec<String>> {
    names.iter().map(|it| vec![(*it).to_owned()]).collect()
}

#[test]
fn l_limit_offset_skips_rows_after_ordering() {
    assert_eq!(
        query(journal(), &format!("{} LIMIT 3 OFFSET 2", IN_LEDGER_ORDER)),
        narrations(&["Bread", "Morning coffee", "Movie night"])
    );
    assert_eq!(
        query(journal(), &format!("{} LIMIT 3 OFFSET 0", IN_LEDGER_ORDER)),
        query(journal(), &format!("{} LIMIT 3", IN_LEDGER_ORDER))
    );
    // the last page is short; OFFSET at or past the end gives no rows
    assert_eq!(
        query(journal(), &format!("{} LIMIT 5 OFFSET 13", IN_LEDGER_ORDER)),
        narrations(&["Only a narration", "ÄPFEL vom Markt"])
    );
    assert_eq!(query(journal(), &format!("{} LIMIT 5 OFFSET 15", IN_LEDGER_ORDER)), rows(&[]));
    assert_eq!(query(journal(), &format!("{} LIMIT 5 OFFSET 1000", IN_LEDGER_ORDER)), rows(&[]));
    assert_eq!(query(journal(), &format!("{} LIMIT 0 OFFSET 2", IN_LEDGER_ORDER)), rows(&[]));
    // after an ORDER BY: date, then narration
    assert_eq!(
        query(journal(), &format!("{} LIMIT 3 OFFSET 1", BY_DATE)),
        narrations(&["Bread", "Groceries", "Morning coffee"])
    );
    // descending pages, newest first
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions ORDER BY date DESC LIMIT 2 OFFSET 2"),
        narrations(&["Closed and unbalanced", "Closed account"])
    );
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions ORDER BY date DESC LIMIT 2 OFFSET 14"),
        narrations(&["December salary"])
    );
    // after DISTINCT: payees in order, NULL first: NULL, Bakery, Bank, Broker, Cafe, ...
    assert_eq!(
        query(journal(), "SELECT DISTINCT payee FROM #transactions ORDER BY payee LIMIT 3 OFFSET 1"),
        narrations(&["Bakery", "Bank", "Broker"])
    );
    // after GROUP BY: accounts with postings, in name order
    assert_eq!(
        query(
            journal(),
            "SELECT account, count(*) FROM #postings GROUP BY account ORDER BY account LIMIT 2 OFFSET 3"
        ),
        rows(&[&["Assets:Broker", "4"], &["Assets:Old", "2"]])
    );
    // on postings, a page of an account journal, newest first, with the running balance of
    // the selected rows (Assets:Bank ends at 533: 535 before ÄPFEL vom Markt, 536 before that)
    assert_eq!(
        query(
            journal(),
            "SELECT narration, balance FROM #postings WHERE account = 'Assets:Bank' ORDER BY date DESC LIMIT 2 OFFSET 1"
        ),
        rows(&[&["Only a narration", "535 CNY"], &["Unbalanced", "536 CNY"]])
    );
}

#[test]
fn l_limit_and_offset_take_parameters() {
    let page = |limit: i64, offset: i64| Params::new().bind("limit", limit).bind("offset", offset);
    let sql = format!("{} LIMIT :limit OFFSET :offset", IN_LEDGER_ORDER);
    assert_eq!(
        query_with(journal(), &sql, &page(3, 2)),
        narrations(&["Bread", "Morning coffee", "Movie night"])
    );
    assert_eq!(query_with(journal(), &sql, &page(5, 13)), narrations(&["Only a narration", "ÄPFEL vom Markt"]));
    assert_eq!(query_with(journal(), &sql, &page(5, 15)), rows(&[]));
    assert_eq!(query_with(journal(), &sql, &page(0, 0)), rows(&[]));
    // one compiled query, executed for several pages
    let compiled = Query::compile_with_params(&sql, &ParamTypes::new().bind("limit", DataType::Int).bind("offset", DataType::Int)).unwrap();
    let pages = (0..4)
        .map(|index| cells(&compiled.execute_with_options(journal(), &page(4, index * 4), &options()).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [4, 4, 4, 3]);
    assert_eq!(pages.concat(), query(journal(), IN_LEDGER_ORDER));
    // positional parameters, and a mix of a literal and a parameter
    assert_eq!(
        query_with(
            journal(),
            &format!("{} LIMIT $1 OFFSET $2", IN_LEDGER_ORDER),
            &Params::new().push(1i64).push(3i64)
        ),
        narrations(&["Morning coffee"])
    );
    assert_eq!(
        query_with(
            journal(),
            &format!("{} LIMIT 1 OFFSET :offset", IN_LEDGER_ORDER),
            &Params::new().bind("offset", 4i64)
        ),
        narrations(&["Movie night"])
    );
    assert_eq!(
        query_with(journal(), &format!("{} LIMIT :limit", IN_LEDGER_ORDER), &Params::new().bind("limit", 1i64)),
        narrations(&["December salary"])
    );
}

fn assert_kind(err: &QueryError, kinds: &[QueryErrorKind], context: &str) {
    assert!(kinds.contains(&err.kind), "{}: {:?} {}", context, err.kind, err.message);
    assert!(!err.message.is_empty(), "{}: empty message", context);
}

/// Negative or overflowing values are clean errors, never a panic or a wrapped-around offset.
#[test]
fn l_limit_offset_reject_negative_overflowing_and_mistyped_values() {
    // literals: parse or compile errors
    for sql in [
        format!("{} LIMIT -1", IN_LEDGER_ORDER),
        format!("{} LIMIT 1 OFFSET -1", IN_LEDGER_ORDER),
        format!("{} LIMIT 99999999999999999999", IN_LEDGER_ORDER),
        format!("{} LIMIT 1 OFFSET 99999999999999999999", IN_LEDGER_ORDER),
        format!("{} LIMIT 1 OFFSET 1.5", IN_LEDGER_ORDER),
        format!("{} LIMIT 'a'", IN_LEDGER_ORDER),
    ] {
        let err = query_error(journal(), &sql, &Params::new());
        assert_kind(&err, &[QueryErrorKind::Parse, QueryErrorKind::Compile], &sql);
    }
    // bound values: negative values fail when the query is executed with them
    let sql = format!("{} LIMIT :limit OFFSET :offset", IN_LEDGER_ORDER);
    for (limit, offset) in [(-1i64, 0i64), (1, -1), (i64::MIN, 0), (0, i64::MIN)] {
        let params = Params::new().bind("limit", limit).bind("offset", offset);
        let err = query_error(journal(), &sql, &params);
        assert_kind(
            &err,
            &[QueryErrorKind::Compile, QueryErrorKind::Eval],
            &format!("{} with {} {}", sql, limit, offset),
        );
    }
    // offset + limit beyond i64: an error or the correct (here: empty or complete) page, never
    // a wrapped-around one
    for (limit, offset, expected) in [
        (i64::MAX, i64::MAX, rows(&[])),
        (i64::MAX, 14, narrations(&["ÄPFEL vom Markt"])),
        (i64::MAX, 0, query(journal(), IN_LEDGER_ORDER)),
    ] {
        let params = Params::new().bind("limit", limit).bind("offset", offset);
        match try_query(journal(), &sql, &params) {
            Ok(result) => assert_eq!(cells(&result), expected, "LIMIT {} OFFSET {}", limit, offset),
            Err(err) => assert_kind(
                &err,
                &[QueryErrorKind::Compile, QueryErrorKind::Eval],
                &format!("LIMIT {} OFFSET {}", limit, offset),
            ),
        }
    }
    // parameters of the wrong type: compile errors when declared so...
    for ty in [DataType::Str, DataType::Decimal, DataType::Date, DataType::Bool] {
        let err = Query::compile_with_params(&sql, &ParamTypes::new().bind("limit", ty).bind("offset", DataType::Int))
            .err()
            .unwrap_or_else(|| panic!("LIMIT :limit compiled with a {} parameter", ty.name()));
        assert_kind(&err, &[QueryErrorKind::Compile], &format!("LIMIT of type {}", ty.name()));
    }
    // ...and execution errors when compiled as INT but bound to something else
    let compiled = Query::compile_with_params(&sql, &ParamTypes::new().bind("limit", DataType::Int).bind("offset", DataType::Int)).unwrap();
    let err = compiled
        .execute_with_options(journal(), &Params::new().bind("limit", "10").bind("offset", 0i64), &options())
        .unwrap_err();
    assert_kind(&err, &[QueryErrorKind::Compile, QueryErrorKind::Eval], "LIMIT bound to a string");
    // an unbound parameter
    let err = compiled
        .execute_with_options(journal(), &Params::new().bind("limit", 1i64), &options())
        .unwrap_err();
    assert_kind(&err, &[QueryErrorKind::Compile, QueryErrorKind::Eval], "OFFSET not bound");
    // READING: a NULL limit or offset is either a clean error or no limit / no offset (as in
    // PostgreSQL), never a panic
    for (limit, offset) in [(Value::Null, Value::Int(0)), (Value::Int(2), Value::Null)] {
        let params = Params::new().bind("limit", limit.clone()).bind("offset", offset.clone());
        match compiled.execute_with_options(journal(), &params, &options()) {
            Ok(result) => {
                let all = query(journal(), IN_LEDGER_ORDER);
                let expected = if limit.is_null() { all } else { all[..2].to_vec() };
                assert_eq!(cells(&result), expected, "LIMIT {} OFFSET {}", limit, offset);
            }
            Err(err) => assert_kind(&err, &[QueryErrorKind::Compile, QueryErrorKind::Eval], "NULL LIMIT/OFFSET"),
        }
    }
}

// =======================================================================================
// track L: bound parameters folded into constants give the same results as literals

#[test]
fn l_parameters_give_the_results_of_the_same_literals() {
    let same = |literal: &str, parameterized: &str, params: Params| {
        assert_eq!(query_with(journal(), parameterized, &params), query(journal(), literal), "{}", parameterized);
    };
    same(
        "SELECT narration, account FROM #postings WHERE narration ~ 'coffee|BREAD'",
        "SELECT narration, account FROM #postings WHERE narration ~ :re",
        Params::new().bind("re", "coffee|BREAD"),
    );
    same(
        "SELECT narration FROM #transactions WHERE payee ~ '^b' OR narration ~ '^b'",
        "SELECT narration FROM #transactions WHERE payee ~ :re OR narration ~ :re",
        Params::new().bind("re", "^b"),
    );
    same(
        "SELECT narration FROM #transactions WHERE icontains(narration, 'AAPL')",
        "SELECT narration FROM #transactions WHERE icontains(narration, :needle)",
        Params::new().bind("needle", "AAPL"),
    );
    same(
        "SELECT narration, account FROM #postings WHERE account IN ('Assets:Bank:Savings', 'Expenses:Fun', 'Nope')",
        "SELECT narration, account FROM #postings WHERE account IN :accounts",
        Params::new().bind("accounts", set(&["Assets:Bank:Savings", "Expenses:Fun", "Nope"])),
    );
    same(
        "SELECT narration, account FROM #postings WHERE account = 'Assets:Broker'",
        "SELECT narration, account FROM #postings WHERE account = $1",
        Params::new().push("Assets:Broker"),
    );
    // an IN over the ids of a page of transactions, as the journal endpoint asks
    let ids = query(journal(), "SELECT DISTINCT id FROM #postings WHERE date >= 2024-03-07")
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration, account FROM #postings WHERE id IN :ids",
            &Params::new().bind("ids", Value::Set(ids))
        ),
        rows(&[
            &["Closed and unbalanced", "Expenses:Food"],
            &["Closed and unbalanced", "Assets:Old"],
            &["Only a narration", "Expenses:Food"],
            &["Only a narration", "Assets:Bank"],
            &["ÄPFEL vom Markt", "Expenses:Food"],
            &["ÄPFEL vom Markt", "Assets:Bank"],
        ])
    );
    // an empty set matches nothing
    assert_eq!(
        query_with(
            journal(),
            "SELECT count(*) FROM #postings WHERE account IN :accounts",
            &Params::new().bind("accounts", set(&[]))
        ),
        rows(&[])
    );
}

/// The constants are folded per execution: one compiled query gives each execution the result
/// of its own parameters.
#[test]
fn l_folded_parameters_belong_to_one_execution() {
    let compiled = Query::compile_with_params(
        "SELECT narration FROM #transactions WHERE narration ~ :re AND icontains(payee, :needle)",
        &ParamTypes::new().bind("re", DataType::Str).bind("needle", DataType::Str),
    )
    .unwrap();
    let run = |re: &str, needle: &str| {
        cells(
            &compiled
                .execute_with_options(journal(), &Params::new().bind("re", re).bind("needle", needle), &options())
                .unwrap(),
        )
    };
    assert_eq!(run("coffee", "CAFE"), narrations(&["Morning coffee"]));
    assert_eq!(run("bread", "bak"), narrations(&["Bread"]));
    assert_eq!(run("coffee", "bak"), rows(&[]));
    assert_eq!(run("AAPL", "broker"), narrations(&["Buy AAPL", "Buy AAPL again", "Sell AAPL"]));
    // an invalid regular expression is a clean error of that execution, and the next one works
    let err = compiled
        .execute_with_options(journal(), &Params::new().bind("re", "(").bind("needle", ""), &options())
        .unwrap_err();
    assert_kind(&err, &[QueryErrorKind::Compile, QueryErrorKind::Eval], "invalid regex parameter");
    assert_eq!(run("bread", ""), narrations(&["Bread"]));
}

// =======================================================================================
// track L: icontains, any_icontains, intersects, under

#[test]
fn l_icontains_is_a_case_insensitive_substring_test() {
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE icontains(narration, 'COFFEE')"),
        narrations(&["Morning coffee"])
    );
    // Unicode lower case: 'ÄPFEL' contains 'äpfel'
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE icontains(narration, 'äpfel')"),
        narrations(&["ÄPFEL vom Markt"])
    );
    assert_eq!(
        query(journal(), "SELECT narration FROM #transactions WHERE icontains(payee, 'STRAßE')"),
        narrations(&["ÄPFEL vom Markt"])
    );
    // lower-casing, not case folding: 'straße' does not contain 'strasse'
    assert_eq!(
        query(journal(), "SELECT count(*) FROM #transactions WHERE icontains(payee, 'STRASSE')"),
        rows(&[])
    );
    // the needle is text, not a pattern
    assert_eq!(
        query(
            journal(),
            "SELECT count(*) FROM #transactions WHERE icontains(narration, '.*') OR icontains(narration, '(')"
        ),
        rows(&[])
    );
    // the empty needle is in every string; NULL in, NULL out
    assert_eq!(
        query(
            journal(),
            "SELECT narration, icontains(narration, ''), icontains(payee, 'x'), icontains(narration, NULL) FROM #transactions WHERE payee IS NULL"
        ),
        rows(&[&["Only a narration", "TRUE", "NULL", "NULL"]])
    );
    assert_eq!(
        query(journal(), "SELECT count(*) FROM #transactions WHERE icontains(payee, '')"),
        rows(&[&["14"]])
    );
}

#[test]
fn l_any_icontains_searches_the_elements_of_a_set() {
    assert_eq!(
        query(journal(), "SELECT narration, tags FROM #transactions WHERE any_icontains(tags, 'WEEK')"),
        rows(&[&["Movie night", "fun, weekend"]])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT narration FROM #transactions WHERE any_icontains(tags, 'Cof') OR any_icontains(links, 'GROC')"
        ),
        narrations(&["Groceries", "Morning coffee"])
    );
    // over another set column: the postings whose other accounts include a savings account
    assert_eq!(
        query(
            journal(),
            "SELECT narration, account FROM #postings WHERE any_icontains(other_accounts, 'SAVINGS')"
        ),
        rows(&[&["Move to savings", "Assets:Bank"]])
    );
    // an empty set has no element containing anything, not even the empty needle
    assert_eq!(
        query(
            journal(),
            "SELECT any_icontains(tags, ''), any_icontains(tags, 'x'), any_icontains(tags, NULL), any_icontains(NULL, 'x') FROM #transactions WHERE narration = 'Bread'"
        ),
        rows(&[&["FALSE", "FALSE", "NULL", "NULL"]])
    );
    assert_eq!(
        query(journal(), "SELECT count(*) FROM #transactions WHERE NOT any_icontains(tags, 'x')"),
        rows(&[&["15"]])
    );
}

#[test]
fn l_intersects_tests_two_sets_for_a_common_element() {
    let with = |name: &str, items: &[&str]| Params::new().bind(name, set(items));
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration FROM #transactions WHERE intersects(tags, :wanted)",
            &with("wanted", &["weekend", "travel"])
        ),
        narrations(&["Movie night"])
    );
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration FROM #transactions WHERE intersects(:wanted, tags)",
            &with("wanted", &["coffee", "fun"])
        ),
        narrations(&["Morning coffee", "Movie night"])
    );
    // the transactions touching an account
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration FROM #transactions WHERE intersects(accounts, :accounts)",
            &with("accounts", &["Assets:Old"])
        ),
        narrations(&["Closed account", "Closed and unbalanced"])
    );
    // an empty set intersects nothing; NULL in, NULL out
    assert_eq!(
        query_with(
            journal(),
            "SELECT count(*) FROM #transactions WHERE intersects(accounts, :accounts)",
            &with("accounts", &[])
        ),
        rows(&[])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT intersects(tags, links), intersects(tags, tags), intersects(tags, NULL) FROM #transactions WHERE narration = 'Movie night'"
        ),
        rows(&[&["FALSE", "TRUE", "NULL"]])
    );
    assert_eq!(
        query(journal(), "SELECT intersects(tags, tags) FROM #transactions WHERE narration = 'Bread'"),
        rows(&[&["FALSE"]])
    );
}

#[test]
fn l_under_matches_an_account_and_its_descendants() {
    assert_eq!(
        query(
            journal(),
            "SELECT DISTINCT account FROM #postings WHERE under(account, 'Assets:Bank') ORDER BY account"
        ),
        rows(&[&["Assets:Bank"], &["Assets:Bank:Savings"]])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT DISTINCT account FROM #postings WHERE under(account, 'Assets') ORDER BY account"
        ),
        rows(&[
            &["Assets:Bank"],
            &["Assets:Bank:Savings"],
            &["Assets:BankCard"],
            &["Assets:Broker"],
            &["Assets:Old"]
        ])
    );
    // a prefix that is not a whole component, a trailing colon and another case match nothing
    assert_eq!(
        query(
            journal(),
            "SELECT count(*) FROM #postings WHERE under(account, 'Assets:Ban') OR under(account, 'Assets:') OR under(account, 'assets')"
        ),
        rows(&[])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT under('Assets:Bank', 'Assets:Bank'), under('Assets', 'Assets:Bank'), under(NULL, 'Assets'), under('Assets:Bank', NULL) FROM #transactions LIMIT 1"
        ),
        rows(&[&["TRUE", "FALSE", "NULL", "NULL"]])
    );
    // as a parameter
    assert_eq!(
        query_with(
            journal(),
            "SELECT account, position FROM #postings WHERE under(account, :root) AND date = 2024-01-20",
            &Params::new().bind("root", "Assets:Bank")
        ),
        rows(&[&["Assets:Bank:Savings", "100 CNY"], &["Assets:Bank", "-100 CNY"]])
    );
    assert_eq!(
        query_with(
            journal(),
            "SELECT sum(position) FROM #postings WHERE under(account, :root)",
            &Params::new().bind("root", "Assets:Bank")
        ),
        query(
            journal(),
            "SELECT sum(position) FROM #postings WHERE account = 'Assets:Bank' OR account ~ '^Assets:Bank:'"
        )
    );
}

/// Tracks D1 and L together: a parent account's page lists its sub-accounts' postings with the
/// running balance of each posting's own account, newest first, a page at a time. The 13
/// postings under Assets:Bank, newest first (seq, then posting_index, descending): ÄPFEL vom
/// Markt, Only a narration, Unbalanced, Sell AAPL, Buy AAPL again, then the page asked for:
/// Buy AAPL (Assets:Bank at 205), Move to savings on Assets:Bank (written second, at 705) and
/// on Assets:Bank:Savings (written first, at 100).
#[test]
fn d1_l_parent_account_page_combines_under_and_account_balance() {
    assert_eq!(
        query_with(
            journal(),
            "SELECT narration, account, position, account_balance FROM #postings WHERE under(account, :root) \
             ORDER BY seq DESC, posting_index DESC LIMIT 3 OFFSET :offset",
            &Params::new().bind("root", "Assets:Bank").bind("offset", 5i64)
        ),
        rows(&[
            &["Buy AAPL", "Assets:Bank", "-500 CNY", "205 CNY"],
            &["Move to savings", "Assets:Bank", "-100 CNY", "705 CNY"],
            &["Move to savings", "Assets:Bank:Savings", "100 CNY", "100 CNY"],
        ])
    );
    // under() is scoped like `account IN` (track D1 recognises it): the rows are those of a
    // full scan
    let columns = "date, narration, account, position, balance, account_balance";
    assert_eq!(
        query(journal(), &format!("SELECT {} WHERE under(account, 'Assets:Bank') AND number < 0", columns)),
        query(journal(), &format!("SELECT {} WHERE account ~ '^Assets:Bank(:|$)' AND number < 0", columns))
    );
}

/// Parameters of the wrong type are compile errors of the new functions, not panics.
#[test]
fn l_new_functions_reject_parameters_of_the_wrong_type() {
    for (sql, right, wrong) in [
        ("SELECT icontains(narration, :p) FROM #transactions", DataType::Str, DataType::Int),
        ("SELECT any_icontains(tags, :p) FROM #transactions", DataType::Str, DataType::Set),
        ("SELECT intersects(tags, :p) FROM #transactions", DataType::Set, DataType::Str),
        ("SELECT under(account, :p) FROM #postings", DataType::Str, DataType::Date),
        ("SELECT meta_values(:p) FROM #postings", DataType::Str, DataType::Int),
        ("SELECT entry_meta_values(:p) FROM #postings", DataType::Str, DataType::Bool),
        ("SELECT date_trunc(:p, date) FROM #postings", DataType::Str, DataType::Date),
        ("SELECT date_add(date, :p) FROM #postings", DataType::Int, DataType::Str),
        ("SELECT date_part(:p, date) FROM #postings", DataType::Str, DataType::Int),
        ("SELECT open_date(:p) FROM #postings", DataType::Str, DataType::Int),
    ] {
        // the function exists and takes a parameter of the right type...
        if let Err(err) = Query::compile_with_params(sql, &ParamTypes::new().bind("p", right)) {
            panic!("{} with a {} parameter: {:?} {}", sql, right.name(), err.kind, err.message);
        }
        // ...and a parameter of the wrong type is a compile error
        let err = Query::compile_with_params(sql, &ParamTypes::new().bind("p", wrong))
            .err()
            .unwrap_or_else(|| panic!("{} compiled with a {} parameter", sql, wrong.name()));
        assert_kind(&err, &[QueryErrorKind::Compile], sql);
    }
}

// =======================================================================================
// track L: meta_values, entry_meta_values and the METAS columns

/// Movie night's metadata: the transaction has `project: alpha`, `project: beta`,
/// `trip: Tokyo`; its Expenses:Fun posting `project: alpha`, `project: gamma`, `seat: F12`.
///
/// READING: a key the posting (or transaction) does not have gives an empty set, not NULL, like
/// `tags` of an untagged transaction.
#[test]
fn l_meta_values_keep_every_value_of_a_repeated_key() {
    assert_eq!(
        query(
            journal(),
            "SELECT account, meta_values('project'), entry_meta_values('project') FROM #postings WHERE narration = 'Movie night'"
        ),
        rows(&[
            &["Assets:Bank", "", "alpha, beta"],
            &["Expenses:Fun", "alpha, gamma", "alpha, beta"],
            &["Expenses:Food", "", "alpha, beta"],
        ])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT DISTINCT narration, account FROM #postings WHERE 'gamma' IN meta_values('project')"
        ),
        rows(&[&["Movie night", "Expenses:Fun"]])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT DISTINCT narration FROM #postings WHERE 'beta' IN entry_meta_values('project')"
        ),
        narrations(&["Movie night"])
    );
    assert_eq!(
        query(
            journal(),
            "SELECT account, meta_values('note'), entry_meta_values('receipt') FROM #postings WHERE narration = 'Morning coffee'"
        ),
        rows(&[&["Expenses:Food", "latte", "r-1"], &["Assets:Bank", "", "r-1"]])
    );
    // a missing key is an empty set on every row; NULL in, NULL out
    assert_eq!(
        query(
            journal(),
            "SELECT count(*), sum(length(meta_values('nosuchkey'))), sum(length(entry_meta_values('nosuchkey'))) FROM #postings"
        ),
        rows(&[&["33", "0", "0"]])
    );
    assert_eq!(
        query(journal(), "SELECT meta_values(NULL), entry_meta_values(NULL) FROM #postings LIMIT 1"),
        rows(&[&["NULL", "NULL"]])
    );
    // the sets work with the other new functions
    assert_eq!(
        query_with(
            journal(),
            "SELECT DISTINCT narration FROM #postings WHERE intersects(entry_meta_values('trip'), :trips) AND any_icontains(meta_values('seat'), 'f')",
            &Params::new().bind("trips", set(&["Tokyo", "Paris"]))
        ),
        narrations(&["Movie night"])
    );
}

fn metas_type(result: &QueryResult, column: usize) -> &'static str {
    result.columns[column].ty.name()
}

/// METAS keeps every key and value in written order, repeated keys included; CSV renders it
/// as `key: value` pairs joined by `; `.
#[test]
fn l_metas_columns_keep_repeated_keys_in_written_order() {
    let result = try_query(
        journal(),
        "SELECT account, metas, entry_metas FROM #postings WHERE narration = 'Movie night'",
        &Params::new(),
    )
    .unwrap();
    assert!(metas_type(&result, 1).eq_ignore_ascii_case("metas"), "{}", metas_type(&result, 1));
    assert!(metas_type(&result, 2).eq_ignore_ascii_case("metas"), "{}", metas_type(&result, 2));
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "account,metas,entry_metas\r\n\
         Assets:Bank,,project: alpha; project: beta; trip: Tokyo\r\n\
         Expenses:Fun,project: alpha; project: gamma; seat: F12,project: alpha; project: beta; trip: Tokyo\r\n\
         Expenses:Food,,project: alpha; project: beta; trip: Tokyo\r\n"
    );
    // #transactions and #entries: the transaction's own metadata
    let result = try_query(
        journal(),
        "SELECT narration, metas FROM #transactions WHERE narration IN ('Morning coffee', 'Movie night', 'Bread')",
        &Params::new(),
    )
    .unwrap();
    assert!(metas_type(&result, 1).eq_ignore_ascii_case("metas"));
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "narration,metas\r\n\
         Bread,\r\n\
         Morning coffee,receipt: r-1\r\n\
         Movie night,project: alpha; project: beta; trip: Tokyo\r\n"
    );
    let result = try_query(
        journal(),
        "SELECT type, metas FROM #entries WHERE narration = 'Movie night' OR (type = 'open' AND 'Assets:Broker' IN accounts)",
        &Params::new(),
    )
    .unwrap();
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "type,metas\r\n\
         open,booking_method: FIFO\r\n\
         transaction,project: alpha; project: beta; trip: Tokyo\r\n"
    );
    // a posting's own metadata
    let result = try_query(
        journal(),
        "SELECT account, metas FROM #postings WHERE narration = 'Morning coffee'",
        &Params::new(),
    )
    .unwrap();
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "account,metas\r\nExpenses:Food,note: latte\r\nAssets:Bank,\r\n"
    );
}

// =======================================================================================
// track L: the beanquery 0.2.0 functions, against the oracle

struct OracleCase {
    area: String,
    name: String,
    query: String,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    /// the reason of an accepted deviation, when generate.py marks the case as one
    accepted_deviation: Option<String>,
}

fn oracle_cases() -> Vec<OracleCase> {
    let path = fixture_dir("oracle").join("oracle.json");
    let json: Json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let strings = |value: &Json| {
        value
            .as_array()
            .expect("array")
            .iter()
            .map(|it| it.as_str().expect("string").to_owned())
            .collect::<Vec<_>>()
    };
    json.as_array()
        .expect("cases")
        .iter()
        .map(|case| OracleCase {
            area: case["area"].as_str().unwrap().to_owned(),
            name: case["name"].as_str().unwrap().to_owned(),
            query: case["query"].as_str().unwrap().to_owned(),
            columns: strings(&case["columns"]),
            rows: case["rows"].as_array().unwrap().iter().map(strings).collect(),
            accepted_deviation: case.get("accepted_deviation").map(|it| it.as_str().expect("reason").to_owned()),
        })
        .collect()
}

/// A deliberate difference from beanquery, as the lead ruled it; the mechanism of
/// `ACCEPTED_DEVIATIONS` in `conformance.rs`. The oracle keeps beanquery's rows, and generate.py
/// marks the case with the same reason; zhang must return `rows` instead.
struct Deviation {
    /// the oracle case
    case: &'static str,
    reason: &'static str,
    /// the rows zhang returns, cells as `Value::to_string()` writes them
    rows: &'static [&'static [&'static str]],
}

const DATE_BIN_BOUNDARY: &str = "zhang buckets correctly: a date exactly on a month or year bin boundary starts that bin \
                                 (date_bin('1 month', 2000-02-01, 2000-01-01) is 2000-02-01); beanquery 0.2.0 puts it into \
                                 the previous bin";
const DATE_BIN_FROM_ORIGIN: &str = "zhang starts bin k at origin + k x stride, computed from the origin, so month and year \
                                    bins do not drift (from 2020-01-31 monthly: 01-31, 02-29, 03-31, 04-30), and a date on a bin \
                                    start begins that bin; beanquery 0.2.0 adds the stride to the previous start (01-31, 02-29, \
                                    03-29, ...) and puts a date on a start into the previous bin";
const INTERVAL_WEEKS: &str = "zhang extension: interval('<n> week[s]') is 7 x n days; beanquery 0.2.0 returns NULL for weeks";

const ACCEPTED_DEVIATIONS: &[Deviation] = &[
    // With the origin on the first of a month, a monthly bin is the date's month, a quarterly bin
    // its calendar quarter and a yearly bin its year, also before the origin: the bins of
    // date_trunc('month'), date_trunc('quarter') and date_trunc('year'). beanquery differs on
    // the dates that are themselves a boundary: 2001-01-01, 2020-01-01, 2023-01-01, 2024-01-01
    // in every column, and 2020-03-01 in the monthly ones (beanquery: the previous bin).
    Deviation {
        case: "date_bin_month_boundaries",
        reason: DATE_BIN_BOUNDARY,
        rows: &[
            &["1969-12-31", "1969-12-01", "1969-12-01", "1969-10-01", "1969-01-01"],
            &["1999-12-31", "1999-12-01", "1999-12-01", "1999-10-01", "1999-01-01"],
            &["2000-01-01", "2000-01-01", "2000-01-01", "2000-01-01", "2000-01-01"],
            &["2000-02-29", "2000-02-01", "2000-02-01", "2000-01-01", "2000-01-01"],
            &["2001-01-01", "2001-01-01", "2001-01-01", "2001-01-01", "2001-01-01"],
            &["2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01"],
            &["2020-01-31", "2020-01-01", "2020-01-01", "2020-01-01", "2020-01-01"],
            &["2020-02-29", "2020-02-01", "2020-02-01", "2020-01-01", "2020-01-01"],
            &["2020-03-01", "2020-03-01", "2020-03-01", "2020-01-01", "2020-01-01"],
            &["2020-12-31", "2020-12-01", "2020-12-01", "2020-10-01", "2020-01-01"],
            &["2021-01-03", "2021-01-01", "2021-01-01", "2021-01-01", "2021-01-01"],
            &["2021-02-28", "2021-02-01", "2021-02-01", "2021-01-01", "2021-01-01"],
            &["2021-03-31", "2021-03-01", "2021-03-01", "2021-01-01", "2021-01-01"],
            &["2023-01-01", "2023-01-01", "2023-01-01", "2023-01-01", "2023-01-01"],
            &["2024-01-01", "2024-01-01", "2024-01-01", "2024-01-01", "2024-01-01"],
            &["2024-02-29", "2024-02-01", "2024-02-01", "2024-01-01", "2024-01-01"],
            &["2024-05-31", "2024-05-01", "2024-05-01", "2024-04-01", "2024-01-01"],
            &["2024-12-30", "2024-12-01", "2024-12-01", "2024-10-01", "2024-01-01"],
            &["2024-12-31", "2024-12-01", "2024-12-01", "2024-10-01", "2024-01-01"],
        ],
    },
    // The examples of the ruling, origin 2000-01-01: 2000-02-01 and 2000-03-01 start their
    // monthly bins (beanquery: 2000-01-01, 2000-02-01); the origin is its own bin; 2000-01-31 is
    // the last day of January's bin; with two-month bins 2000-03-01 starts one (beanquery:
    // 2000-01-01) while 2000-02-29 ends the first; 2001-01-01 starts a yearly bin (beanquery:
    // 2000-01-01) while 2000-12-31 ends the first; 1999-12-01, a boundary before the origin,
    // starts its bin (as in beanquery).
    Deviation {
        case: "date_bin_boundary_examples",
        reason: DATE_BIN_BOUNDARY,
        rows: &[&[
            "2000-02-01",
            "2000-03-01",
            "2000-01-01",
            "2000-01-01",
            "2000-03-01",
            "2000-01-01",
            "2001-01-01",
            "2000-01-01",
            "1999-12-01",
        ]],
    },
    // Month-end origins (ruled): bin k starts at origin + k × stride, computed from the origin
    // and clamped to the month's last day, never from the previous start; a date belongs to the
    // last bin starting on or before it.
    // - '1 month' from 2020-01-31: every start is the last day of a month (Jan 31 + k months
    //   clamps to the month end), so a date that is its month's last day starts a bin, and any
    //   other date is in the bin of the previous month's last day: 2000-01-01 -> 1999-12-31,
    //   2020-02-29 -> 2020-02-29, 2020-03-01 -> 2020-02-29, 2021-02-28 -> 2021-02-28,
    //   2024-12-30 -> 2024-11-30. (beanquery drifts to 02-29, 03-29, ..., the 29th, then the
    //   28th.)
    // - '2 months' from 2020-01-31: the starts are the last days of the odd months (Jan, Mar,
    //   May, Jul, Sep, Nov): 1969-12-31 -> 1969-11-30, 2000-02-29 -> 2000-01-31,
    //   2021-03-31 -> 2021-03-31, 2024-05-31 -> 2024-05-31, 2024-12-31 -> 2024-11-30.
    // - '1 year' from 2020-02-29: the starts are Feb 29 in leap years and Feb 28 otherwise:
    //   2000-01-01 -> 1999-02-28, 2000-02-29 -> 2000-02-29 (2000 is a leap year),
    //   2020-01-31 -> 2019-02-28, 2021-02-28 -> 2021-02-28 (the 2021 bin starts on Feb 28),
    //   2024-01-01 -> 2023-02-28, 2024-02-29 -> 2024-02-29. (beanquery: 2021-02-28 ->
    //   2020-02-29, and from 2021 on its starts stay on Feb 28.)
    Deviation {
        case: "date_bin_month_end_origin",
        reason: DATE_BIN_FROM_ORIGIN,
        rows: &[
            &["1969-12-31", "1969-12-31", "1969-11-30", "1969-02-28"],
            &["1999-12-31", "1999-12-31", "1999-11-30", "1999-02-28"],
            &["2000-01-01", "1999-12-31", "1999-11-30", "1999-02-28"],
            &["2000-02-29", "2000-02-29", "2000-01-31", "2000-02-29"],
            &["2001-01-01", "2000-12-31", "2000-11-30", "2000-02-29"],
            &["2020-01-01", "2019-12-31", "2019-11-30", "2019-02-28"],
            &["2020-01-31", "2020-01-31", "2020-01-31", "2019-02-28"],
            &["2020-02-29", "2020-02-29", "2020-01-31", "2020-02-29"],
            &["2020-03-01", "2020-02-29", "2020-01-31", "2020-02-29"],
            &["2020-12-31", "2020-12-31", "2020-11-30", "2020-02-29"],
            &["2021-01-03", "2020-12-31", "2020-11-30", "2020-02-29"],
            &["2021-02-28", "2021-02-28", "2021-01-31", "2021-02-28"],
            &["2021-03-31", "2021-03-31", "2021-03-31", "2021-02-28"],
            &["2023-01-01", "2022-12-31", "2022-11-30", "2022-02-28"],
            &["2024-01-01", "2023-12-31", "2023-11-30", "2023-02-28"],
            &["2024-02-29", "2024-02-29", "2024-01-31", "2024-02-29"],
            &["2024-05-31", "2024-05-31", "2024-05-31", "2024-02-29"],
            &["2024-12-30", "2024-11-30", "2024-11-30", "2024-02-29"],
            &["2024-12-31", "2024-12-31", "2024-11-30", "2024-02-29"],
        ],
    },
    // Single dates around month-end starts, by the same rule:
    // - monthly from 2020-01-31 (starts 01-31, 02-29, 03-31, 04-30, 05-31; before the origin
    //   2019-12-31, 11-30, 10-31, 09-30): 2020-03-30 -> 2020-02-29 (beanquery 2020-03-29),
    //   2020-03-31 -> 2020-03-31 (beanquery 2020-03-29), 2020-04-30 -> 2020-04-30 (beanquery
    //   2020-04-29), 2019-10-30 -> 2019-09-30, the day before the start 2019-10-31 (beanquery
    //   2019-10-30, a drifted start);
    // - two-monthly from 2020-01-31 (starts 01-31, 03-31): 2020-03-31 -> 2020-03-31 (beanquery
    //   2020-01-31), 2020-03-30 -> 2020-01-31;
    // - yearly from 2020-02-29 (starts 2019-02-28, 2020-02-29, 2021-02-28, 2022-02-28,
    //   2023-02-28, 2024-02-29): 2021-02-28 -> 2021-02-28 (beanquery 2020-02-29), 2021-02-27 ->
    //   2020-02-29, 2024-02-28 -> 2023-02-28, 2024-02-29 -> 2024-02-29 (beanquery 2024-02-28),
    //   2019-02-28 -> 2019-02-28;
    // - the interval form: 2020-05-31 -> 2020-05-31 (beanquery 2020-05-29).
    Deviation {
        case: "date_bin_month_end_examples",
        reason: DATE_BIN_FROM_ORIGIN,
        rows: &[&[
            "2020-02-29",
            "2020-03-31",
            "2020-04-30",
            "2019-09-30",
            "2020-03-31",
            "2020-01-31",
            "2021-02-28",
            "2020-02-29",
            "2023-02-28",
            "2024-02-29",
            "2019-02-28",
            "2020-05-31",
        ]],
    },
    // date + 7 days, date - 14 days, date - 7 days, date + 21 days (beanquery: NULL), across
    // month, year and leap-day ends: 1969-12-31 + 7 = 1970-01-07, 2000-02-29 + 7 = 2000-03-07,
    // 2021-02-28 + 7 = 2021-03-07, 2024-12-30 + 7 = 2025-01-06.
    Deviation {
        case: "interval_weeks_are_seven_days",
        reason: INTERVAL_WEEKS,
        rows: &[
            &["1969-12-31", "1970-01-07", "1969-12-17", "1969-12-24", "1970-01-21"],
            &["1999-12-31", "2000-01-07", "1999-12-17", "1999-12-24", "2000-01-21"],
            &["2000-01-01", "2000-01-08", "1999-12-18", "1999-12-25", "2000-01-22"],
            &["2000-02-29", "2000-03-07", "2000-02-15", "2000-02-22", "2000-03-21"],
            &["2001-01-01", "2001-01-08", "2000-12-18", "2000-12-25", "2001-01-22"],
            &["2020-01-01", "2020-01-08", "2019-12-18", "2019-12-25", "2020-01-22"],
            &["2020-01-31", "2020-02-07", "2020-01-17", "2020-01-24", "2020-02-21"],
            &["2020-02-29", "2020-03-07", "2020-02-15", "2020-02-22", "2020-03-21"],
            &["2020-03-01", "2020-03-08", "2020-02-16", "2020-02-23", "2020-03-22"],
            &["2020-12-31", "2021-01-07", "2020-12-17", "2020-12-24", "2021-01-21"],
            &["2021-01-03", "2021-01-10", "2020-12-20", "2020-12-27", "2021-01-24"],
            &["2021-02-28", "2021-03-07", "2021-02-14", "2021-02-21", "2021-03-21"],
            &["2021-03-31", "2021-04-07", "2021-03-17", "2021-03-24", "2021-04-21"],
            &["2023-01-01", "2023-01-08", "2022-12-18", "2022-12-25", "2023-01-22"],
            &["2024-01-01", "2024-01-08", "2023-12-18", "2023-12-25", "2024-01-22"],
            &["2024-02-29", "2024-03-07", "2024-02-15", "2024-02-22", "2024-03-21"],
            &["2024-05-31", "2024-06-07", "2024-05-17", "2024-05-24", "2024-06-21"],
            &["2024-12-30", "2025-01-06", "2024-12-16", "2024-12-23", "2025-01-20"],
            &["2024-12-31", "2025-01-07", "2024-12-17", "2024-12-24", "2025-01-21"],
        ],
    },
    // '1 week', '2 weeks', '-1 week', '+2 weeks', '1 weeks' and '2 week' parse (beanquery: NULL).
    // READING: weeks follow the grammar beanquery has for days, months and years (a signed
    // integer, whitespace, the unit with an optional plural s, case-sensitive), so '1 Week' and
    // '1week' are NULL like '1 Month' and '1day'.
    Deviation {
        case: "interval_week_parsing",
        reason: INTERVAL_WEEKS,
        rows: &[&["FALSE", "FALSE", "FALSE", "FALSE", "FALSE", "FALSE", "TRUE", "TRUE"]],
    },
];

fn deviation(case: &str) -> Option<&'static Deviation> {
    ACCEPTED_DEVIATIONS.iter().find(|it| it.case == case)
}

/// Run every oracle case of `area` with zhang over the same ledger, and compare the column
/// types with beanquery's and the rows (as text, in order) with beanquery's, or with the
/// accepted rows of an [`ACCEPTED_DEVIATIONS`] entry. The accepted rows must still differ from
/// beanquery's: an entry beanquery agrees with is stale.
fn check_oracle(area: &str) {
    let cases = oracle_cases().into_iter().filter(|case| case.area == area).collect::<Vec<_>>();
    assert!(!cases.is_empty(), "no oracle case of {}", area);
    let mut failures = vec![];
    for case in &cases {
        let (expected, source) = match deviation(&case.name) {
            Some(deviation) => {
                let accepted = deviation
                    .rows
                    .iter()
                    .map(|row| row.iter().map(|cell| (*cell).to_owned()).collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                assert_ne!(accepted, case.rows, "{}: beanquery agrees with the accepted deviation; remove it", case.name);
                (accepted, "accepted")
            }
            None => (case.rows.clone(), "beanquery"),
        };
        let outcome = Query::compile(&case.query).and_then(|query| query.execute_with_options(oracle_ledger(), &Params::new(), &options()));
        match outcome {
            Err(err) => failures.push(format!("{}: {:?} {}\n    {}", case.name, err.kind, err.message, case.query)),
            Ok(result) => {
                let types = result.columns.iter().map(|column| column.ty.name().to_owned()).collect::<Vec<_>>();
                if types != case.columns {
                    failures.push(format!("{}: column types {:?}, beanquery {:?}", case.name, types, case.columns));
                }
                let rows = result
                    .rows
                    .iter()
                    .map(|row| row.iter().map(Value::to_string).collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                if rows != expected {
                    let first = rows
                        .iter()
                        .zip(&expected)
                        .position(|(a, b)| a != b)
                        .map(|index| format!("row {}: zhang {:?}, {} {:?}", index, rows[index], source, expected[index]))
                        .unwrap_or_else(|| format!("{} rows, {} {}", rows.len(), source, expected.len()));
                    failures.push(format!("{}: {}\n    {}", case.name, first, case.query));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} {} cases differ from beanquery 0.2.0 or its accepted deviations:\n  {}",
        failures.len(),
        cases.len(),
        area,
        failures.join("\n  ")
    );
}

#[test]
fn l_oracle_date_trunc() {
    check_oracle("date_trunc");
}

#[test]
fn l_oracle_date_part() {
    check_oracle("date_part");
}

#[test]
fn l_oracle_date_add() {
    check_oracle("date_add");
}

#[test]
fn l_oracle_date_diff() {
    check_oracle("date_diff");
}

#[test]
fn l_oracle_date_from_year_month_day() {
    check_oracle("date_ymd");
}

#[test]
fn l_oracle_interval_and_date_arithmetic() {
    check_oracle("interval");
}

/// `interval('<n> week[s]')` is 7 × n days, a zhang extension (accepted deviation: beanquery
/// returns NULL).
#[test]
fn l_oracle_interval_weeks() {
    check_oracle("interval_weeks");
}

#[test]
fn l_oracle_date_bin() {
    check_oracle("date_bin");
}

/// Accepted deviations, as the lead ruled:
/// - bin k starts at origin + k × stride, computed from the origin itself, so month and year
///   bins from a month-end origin do not drift (from 2020-01-31, monthly bins start on 01-31,
///   02-29, 03-31, 04-30, ...; beanquery 0.2.0 adds the stride to the previous start: 01-31,
///   02-29, 03-29, ...);
/// - a date exactly on a bin start begins that bin (`date_bin('1 month', 2000-02-01,
///   2000-01-01)` is 2000-02-01); beanquery puts it into the previous bin, the kind of
///   off-by-one the report graph must not have.
///
/// The oracle keeps beanquery's rows and `ACCEPTED_DEVIATIONS` has zhang's.
#[test]
fn l_oracle_date_bin_month_boundaries() {
    check_oracle("date_bin_boundary");
}

#[test]
fn l_oracle_open_and_commodity_directive_functions() {
    check_oracle("directive_meta");
}

/// The oracle file is the one generate.py writes: every case has rows, and their widths
/// match the column lists. The cases generate.py marks as accepted deviations are exactly those
/// of [`ACCEPTED_DEVIATIONS`], with the same shape of rows.
#[test]
fn l_oracle_fixture_is_well_formed() {
    let cases = oracle_cases();
    assert!(cases.len() >= 20, "{} cases", cases.len());
    let mut names = BTreeSet::new();
    for case in &cases {
        assert!(names.insert(case.name.clone()), "duplicate case {}", case.name);
        assert!(!case.rows.is_empty(), "{} has no rows", case.name);
        for row in &case.rows {
            assert_eq!(row.len(), case.columns.len(), "{}", case.name);
        }
        match (deviation(&case.name), &case.accepted_deviation) {
            (Some(deviation), Some(reason)) => {
                assert!(!reason.is_empty() && !deviation.reason.is_empty(), "{}: no reason", case.name);
                assert_eq!(deviation.rows.len(), case.rows.len(), "{}: accepted rows", case.name);
                for row in deviation.rows {
                    assert_eq!(row.len(), case.columns.len(), "{}: accepted row {:?}", case.name, row);
                }
            }
            (None, None) => {}
            (Some(_), None) => panic!("{} is in ACCEPTED_DEVIATIONS but generate.py does not mark it", case.name),
            (None, Some(_)) => panic!("generate.py marks {} as an accepted deviation, ACCEPTED_DEVIATIONS lacks it", case.name),
        }
    }
    for deviation in ACCEPTED_DEVIATIONS {
        assert!(names.contains(deviation.case), "ACCEPTED_DEVIATIONS refers to unknown case {}", deviation.case);
    }
}

/// A zhang extension of the ruling on weeks: `date_bin(interval('1 week'), date, origin)` puts
/// a date into the 7-day bin anchored at the origin, whatever weekday the origin is, the same
/// bins as `date_bin('7 days', ...)` (checked against beanquery in date_bin_days).
///
/// - Origin 2024-01-01 (a Monday): 2024-01-07 (Sunday) is in the first bin, 2024-01-01;
///   2024-01-08 is exactly one week later and starts the next bin; 2023-12-31 is before the
///   origin, in the bin of 2023-12-25; 2024-02-29 is 59 days later, 8 whole weeks (56 days) and
///   3 days, in the bin of 2024-01-01 + 56 days = 2024-02-26.
/// - Origin 2024-01-03 (a Wednesday): 2024-01-09 is in the bin of 2024-01-03, 2024-01-10 starts
///   the next one, 2024-01-02 is in the bin of 2023-12-27.
/// - Two weeks from 2024-01-01: 2024-01-14 is in the first bin, 2024-01-15 starts the second,
///   2023-12-31 is in the bin of 2023-12-18.
#[test]
fn l_date_bin_buckets_by_interval_weeks() {
    assert_eq!(
        query(
            oracle_ledger(),
            "SELECT date_bin(interval('1 week'), 2024-01-07, 2024-01-01), date_bin(interval('1 week'), 2024-01-08, 2024-01-01), \
             date_bin(interval('1 week'), 2023-12-31, 2024-01-01), date_bin(interval('1 week'), 2024-02-29, 2024-01-01), \
             date_bin(interval('1 week'), 2024-01-09, 2024-01-03), date_bin(interval('1 week'), 2024-01-10, 2024-01-03), \
             date_bin(interval('1 week'), 2024-01-02, 2024-01-03), \
             date_bin(interval('2 weeks'), 2024-01-14, 2024-01-01), date_bin(interval('2 weeks'), 2024-01-15, 2024-01-01), \
             date_bin(interval('2 weeks'), 2023-12-31, 2024-01-01) \
             FROM #postings WHERE account = 'Assets:Wallet' LIMIT 1"
        ),
        rows(&[&[
            "2024-01-01",
            "2024-01-08",
            "2023-12-25",
            "2024-02-26",
            "2024-01-03",
            "2024-01-10",
            "2023-12-27",
            "2024-01-01",
            "2024-01-15",
            "2023-12-18",
        ]])
    );
    // the same bins as 7 and 14 days, on every posting date, before and after the origins; the
    // string form of the stride parses weeks too
    assert_eq!(
        query(
            oracle_ledger(),
            "SELECT DISTINCT date_bin(interval('1 week'), date, 2020-01-06) = date_bin('7 days', date, 2020-01-06), \
             date_bin('1 week', date, 2021-03-03) = date_bin('7 days', date, 2021-03-03), \
             date_bin(interval('2 weeks'), date, 2024-01-01) = date_bin('14 days', date, 2024-01-01)"
        ),
        rows(&[&["TRUE", "TRUE", "TRUE"]])
    );
    // a weekly report: the journal ledger's transactions of 2024-01-15 to 2024-01-21 are in the
    // week of Monday 2024-01-15 (Groceries, Bread, Morning coffee, Movie night, Move to savings,
    // Card payment); December salary (2023-12-31, a Sunday) is in the week of 2023-12-25
    assert_eq!(
        query(
            journal(),
            "SELECT date_bin(interval('1 week'), date, 2024-01-01) AS week, count(*) FROM #transactions \
             WHERE date < 2024-02-01 GROUP BY week ORDER BY week"
        ),
        rows(&[&["2023-12-25", "1"], &["2024-01-15", "6"]])
    );
}

/// beanquery rejects an untyped NULL argument of these functions at compile time ("no function
/// matches"); in zhang a NULL literal fits every type, and the spec's general rule applies:
/// NULL in, NULL out.
#[test]
fn l_date_and_directive_functions_return_null_for_null_arguments() {
    assert_eq!(
        query(
            oracle_ledger(),
            "SELECT date_trunc('month', NULL), date_trunc(NULL, date), date_part('year', NULL), date_part(NULL, date), \
             date_add(NULL, 1), date_add(date, NULL), date_diff(NULL, date), date_diff(date, NULL), \
             date(2024, NULL, 1), date(NULL, 1, 1), interval(NULL) IS NULL, date + interval(NULL), \
             date_bin('7 days', NULL, 2024-01-01), date_bin('7 days', date, NULL), \
             open_date(NULL), close_date(NULL), open_meta(NULL, 'owner'), open_meta('Assets:Bank', NULL), \
             commodity_meta(NULL, 'name'), commodity_meta('CNY', NULL) \
             FROM #postings WHERE account = 'Assets:Wallet' LIMIT 1"
        ),
        vec![vec![
            "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "TRUE", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL",
            "NULL", "NULL",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()]
    );
}

/// beanquery 0.2.0 raises ZeroDivisionError for a zero day stride, so there is no oracle.
/// Ruled: zhang returns NULL, as for a negative stride, or a clean error; never a panic. The
/// same holds for a zero or negative week stride.
#[test]
fn l_date_bin_with_a_zero_stride_does_not_panic() {
    // date_bin exists: 2024-01-10 in weeks from Monday 2024-01-01 is 2024-01-08
    assert_eq!(
        query(oracle_ledger(), "SELECT date_bin('7 days', 2024-01-10, 2024-01-01) FROM #postings LIMIT 1"),
        rows(&[&["2024-01-08"]])
    );
    for sql in [
        "SELECT date_bin('0 days', date, 2024-01-01) FROM #postings LIMIT 1",
        "SELECT date_bin('0 months', date, 2024-01-01) FROM #postings LIMIT 1",
        "SELECT date_bin(interval('0 years'), date, 2024-01-01) FROM #postings LIMIT 1",
        "SELECT date_bin(interval('0 weeks'), date, 2024-01-01) FROM #postings LIMIT 1",
        "SELECT date_bin('-1 week', date, 2024-01-01) FROM #postings LIMIT 1",
    ] {
        match try_query(oracle_ledger(), sql, &Params::new()) {
            Ok(result) => assert_eq!(cells(&result), rows(&[&["NULL"]]), "{}", sql),
            Err(err) => assert_kind(&err, &[QueryErrorKind::Compile, QueryErrorKind::Eval], sql),
        }
    }
}

/// The date functions bucket a report by week, month or year, the use the spec names: monthly
/// spending of the journal ledger by `date_trunc`.
#[test]
fn l_date_functions_bucket_a_report() {
    // Expenses:Food in January: 30 + 50 + 20 + 15 = 115; March: 10 + 5 + 7 + 1 + 2 = 25
    assert_eq!(
        query(
            journal(),
            "SELECT date_trunc('month', date) AS month, sum(position) FROM #postings WHERE account = 'Expenses:Food' GROUP BY month ORDER BY month"
        ),
        rows(&[&["2024-01-01", "115 CNY"], &["2024-03-01", "25 CNY"]])
    );
    // ISO weeks: 2024-01-15 (a Monday) to 2024-01-21 is one week, with Groceries, Bread,
    // Morning coffee, Movie night, Move to savings and Card payment
    assert_eq!(
        query(
            journal(),
            "SELECT date_trunc('week', date) AS week, count(*) FROM #transactions WHERE date >= 2024-01-15 AND date < 2024-02-01 GROUP BY week ORDER BY week"
        ),
        rows(&[&["2024-01-15", "6"]])
    );
    // with parameters for the bucket and a period
    assert_eq!(
        query_with(
            journal(),
            "SELECT date_trunc(:unit, date) AS bucket, count(*) FROM #transactions WHERE date >= :from GROUP BY bucket ORDER BY bucket",
            &Params::new().bind("unit", "year").bind("from", NaiveDate::from_ymd_opt(2023, 1, 1).unwrap())
        ),
        rows(&[&["2023-01-01", "1"], &["2024-01-01", "14"]])
    );
}

// =======================================================================================
// performance smoke tests (ignored by default; run in release mode)

fn median_time(runs: usize, mut run: impl FnMut()) -> Duration {
    for _ in 0..3 {
        run();
    }
    let mut times = (0..runs)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect::<Vec<_>>();
    times.sort();
    times[runs / 2]
}

fn timed(ledger: &Ledger, sql: &str, params: &Params, runs: usize) -> Duration {
    try_timed(ledger, sql, params, runs).unwrap_or_else(|err| panic!("{}: {}", sql, err))
}

/// The median time of `sql`, or its error (a feature that has not landed yet).
fn try_timed(ledger: &Ledger, sql: &str, params: &Params, runs: usize) -> Result<Duration, QueryError> {
    let compiled = Query::compile_with_params(sql, &params.types())?;
    compiled.execute_with_options(ledger, params, &options())?;
    Ok(median_time(runs, || {
        compiled.execute_with_options(ledger, params, &options()).unwrap();
    }))
}

/// Time every query, print the timings and collect the slow and failing ones.
fn time_all(ledger: &Ledger, queries: &[(&str, Params)], runs: usize, limit: Duration) -> Vec<String> {
    let mut problems = vec![];
    for (sql, params) in queries {
        match try_timed(ledger, sql, params, runs) {
            Ok(time) => {
                eprintln!("{:>10.3} ms  {}", time.as_secs_f64() * 1000.0, sql);
                if time >= limit {
                    problems.push(format!("{:?} {}", time, sql));
                }
            }
            Err(err) => {
                eprintln!("    failed  {}: {}", sql, err);
                problems.push(format!("failed: {}: {}", sql, err));
            }
        }
    }
    problems
}

fn release_only() -> bool {
    if cfg!(debug_assertions) {
        eprintln!("skipped: performance smoke tests need --release");
        return false;
    }
    true
}

fn big_ledger() -> Option<Ledger> {
    let dir = std::env::var("ZHANG_QUERY_BIG_LEDGER").unwrap_or_else(|_| "/tmp/qm-b/big".to_owned());
    let dir = PathBuf::from(dir);
    if !dir.join("main.zhang").exists() {
        eprintln!("skipped: no ledger at {} (set ZHANG_QUERY_BIG_LEDGER)", dir.display());
        return None;
    }
    let start = Instant::now();
    let ledger = common::load_ledger(dir, "main.zhang");
    eprintln!("loaded the big ledger in {:?}", start.elapsed());
    Some(ledger)
}

/// Target of the spec: single-account queries on the 80k-posting synthetic ledger take under
/// 2 ms (18-26 ms before the account-scoped scans).
#[test]
#[ignore = "performance smoke test: cargo test --release -p zhang-query --test server_features -- --ignored"]
fn perf_single_account_queries_on_the_big_ledger() {
    if !release_only() {
        return;
    }
    let Some(ledger) = big_ledger() else { return };
    let account = Params::new().bind("account", "Assets:Bank:B007");
    let accounts = Params::new().bind("accounts", set(&["Assets:Bank:B007", "Assets:Bank:B008"]));
    let slow = time_all(
        &ledger,
        &[
            ("SELECT sum(position) WHERE account = 'Assets:Bank:B007'", Params::new()),
            ("SELECT sum(position) WHERE account = :account", account.clone()),
            ("SELECT date, narration, position, balance WHERE account = :account", account.clone()),
            (
                "SELECT date, narration, position, account_balance WHERE account = :account ORDER BY seq DESC LIMIT 50",
                account.clone(),
            ),
            ("SELECT account, sum(position) WHERE account IN :accounts GROUP BY account", accounts.clone()),
            ("SELECT sum(position) WHERE under(account, 'Assets:Bank:B007')", Params::new()),
        ],
        21,
        Duration::from_millis(2),
    );
    assert!(slow.is_empty(), "single-account queries over 2 ms:\n  {}", slow.join("\n  "));
}

/// Target of the spec: under 0.1 ms of fixed cost per query on fava-demo once the dataset of
/// an unchanged ledger is cached (0.4-0.9 ms before). A query that stops at its first row is
/// almost all fixed cost.
#[test]
#[ignore = "performance smoke test: cargo test --release -p zhang-query --test server_features -- --ignored"]
fn perf_fixed_cost_per_query_on_fava_demo() {
    if !release_only() {
        return;
    }
    let ledger = common::fava_demo_ledger();
    let slow = time_all(
        &ledger,
        &[
            ("SELECT date LIMIT 1", Params::new()),
            ("SELECT id FROM #transactions LIMIT 1", Params::new()),
            ("SELECT seq, id FROM #postings LIMIT 1", Params::new()),
        ],
        201,
        Duration::from_micros(100),
    );
    // for the report: the payee list, which hashed an id per row before (no target)
    time_all(&ledger, &[("SELECT DISTINCT payee FROM #transactions", Params::new())], 51, Duration::MAX);
    assert!(slow.is_empty(), "fixed cost over 0.1 ms:\n  {}", slow.join("\n  "));
}

/// The survey measured a keyword regex given as a parameter at 19 ms against 0.95 ms as a
/// literal on fava-demo, and 447 ms against 44 ms on the big ledger. Folded parameters compile
/// the regex once per execution, so both forms cost about the same. The spec asks for the
/// numbers; the bound here (a parameter within 1.5x of the literal, plus 0.2 ms of noise) is
/// this test's own.
#[test]
#[ignore = "performance smoke test: cargo test --release -p zhang-query --test server_features -- --ignored"]
fn perf_parameterized_regex_costs_what_a_literal_costs() {
    if !release_only() {
        return;
    }
    let literal = "SELECT count(*) WHERE payee ~ 'restaurant' OR narration ~ 'restaurant' OR account ~ 'restaurant'";
    let parameterized = literal.replace("'restaurant'", ":keyword");
    let params = Params::new().bind("keyword", "restaurant");
    let mut ledgers = vec![("fava-demo", common::fava_demo_ledger())];
    ledgers.extend(big_ledger().map(|it| ("big", it)));
    let mut slow = vec![];
    for (name, ledger) in &ledgers {
        let literal_time = timed(ledger, literal, &Params::new(), 11);
        let param_time = timed(ledger, &parameterized, &params, 11);
        eprintln!(
            "{}: literal {:.3} ms, parameter {:.3} ms",
            name,
            literal_time.as_secs_f64() * 1000.0,
            param_time.as_secs_f64() * 1000.0
        );
        if param_time > literal_time.mul_f64(1.5) + Duration::from_micros(200) {
            slow.push(format!("{}: {:?} vs {:?}", name, param_time, literal_time));
        }
        // and the same result
        assert_eq!(
            cells(&try_query(ledger, &parameterized, &params).unwrap()),
            cells(&try_query(ledger, literal, &Params::new()).unwrap())
        );
    }
    assert!(slow.is_empty(), "parameterized regex slower than the literal:\n  {}", slow.join("\n  "));
}
