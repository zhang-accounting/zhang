//! Posting flags (#474) on the `integration-tests` posting flag ledgers: the `posting_flag` column
//! has the flag written before each posting, and is NULL for a posting without one, as in beanquery.
//! An indented line starting with `*` is a posting flagged `*` in both formats. One starting with `#`
//! is a comment in a zhang file, so `posting-flags-star-hash-zhang` books none of those lines, while
//! `posting-flags-star-hash-beancount` books them as postings flagged `#`, as beancount does.
//!
//! The same rows used to be read through `POST /api/query` in zhang-server's tests; the fact is the
//! parsers' and the engine's, and the query route has tests of its own.

use zhang_core::ledger::Ledger;
use zhang_query::Value;
use zhang_testkit::fixtures::fixture_ledger;

const POSTINGS: &str = "SELECT date, flag, posting_flag, account, number";

/// The fixture `name` with its `main` file, in the format of the file, which loads without errors.
fn load(name: &str, main: &str) -> Ledger {
    let ledger = fixture_ledger(name, main)
        .load()
        .unwrap_or_else(|error| panic!("{name}/{main} should load: {error}"));
    assert!(ledger.errors.is_empty(), "{name}/{main} has errors");
    ledger
}

/// The rows `query` returns over the fixture, `NULL` for a null cell.
fn rows(name: &str, main: &str, query: &str) -> Vec<Vec<String>> {
    let ledger = load(name, main);
    let result = zhang_query::execute(&ledger, query).unwrap_or_else(|error| panic!("{name}/{main}: {query}: {error:?}"));
    result
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| match cell {
                    Value::Null => "NULL".to_owned(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect()
}

/// One row per line, the cells separated by `|`.
fn table(rows: &str) -> Vec<Vec<String>> {
    rows.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('|').map(|cell| cell.trim().to_owned()).collect())
        .collect()
}

#[test]
fn both_formats_read_the_flags_they_share() {
    let expected = table(
        r#"
        2020-01-02 | * | NULL | Assets:Cash     | 100
        2020-01-02 | * | NULL | Assets:Bank     | 1000
        2020-01-02 | * | NULL | Equity:Opening  | -1100
        2020-01-10 | * | !    | Assets:Cash     | -10
        2020-01-10 | * | NULL | Expenses:Food   | 10
        2020-01-11 | ! | !    | Assets:Cash     | -20
        2020-01-11 | ! | X    | Expenses:Food   | 15
        2020-01-11 | ! | ?    | Expenses:Drinks | 5
        2020-01-12 | * | &    | Assets:Bank     | -1
        2020-01-12 | * | ?    | Assets:Bank     | -2
        2020-01-12 | * | %    | Assets:Bank     | -3
        2020-01-12 | * | P    | Assets:Bank     | -4
        2020-01-12 | * | C    | Expenses:Drinks | 10
        "#,
    );
    for main in ["main.zhang", "main.bean"] {
        assert_eq!(rows("posting-flags", main, POSTINGS), expected, "{main}");
    }
}

#[test]
fn star_lines_are_postings_in_both_formats_and_hash_lines_only_in_beancount() {
    // in a zhang file the `*` lines are postings flagged `*` and the `#` lines are comments: the
    // elided Lunch posting is inferred without the `#` line, and the Dinner amounts balance without it
    let zhang = rows("posting-flags-star-hash-zhang", "main.zhang", POSTINGS);
    assert_eq!(
        zhang,
        table(
            r#"
            2020-01-02 | * | NULL | Assets:Cash     | 100
            2020-01-02 | * | NULL | Equity:Opening  | -100
            2020-01-10 | * | NULL | Assets:Cash     | -10
            2020-01-10 | * | NULL | Expenses:Food   | 15
            2020-01-10 | * | *    | Assets:Cash     | -5
            2020-01-11 | * | NULL | Assets:Cash     | -27
            2020-01-11 | * | NULL | Expenses:Food   | 20
            2020-01-11 | * | *    | Expenses:Drinks | 7
            "#
        )
    );
    // in a beancount file the `*` and `#` lines are all postings, flagged `*` and `#`, as beancount reads them
    let beancount = rows("posting-flags-star-hash-beancount", "main.bean", POSTINGS);
    assert_eq!(
        beancount,
        table(
            r#"
            2020-01-02 | * | NULL | Assets:Cash     | 100
            2020-01-02 | * | NULL | Equity:Opening  | -100
            2020-01-10 | * | NULL | Assets:Cash     | -10
            2020-01-10 | * | NULL | Expenses:Food   | 10
            2020-01-10 | * | *    | Assets:Cash     | -5
            2020-01-10 | * | #    | Expenses:Food   | 5
            2020-01-11 | * | NULL | Assets:Cash     | -20
            2020-01-11 | * | NULL | Expenses:Food   | 20
            2020-01-11 | * | #    | Assets:Cash     | -7
            2020-01-11 | * | *    | Expenses:Drinks | 7
            "#
        )
    );
}
