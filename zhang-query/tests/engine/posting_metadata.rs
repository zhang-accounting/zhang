//! `meta()`, `entry_meta()` and `any_meta()` on posting-level metadata, checked against
//! beanquery: `meta(key)` is the posting's value (NULL when the posting has none),
//! `entry_meta(key)` the transaction's, and `any_meta(key)` the posting's, else the
//! transaction's.
//!
//! The ledger is written so that both formats attach every metadata line the same way
//! (transaction metadata before the first posting, posting metadata deeper than its
//! posting): every expected row was produced by beanquery 0.2.0 on beancount 3.2.3 for the
//! same text saved as a `.bean` file.

use zhang_core::ledger::Ledger;
use zhang_query::Value;
use zhang_testkit::ledger::load_text_as;

const LEDGER: &str = r#"1970-01-01 commodity USD
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Drink

2024-01-02 * "Cafe" "lunch"
  category: "meal"
  trip: "rome"
  Assets:Cash -10 USD
    receipt: "r-1"
  Expenses:Food 7 USD
    category: "food"
    note: "pasta \"al dente\""
  Expenses:Drink 3 USD

2024-01-03 * "Shop" "snack"
  Assets:Cash -2 USD
  Expenses:Food 2 USD
    receipt: "r-2"
    trip: "home"
"#;

fn render(value: Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        other => other.to_string(),
    }
}

fn rows(ledger: &Ledger, query: &str) -> Vec<Vec<String>> {
    let result = zhang_query::execute(ledger, query).unwrap_or_else(|err| panic!("{query}: {err:?}"));
    result.rows.into_iter().map(|row| row.into_iter().map(render).collect()).collect()
}

fn expect(ledger: &Ledger, query: &str, expected: &[&[&str]]) {
    let expected: Vec<Vec<String>> = expected.iter().map(|row| row.iter().map(|it| it.to_string()).collect()).collect();
    assert_eq!(rows(ledger, query), expected, "{query}");
}

/// `expected` is one row per line, the cells separated by `|`.
fn expect_table(ledger: &Ledger, query: &str, expected: &str) {
    let expected: Vec<Vec<String>> = expected
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('|').map(|cell| cell.trim().to_owned()).collect())
        .collect();
    assert_eq!(rows(ledger, query), expected, "{query}");
}

fn ledger() -> Ledger {
    let ledger = zhang_testkit::ledger::load_text(LEDGER);
    let errors: Vec<String> = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
    assert!(errors.is_empty(), "{errors:?}");
    ledger
}

#[test]
fn meta_reads_the_posting_and_entry_meta_the_transaction() {
    let ledger = ledger();
    // produced by beanquery 0.2.0
    expect_table(
        &ledger,
        "SELECT date, account, meta('category'), entry_meta('category'), any_meta('category'), \
         meta('receipt'), entry_meta('receipt'), any_meta('receipt'), \
         meta('trip'), entry_meta('trip'), any_meta('trip'), meta('note') ORDER BY date, account",
        r#"
        2024-01-02 | Assets:Cash    | NULL | meal | meal | r-1  | NULL | r-1  | NULL | rome | rome | NULL
        2024-01-02 | Expenses:Drink | NULL | meal | meal | NULL | NULL | NULL | NULL | rome | rome | NULL
        2024-01-02 | Expenses:Food  | food | meal | food | NULL | NULL | NULL | NULL | rome | rome | pasta "al dente"
        2024-01-03 | Assets:Cash    | NULL | NULL | NULL | NULL | NULL | NULL | NULL | NULL | NULL | NULL
        2024-01-03 | Expenses:Food  | NULL | NULL | NULL | r-2  | NULL | r-2  | home | NULL | home | NULL
        "#,
    );
}

#[test]
fn posting_metadata_filters_and_groups() {
    let ledger = ledger();
    // produced by beanquery 0.2.0
    expect(
        &ledger,
        "SELECT date, account, meta('receipt') WHERE meta('receipt') IS NOT NULL ORDER BY date, account",
        &[&["2024-01-02", "Assets:Cash", "r-1"], &["2024-01-03", "Expenses:Food", "r-2"]],
    );
    // produced by beanquery 0.2.0
    expect(
        &ledger,
        "SELECT account, sum(number) WHERE any_meta('trip') = 'rome' GROUP BY account ORDER BY account",
        &[&["Assets:Cash", "-10"], &["Expenses:Drink", "3"], &["Expenses:Food", "7"]],
    );
    // produced by beanquery 0.2.0
    expect(&ledger, "SELECT count(*) WHERE any_meta('trip') IS NOT NULL", &[&["4"]]);
    // produced by beanquery 0.2.0
    expect(&ledger, "SELECT count(*) WHERE meta('trip') IS NULL", &[&["4"]]);
}

/// A beancount ledger with posting metadata at the postings' indentation, deeper, after an
/// indented comment, on a posting without an amount, plus `pushmeta`.
const BEAN_LEDGER: &str = r#"1970-01-01 commodity USD
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Drink

pushmeta source: "import"
2024-01-02 * "Cafe" "lunch"
  category: "meal"
  Assets:Cash -10 USD
  receipt: "r-1"
  ; checked by hand
  Expenses:Food 7 USD
  category: "food"
  Expenses:Drink 3 USD
    trip: "rome"
popmeta source:

2024-01-03 * "Shop" "snack"
    trip: "home"
  Assets:Cash -2 USD
  Expenses:Food
  receipt: "r-2"
"#;

/// [`BEAN_LEDGER`] read by the beancount parser, which loads without errors.
fn beancount_ledger() -> Ledger {
    let ledger = load_text_as("main.bean", BEAN_LEDGER);
    let errors: Vec<String> = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
    assert!(errors.is_empty(), "the beancount ledger has errors: {errors:?}");
    ledger
}

/// The same rows used to be read through `POST /api/query` in zhang-server's tests.
#[test]
fn beanquery_reads_the_posting_metadata_of_a_beancount_ledger() {
    let ledger = beancount_ledger();
    // produced by beanquery 0.2.0 on the same ledger
    expect_table(
        &ledger,
        "SELECT date, account, meta('category'), entry_meta('category'), any_meta('category'), \
         meta('receipt'), entry_meta('receipt'), any_meta('receipt'), meta('trip'), entry_meta('trip'), any_meta('trip'), \
         meta('source'), entry_meta('source'), any_meta('source') ORDER BY date, account",
        r#"
        2024-01-02 | Assets:Cash    | NULL | meal | meal | r-1  | NULL | r-1  | NULL | NULL | NULL | NULL | import | import
        2024-01-02 | Expenses:Drink | NULL | meal | meal | NULL | NULL | NULL | rome | NULL | rome | NULL | import | import
        2024-01-02 | Expenses:Food  | food | meal | food | NULL | NULL | NULL | NULL | NULL | NULL | NULL | import | import
        2024-01-03 | Assets:Cash    | NULL | NULL | NULL | NULL | NULL | NULL | NULL | home | home | NULL | NULL   | NULL
        2024-01-03 | Expenses:Food  | NULL | NULL | NULL | r-2  | NULL | r-2  | NULL | home | home | NULL | NULL   | NULL
        "#,
    );
    // produced by beanquery 0.2.0 on the same ledger
    expect_table(
        &ledger,
        "SELECT account, any_meta('category') WHERE entry_meta('category') = 'meal' ORDER BY account",
        "Assets:Cash | meal\nExpenses:Drink | meal\nExpenses:Food | food",
    );
    // produced by beanquery 0.2.0 on the same ledger
    expect_table(&ledger, "SELECT date, account WHERE meta('category') = 'food'", "2024-01-02 | Expenses:Food");
}
