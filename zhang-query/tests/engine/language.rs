//! The query-language features of issue #479 (wave 1): `LIMIT` / `OFFSET` with parameters and
//! the total row count, parameters bound as constants, the search functions, `under()`, the
//! account and commodity directive functions, the date functions with intervals, and the
//! structured `metas` columns with `meta_values()` / `entry_meta_values()`.
//!
//! The beanquery functions are also checked against beanquery by the conformance fixtures
//! (`tests/oracle/cases/conformance`, Phase 4); the expectations here are worked out by hand on a small
//! ledger, each explained where it is not obvious.

use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, Interval, ParamTypes, Params, Query, QueryError, QueryErrorKind, QueryResult, Value};

const LEDGER: &str = r#"option "operating_currency" "USD"

1970-01-01 commodity USD
  name: "US Dollar"
  export: "CASH"
1970-01-01 commodity EUR
  name: "Old Euro"
2020-01-01 commodity EUR
  name: "Euro"

1970-01-01 open Assets:Bank USD
  institution: "Bank"
  alias: "b1"
  alias: "b2"
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Assets:Banking
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Food:Coffee
2024-06-30 close Assets:Banking

2024-01-31 * "Café Lumière" "Breakfast" #trip-Paris ^receipt-1
  invoice: "b.pdf"
  invoice: "a.pdf"
  category: "meal"
  Expenses:Food:Coffee 4.50 USD
    note: "croissant"
    note: "CAFÉ au lait"
  Assets:Bank -4.50 USD

2024-02-29 * "Market" "Groceries" #food
  Expenses:Food 20 USD
  Assets:Bank:Savings -20 USD

2024-03-31 * "Bookshop" "Novel"
  Expenses:Food 10 USD
  Assets:Banking -10 USD
"#;

fn ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| zhang_testkit::ledger::load_text(LEDGER))
}

fn fava() -> &'static Ledger {
    static FAVA: OnceLock<Ledger> = OnceLock::new();
    FAVA.get_or_init(zhang_testkit::ledger::fava_demo_ledger)
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
}

fn options(count_total: bool) -> ExecuteOptions {
    ExecuteOptions {
        today: Some(today()),
        timeout: None,
        count_total,
        ..ExecuteOptions::default()
    }
}

fn run_on(ledger: &Ledger, sql: &str, params: &Params, count_total: bool) -> Result<QueryResult, QueryError> {
    Query::compile_with_params(sql, &params.types())?.execute_with_options(ledger, params, &options(count_total))
}

fn render(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        other => other.to_string(),
    }
}

fn table(result: &QueryResult) -> Vec<Vec<String>> {
    result.rows.iter().map(|row| row.iter().map(render).collect()).collect()
}

/// The rows of `sql` on the small ledger, rendered with `str()`.
fn rows(sql: &str) -> Vec<Vec<String>> {
    table(&run_on(ledger(), sql, &Params::new(), false).unwrap_or_else(|err| panic!("{sql}: {err}")))
}

fn rows_with(sql: &str, params: &Params) -> Vec<Vec<String>> {
    table(&run_on(ledger(), sql, params, false).unwrap_or_else(|err| panic!("{sql}: {err}")))
}

fn expected(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|cell| cell.to_string()).collect()).collect()
}

// ---------------------------------------------------------------------------------------
// LIMIT, OFFSET and the total
// ---------------------------------------------------------------------------------------

#[test]
fn offset_skips_rows_of_the_ordered_result() {
    let all = rows("SELECT date, account ORDER BY date, account");
    assert_eq!(all.len(), 6);
    assert_eq!(rows("SELECT date, account ORDER BY date, account LIMIT 2 OFFSET 1"), all[1..3].to_vec());
    assert_eq!(rows("SELECT date, account ORDER BY date, account LIMIT 10 OFFSET 4"), all[4..].to_vec());
    // past the end, and LIMIT 0
    assert!(rows("SELECT date, account ORDER BY date, account LIMIT 3 OFFSET 6").is_empty());
    assert!(rows("SELECT date, account ORDER BY date, account LIMIT 0 OFFSET 1").is_empty());
    // the same windows from parameters
    let params = |limit: i64, offset: i64| Params::new().bind("limit", limit).bind("offset", offset);
    let sql = "SELECT date, account ORDER BY date, account LIMIT :limit OFFSET :offset";
    assert_eq!(rows_with(sql, &params(2, 1)), all[1..3].to_vec());
    assert_eq!(rows_with(sql, &params(i64::MAX, 5)), all[5..].to_vec());
    assert!(rows_with(sql, &params(0, 0)).is_empty());
    assert!(rows_with(sql, &params(1, i64::MAX)).is_empty());
    // positional parameters, and DISTINCT before the window
    let accounts = Params::new().push(2i64).push(1i64);
    assert_eq!(
        rows_with("SELECT DISTINCT account ORDER BY account LIMIT $1 OFFSET $2", &accounts),
        expected(&[&["Assets:Bank:Savings"], &["Assets:Banking"]])
    );
    // without ORDER BY the window follows the ledger order; groups too
    assert_eq!(rows("SELECT account LIMIT 1 OFFSET 2"), expected(&[&["Expenses:Food"]]));
    assert_eq!(
        rows("SELECT root(account, 1) AS root, count(*) GROUP BY root LIMIT 1 OFFSET 1"),
        expected(&[&["Assets", "3"]])
    );
}

#[test]
fn the_total_counts_rows_before_the_window() {
    let total = |sql: &str, params: &Params| run_on(ledger(), sql, params, true).unwrap().total;
    let none = Params::new();
    assert_eq!(total("SELECT date LIMIT 2", &none), Some(6));
    assert_eq!(total("SELECT date ORDER BY date DESC LIMIT 2 OFFSET 3", &none), Some(6));
    assert_eq!(total("SELECT date WHERE account ~ '^Expenses' LIMIT 1", &none), Some(3));
    assert_eq!(total("SELECT DISTINCT payee LIMIT 1", &none), Some(3));
    assert_eq!(total("SELECT DISTINCT root(account, 1) ORDER BY 1 LIMIT 1", &none), Some(2));
    // five accounts have postings
    assert_eq!(total("SELECT account, count(*) GROUP BY account LIMIT 2", &none), Some(5));
    assert_eq!(total("SELECT account, count(*) GROUP BY account ORDER BY account LIMIT 0", &none), Some(5));
    assert_eq!(
        total("SELECT root(account, 1), count(*) GROUP BY 1 HAVING count(*) > 2 LIMIT 1", &none),
        Some(2)
    );
    assert_eq!(total("SELECT date, balance ORDER BY date LIMIT 1 OFFSET 99", &none), Some(6));
    assert_eq!(total("SELECT date WHERE FALSE LIMIT 1", &none), Some(0));
    // without a window the total is the number of rows
    assert_eq!(total("SELECT date", &none), Some(6));
    assert_eq!(total("SELECT name FROM #commodities LIMIT :n", &Params::new().bind("n", 1i64)), Some(3));
    // and it is only counted when asked for
    assert_eq!(run_on(ledger(), "SELECT date LIMIT 1", &none, false).unwrap().total, None);
}

#[test]
fn window_parameters_are_checked_when_bound() {
    let err = |sql: &str, params: Params| run_on(ledger(), sql, &params, false).unwrap_err();
    // a negative value is rejected at the parameter, never wrapped around
    let negative = err("SELECT date LIMIT :n", Params::new().bind("n", -1i64));
    assert_eq!((negative.kind, negative.column), (QueryErrorKind::Compile, Some(19)), "{negative}");
    assert!(negative.message.contains("LIMIT must not be negative, but parameter :n is -1"), "{negative}");
    let negative = err("SELECT date LIMIT 1 OFFSET $1", Params::new().push(i64::MIN));
    assert!(negative.message.contains("OFFSET must not be negative"), "{negative}");
    assert_eq!(negative.column, Some(28), "{negative}");
    // NULL is no count
    let null = Query::compile_with_params("SELECT date LIMIT :n", &ParamTypes::new().bind("n", DataType::Int))
        .unwrap()
        .execute_with_options(ledger(), &Params::new().bind("n", Value::Null), &options(false))
        .unwrap_err();
    assert!(null.message.contains("LIMIT expects an integer, but parameter :n is NULL"), "{null}");
    // a parameter of another type does not compile
    let typed = Query::compile_with_params("SELECT date LIMIT :n", &ParamTypes::new().bind("n", DataType::Decimal))
        .err()
        .unwrap();
    assert_eq!((typed.kind, typed.column), (QueryErrorKind::Compile, Some(19)), "{typed}");
    assert!(typed.message.contains("LIMIT expects an integer, but parameter :n is a decimal"), "{typed}");
    // OFFSET plus LIMIT must fit: at compile time for literals, when bound otherwise
    let literal = Query::compile("SELECT date LIMIT 18446744073709551615 OFFSET 1").err().unwrap();
    assert_eq!((literal.kind, literal.column), (QueryErrorKind::Compile, Some(47)), "{literal}");
    assert!(literal.message.contains("OFFSET plus LIMIT is too large"), "{literal}");
    let bound = err("SELECT date LIMIT 18446744073709551615 OFFSET :o", Params::new().bind("o", 1i64));
    assert!(bound.message.contains("OFFSET plus LIMIT is too large"), "{bound}");
    // the largest window that fits runs
    assert_eq!(rows("SELECT date LIMIT 18446744073709551614 OFFSET 1").len(), 5);
}

// ---------------------------------------------------------------------------------------
// Parameters bound as constants
// ---------------------------------------------------------------------------------------

#[test]
fn bound_parameters_give_the_results_of_literals() {
    let pairs = [
        (
            "SELECT DISTINCT id WHERE payee ~ 'restaurant' OR narration ~ 'restaurant' OR str(tags) ~ 'restaurant' ORDER BY id",
            "SELECT DISTINCT id WHERE payee ~ :kw OR narration ~ :kw OR str(tags) ~ :kw ORDER BY id",
        ),
        (
            "SELECT account, count(*) WHERE account IN ('Assets:US:BofA:Checking', 'Expenses:Food:Coffee') GROUP BY account ORDER BY account",
            "SELECT account, count(*) WHERE account IN :accounts GROUP BY account ORDER BY account",
        ),
        (
            "SELECT count(*) WHERE icontains(payee, 'CAFE') OR icontains(narration, 'cafe')",
            "SELECT count(*) WHERE icontains(payee, :needle) OR icontains(narration, :needle)",
        ),
        (
            "SELECT account, sum(position) WHERE under(account, 'Assets:US') GROUP BY account ORDER BY account",
            "SELECT account, sum(position) WHERE under(account, :root) GROUP BY account ORDER BY account",
        ),
    ];
    let accounts = ["Assets:US:BofA:Checking".to_owned(), "Expenses:Food:Coffee".to_owned()].into_iter().collect();
    let params = Params::new()
        .bind("kw", "restaurant")
        .bind("accounts", Value::Set(accounts))
        .bind("needle", "CAFE")
        .bind("root", "Assets:US");
    for (literal, parameter) in pairs {
        let literal_rows = run_on(fava(), literal, &Params::new(), true).unwrap();
        assert!(!literal_rows.rows.is_empty(), "{literal}");
        let parameter_rows = run_on(fava(), parameter, &params, true).unwrap();
        assert_eq!(format!("{:?}", literal_rows), format!("{:?}", parameter_rows), "{parameter}");
    }
}

#[test]
fn an_invalid_pattern_parameter_fails_where_it_is_matched() {
    let err = run_on(ledger(), "SELECT date WHERE payee ~ :p", &Params::new().bind("p", "("), false).unwrap_err();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Eval, Some(19)), "{err}");
    assert!(err.message.contains("invalid regular expression"), "{err}");
    // a pattern no row is matched against fails nowhere, as before binding
    assert!(run_on(ledger(), "SELECT date WHERE FALSE AND payee ~ :p", &Params::new().bind("p", "("), false).is_ok());
    // a NULL pattern matches nothing
    let null = Query::compile_with_params("SELECT date WHERE payee ~ :p", &ParamTypes::new().bind("p", DataType::Str))
        .unwrap()
        .execute_with_options(ledger(), &Params::new().bind("p", Value::Null), &options(false))
        .unwrap();
    assert!(null.rows.is_empty());
}

#[test]
fn in_a_set_parameter_is_null_aware() {
    let set = |items: &[&str]| Value::Set(items.iter().map(|it| it.to_string()).collect());
    let params = Params::new().bind("accounts", set(&["Assets:Bank", "Assets:Banking"])).bind("none", set(&[]));
    assert_eq!(
        rows_with("SELECT account WHERE account IN :accounts ORDER BY account", &params),
        expected(&[&["Assets:Bank"], &["Assets:Banking"]])
    );
    assert_eq!(rows_with("SELECT count(*) WHERE account NOT IN :accounts", &params), expected(&[&["4"]]));
    // An empty set matches no postings; the ungrouped count is still a single zero.
    assert_eq!(rows_with("SELECT count(*) WHERE account IN :none", &params), expected(&[&["0"]]));
    // NULL in a list makes a miss unknown; a NULL needle is unknown too
    assert_eq!(
        rows_with("SELECT DISTINCT account IN ('Assets:Bank', NULL), payee IN :accounts ORDER BY 1", &params),
        expected(&[&["NULL", "FALSE"], &["TRUE", "FALSE"]])
    );
}

// ---------------------------------------------------------------------------------------
// Search functions and under()
// ---------------------------------------------------------------------------------------

#[test]
fn search_functions_ignore_case() {
    assert_eq!(
        rows("SELECT DISTINCT payee WHERE icontains(payee, 'CAFÉ') OR icontains(narration, 'groc')"),
        expected(&[&["Café Lumière"], &["Market"]])
    );
    assert_eq!(rows("SELECT DISTINCT payee WHERE any_icontains(tags, 'paris')"), expected(&[&["Café Lumière"]]));
    assert_eq!(
        rows_with(
            "SELECT DISTINCT payee WHERE intersects(tags, :tags) OR intersects(links, :tags)",
            &Params::new().bind("tags", Value::Set(["food".to_owned(), "x".to_owned()].into_iter().collect()))
        ),
        expected(&[&["Market"]])
    );
    // NULL in, NULL out
    assert_eq!(
        rows("SELECT DISTINCT icontains(NULL, 'x'), any_icontains(tags, NULL), intersects(tags, NULL) LIMIT 1"),
        expected(&[&["NULL", "NULL", "NULL"]])
    );
}

#[test]
fn under_selects_an_account_and_its_sub_accounts() {
    // Assets:Banking is a sibling whose name starts with Assets:Bank, not a sub-account
    assert_eq!(
        rows("SELECT account, sum(number) WHERE under(account, 'Assets:Bank') GROUP BY account ORDER BY account"),
        expected(&[&["Assets:Bank", "-4.50"], &["Assets:Bank:Savings", "-20"]])
    );
    assert_eq!(
        rows_with("SELECT count(*) WHERE NOT under(account, :root)", &Params::new().bind("root", "Expenses")),
        expected(&[&["3"]])
    );
    assert_eq!(
        rows("SELECT account FROM #accounts WHERE under(account, 'Expenses:Food') ORDER BY account"),
        expected(&[&["Expenses:Food"], &["Expenses:Food:Coffee"]])
    );
}

// ---------------------------------------------------------------------------------------
// Account and commodity directives
// ---------------------------------------------------------------------------------------

#[test]
fn directive_functions_read_open_close_and_commodity() {
    assert_eq!(
        rows(
            "SELECT account, open_date(account), close_date(account), open_meta(account, 'institution'), open_meta(account, 'alias') \
             FROM #accounts ORDER BY account"
        ),
        expected(&[
            // a repeated key reads as its first value, as meta() does
            &["Assets:Bank", "1970-01-01", "NULL", "Bank", "b1"],
            &["Assets:Bank:Savings", "1970-01-01", "NULL", "NULL", "NULL"],
            &["Assets:Banking", "1970-01-01", "2024-06-30", "NULL", "NULL"],
            &["Expenses:Food", "1970-01-01", "NULL", "NULL", "NULL"],
            &["Expenses:Food:Coffee", "1970-01-01", "NULL", "NULL", "NULL"],
        ])
    );
    // the whole metadata, as pairs sorted by key with every value of a repeated key
    let metas = run_on(
        ledger(),
        "SELECT DISTINCT open_meta('Assets:Bank'), open_meta('Assets:Bank:Savings'), open_meta('Nope')",
        &Params::new(),
        false,
    )
    .unwrap();
    assert_eq!(metas.columns.iter().map(|column| column.ty).collect::<Vec<_>>(), [DataType::Metas; 3]);
    assert_eq!(
        metas.rows[0],
        vec![
            Value::Metas(vec![
                ("alias".into(), "b1".into()),
                ("alias".into(), "b2".into()),
                ("institution".into(), "Bank".into())
            ]),
            Value::Metas(vec![]),
            Value::Null,
        ]
    );
    // the last commodity directive of a currency wins, as in beancount
    assert_eq!(
        rows("SELECT name, commodity_meta(name, 'name'), currency_meta(name, 'export'), str(commodity_meta(name)) FROM #commodities ORDER BY name, date"),
        expected(&[
            &["EUR", "Euro", "NULL", "name: Euro"],
            &["EUR", "Euro", "NULL", "name: Euro"],
            &["USD", "US Dollar", "CASH", "export: CASH; name: US Dollar"],
        ])
    );
}

/// An account opened twice reads its earliest `open` directive, as beancount keeps it for
/// beanquery, whichever comes first in the file.
#[test]
fn directive_functions_read_the_earliest_open() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Equity:Opening
2021-01-01 open Assets:Bank
  institution: "second"
2020-01-01 open Assets:Bank
  institution: "first"

2024-01-10 * "Deposit"
  Assets:Bank 30 CNY
  Equity:Opening
"#,
    );
    let result = run_on(
        &ledger,
        "SELECT DISTINCT open_date(account), open_meta(account, 'institution'), str(open_meta('Assets:Bank')) WHERE account = 'Assets:Bank'",
        &Params::new(),
        false,
    )
    .unwrap();
    assert_eq!(table(&result), expected(&[&["2020-01-01", "first", "institution: first"]]));
}

// ---------------------------------------------------------------------------------------
// Dates and intervals
// ---------------------------------------------------------------------------------------

#[test]
fn dates_and_intervals_on_month_ends() {
    assert_eq!(
        rows(
            "SELECT date, date + interval('1 month'), date - interval('1 year'), date_trunc('week', date), \
             date_bin('1 month', date, 2024-01-31), date_bin(interval('2 weeks'), date, 2024-01-01) ORDER BY date LIMIT 3 OFFSET 0"
        )
        .into_iter()
        .step_by(2)
        .collect::<Vec<_>>(),
        expected(&[
            // 2024-01-31: a month later is the last day of February (a leap year); bins from
            // a month end stay on month ends; 2024-01-29 is the second two-week bin
            &["2024-01-31", "2024-02-29", "2023-01-31", "2024-01-29", "2024-01-31", "2024-01-29"],
            // 2024-02-29: a year earlier there is no 29 February; 2024-02-26 is a Monday
            &["2024-02-29", "2024-03-29", "2023-02-28", "2024-02-26", "2024-02-29", "2024-02-26"],
        ])
    );
    let interval = run_on(
        ledger(),
        "SELECT DISTINCT interval('1 week') + interval('13 months') LIMIT 1",
        &Params::new(),
        false,
    )
    .unwrap();
    assert_eq!(interval.columns[0].ty, DataType::Interval);
    assert_eq!(interval.rows[0][0], Value::Interval(Interval::new(13, 7)));
    assert_eq!(render(&interval.rows[0][0]), "1 year 1 month 7 days");
    // NULL literals are NULL, not compile errors
    assert_eq!(
        rows("SELECT DISTINCT date_add(NULL, 1), date_trunc(NULL, date), date_part('year', NULL), date(NULL), date(NULL, 1, 1), date_bin(NULL, date, date), interval(NULL), date + NULL LIMIT 1"),
        expected(&[&["NULL"; 8]])
    );
    // intervals are equal or not (by their months and days), but have no order
    assert_eq!(
        rows(
            "SELECT DISTINCT interval('12 months') = interval('1 year'), interval('1 year') + interval('-1 month') != interval('11 months'), \
             interval('30 days') = interval('1 month'), interval('1 month') IN (interval('1 day'), interval('1 month')) LIMIT 1"
        ),
        expected(&[&["TRUE", "FALSE", "FALSE", "TRUE"]])
    );
    assert_eq!(
        rows("SELECT interval('1 month') AS every, count(*) GROUP BY every"),
        expected(&[&["1 month", "6"]])
    );
    for (sql, message) in [
        ("SELECT interval('1 day') < interval('2 days')", "intervals have no order"),
        ("SELECT date, interval('1 day') AS i ORDER BY i", "cannot order by 'i'"),
        ("SELECT min(interval('1 day'))", "min() is not supported for intervals"),
        ("SELECT max(interval('1 day'))", "max() is not supported for intervals"),
        (
            "SELECT account, interval('1 day') AS i, count(*) AS n GROUP BY 1, 2 PIVOT BY account, i",
            "cannot pivot by 'i'",
        ),
    ] {
        let err = Query::compile(sql).err().unwrap_or_else(|| panic!("{sql}"));
        assert_eq!(err.kind, QueryErrorKind::Compile, "{sql}");
        assert!(err.message.contains(message), "{sql}: {err}");
    }
    // dates stay in the years 1 to 9999: results outside are NULL
    assert_eq!(
        rows(
            "SELECT DISTINCT 9999-12-31 + 1, 0001-01-01 - 1, 1 + 9999-12-31, 9999-12-31 + interval('1 day'), \
             0001-01-31 - interval('1 month'), date_add(9999-12-31, 1), date_trunc('decade', 0002-12-15), \
             date_bin('1 year', 9999-06-01, 0001-07-01), 9999-12-30 + 1 LIMIT 1"
        ),
        expected(&[&["NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "NULL", "9998-07-01", "9999-12-31"]])
    );
    // a stride whose months and days have opposite signs is NULL
    assert_eq!(
        rows("SELECT DISTINCT date_bin(interval('2 months') - interval('61 days'), date, 2000-01-01) LIMIT 1"),
        expected(&[&["NULL"]])
    );
    // a zero stride is NULL
    assert_eq!(
        rows("SELECT DISTINCT date_bin('0 days', date, 2024-01-01), date_bin(interval('0 months'), date, 2024-01-01) LIMIT 1"),
        expected(&[&["NULL", "NULL"]])
    );
}

// ---------------------------------------------------------------------------------------
// Structured metadata
// ---------------------------------------------------------------------------------------

#[test]
fn metas_columns_keep_repeated_keys() {
    let result = run_on(
        ledger(),
        "SELECT account, metas, entry_metas WHERE date = 2024-01-31 ORDER BY account",
        &Params::new(),
        false,
    )
    .unwrap();
    assert_eq!(result.columns[1].ty, DataType::Metas);
    let pairs = |items: &[(&str, &str)]| Value::Metas(items.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect());
    let entry = pairs(&[("category", "meal"), ("invoice", "b.pdf"), ("invoice", "a.pdf")]);
    assert_eq!(
        result.rows,
        vec![
            vec![Value::from("Assets:Bank"), pairs(&[]), entry.clone()],
            // sorted by key; the values of a repeated key in written order
            vec![
                Value::from("Expenses:Food:Coffee"),
                pairs(&[("note", "croissant"), ("note", "CAFÉ au lait")]),
                entry
            ],
        ]
    );
    assert_eq!(
        rows("SELECT payee, metas FROM #transactions ORDER BY date"),
        expected(&[
            &["Café Lumière", "category: meal; invoice: b.pdf; invoice: a.pdf"],
            &["Market", ""],
            &["Bookshop", ""]
        ])
    );
    assert_eq!(
        rows("SELECT metas FROM #entries WHERE type = 'open' AND 'Assets:Bank' IN accounts"),
        expected(&[&["alias: b1; alias: b2; institution: Bank"]])
    );
    assert_eq!(
        rows("SELECT str(metas) FROM #entries WHERE type = 'commodity' ORDER BY date LIMIT 1"),
        expected(&[&["export: CASH; name: US Dollar"]])
    );
    // the CSV export writes the pairs, the JSON-like Debug keeps them apart
    let csv = zhang_query::export::to_csv(&run_on(ledger(), "SELECT entry_metas WHERE account = 'Assets:Bank'", &Params::new(), false).unwrap());
    assert_eq!(csv, "entry_metas\r\ncategory: meal; invoice: b.pdf; invoice: a.pdf\r\n");
}

#[test]
fn meta_values_return_every_value() {
    assert_eq!(
        rows("SELECT account, meta_values('note'), entry_meta_values('invoice'), meta('note'), entry_meta('invoice') WHERE date = 2024-01-31 ORDER BY account"),
        expected(&[
            &["Assets:Bank", "", "a.pdf, b.pdf", "NULL", "b.pdf"],
            &["Expenses:Food:Coffee", "CAFÉ au lait, croissant", "a.pdf, b.pdf", "croissant", "b.pdf"],
        ])
    );
    assert_eq!(
        rows("SELECT count(*) WHERE 'a.pdf' IN entry_meta_values('invoice') AND any_icontains(meta_values('note'), 'café')"),
        expected(&[&["1"]])
    );
    // on a record table they read the row's own metadata
    assert_eq!(
        rows("SELECT meta_values('alias') FROM #accounts WHERE account = 'Assets:Bank'"),
        expected(&[&["b1, b2"]])
    );
    // they read the row, so a grouped query must group or aggregate them
    let err = Query::compile("SELECT account, meta_values('x'), count(*) GROUP BY account").err().unwrap();
    assert!(err.message.contains("'meta_values('x')' is missing"), "{err}");
    let err = Query::compile("SELECT metas, count(*) GROUP BY metas").err().unwrap();
    assert!(err.message.contains("values of type metas cannot be grouped"), "{err}");
}

// ---------------------------------------------------------------------------------------
// Schema and syntax
// ---------------------------------------------------------------------------------------

#[test]
fn new_functions_and_columns_are_in_the_schema() {
    let schema = zhang_query::schema();
    for name in [
        "icontains",
        "any_icontains",
        "intersects",
        "under",
        "date_trunc",
        "date_add",
        "date_diff",
        "date_part",
        "date",
        "date_bin",
        "interval",
        "open_meta",
        "commodity_meta",
        "currency_meta",
        "open_date",
        "close_date",
        "meta_values",
        "entry_meta_values",
    ] {
        assert!(schema.functions.iter().any(|function| function.name == name), "{name}");
    }
    let columns = |table: &str| {
        schema
            .tables
            .iter()
            .find(|it| it.name == table)
            .unwrap()
            .columns
            .iter()
            .map(|column| (column.name, column.ty))
            .collect::<Vec<_>>()
    };
    assert!(columns("postings").contains(&("metas", DataType::Metas)));
    assert!(columns("postings").contains(&("entry_metas", DataType::Metas)));
    assert!(columns("transactions").contains(&("metas", DataType::Metas)));
    assert!(columns("entries").contains(&("metas", DataType::Metas)));
    // the zhang extensions say so, in one wording
    for function in schema
        .functions
        .iter()
        .filter(|it| ["icontains", "any_icontains", "intersects", "under", "meta_values", "entry_meta_values"].contains(&it.name))
    {
        assert!(function.description.ends_with(". A zhang extension."), "{}", function.description);
    }
    for (table, column) in [
        ("postings", "metas"),
        ("postings", "entry_metas"),
        ("transactions", "metas"),
        ("entries", "metas"),
    ] {
        let doc = schema
            .tables
            .iter()
            .find(|it| it.name == table)
            .unwrap()
            .columns
            .iter()
            .find(|it| it.name == column)
            .unwrap();
        assert!(doc.description.ends_with(". A zhang extension."), "{table}.{column}: {}", doc.description);
    }
    let signatures = schema.functions.iter().map(|function| function.signature.as_str()).collect::<Vec<_>>();
    assert!(signatures.contains(&"date_bin(interval, date, date) -> date"), "{signatures:?}");
    assert!(signatures.contains(&"open_meta(str) -> metas"), "{signatures:?}");
}

/// `offset` is a keyword only right after a LIMIT count, so it stays a name elsewhere, as in
/// beanquery, which has no OFFSET.
#[test]
fn offset_is_a_name_outside_the_limit_clause() {
    assert_eq!(rows("SELECT date AS offset ORDER BY offset DESC LIMIT 1"), expected(&[&["2024-03-31"]]));
    assert_eq!(
        rows("SELECT account AS offset, count(*) GROUP BY offset ORDER BY offset LIMIT 1 OFFSET 1"),
        expected(&[&["Assets:Bank:Savings", "1"]])
    );
    let err = Query::compile("SELECT offset").err().unwrap();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(8)), "{err}");
}

#[test]
fn offset_syntax_errors_point_at_the_clause() {
    for (sql, column, message) in [
        ("SELECT date OFFSET 2", 13, "OFFSET must follow LIMIT"),
        (
            "SELECT date LIMIT 2 OFFSET",
            27,
            "expected a non-negative integer or a parameter after OFFSET, found end of query",
        ),
        ("SELECT date LIMIT 2 OFFSET -1", 28, "after OFFSET, found '-'"),
        ("SELECT date LIMIT 2.5", 19, "after LIMIT, found '2.5'"),
        ("SELECT date LIMIT :", 19, "expected a parameter name after ':'"),
        ("SELECT date LIMIT 1 OFFSET 99999999999999999999", 28, "OFFSET is too large"),
        ("SELECT date LIMIT 1 OFFSET 1 OFFSET 2", 30, "unexpected 'OFFSET'"),
    ] {
        let err = Query::compile(sql).err().unwrap_or_else(|| panic!("{sql}"));
        assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(column)), "{sql}: {err}");
        assert!(err.message.contains(message), "{sql}: {err}");
    }
}

/// The new functions nest up to the parser's limit on the 2 MiB stack of tokio's blocking
/// pool, and deeper nesting is a parse error, not a crash.
#[test]
fn new_functions_nest_to_the_limit_on_a_small_stack() {
    let on_small_stack = |sql: String, params: Params| {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let query = Query::compile_with_params(&sql, &params.types())?;
                query.execute_with_options(ledger(), &params, &options(true)).map(|result| table(&result))
            })
            .unwrap()
            .join()
            .expect("the query thread crashed")
    };
    let depth = zhang_query::MAX_DEPTH - 8;
    let nested = format!("SELECT {}date{} LIMIT :n OFFSET :o", "date_add(".repeat(depth), ", 1)".repeat(depth));
    let rows = on_small_stack(nested, Params::new().bind("n", 1i64).bind("o", 0i64)).unwrap();
    // the first posting, 2024-01-31, plus MAX_DEPTH - 8 = 56 days
    assert_eq!(rows, expected(&[&["2024-03-27"]]));
    let nested = format!("SELECT count(*) WHERE {}under(account, :root){}", "NOT NOT ".repeat(depth / 2 - 1), "");
    assert_eq!(on_small_stack(nested, Params::new().bind("root", "Assets")).unwrap(), expected(&[&["3"]]));
    let too_deep = format!("SELECT {}date{}", "date_trunc('month', ".repeat(2_000), ")".repeat(2_000));
    let err = on_small_stack(too_deep, Params::new()).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Parse, "{err}");
    assert!(err.message.contains("nested too deeply"), "{err}");
    // a long IN list and a large set parameter are one hash lookup each
    let accounts = (0..3_000).map(|idx| format!("'Expenses:X{}'", idx)).collect::<Vec<_>>().join(", ");
    let sql = format!("SELECT count(*) WHERE account IN ({}, 'Expenses:Food') OR account IN :set", accounts);
    assert!(sql.len() < zhang_query::MAX_QUERY_LENGTH);
    let set = Value::Set((0..100_000).map(|idx| format!("Assets:Y{}", idx)).chain(["Assets:Bank".to_owned()]).collect());
    assert_eq!(on_small_stack(sql, Params::new().bind("set", set)).unwrap(), expected(&[&["3"]]));
}

/// METAS cells, and sets of long texts, count their text against the result budget like
/// strings do, so a few rows of large metadata cannot slip past the limit.
#[test]
fn metas_and_sets_count_their_text_against_the_result_budget() {
    let mut text = String::from("1970-01-01 commodity USD\n1970-01-01 open Assets:A\n1970-01-01 open Expenses:B\n");
    let value = "x".repeat(200);
    for txn in 0..50 {
        text.push_str(&format!("\n2024-01-01 * \"big meta {txn}\"\n"));
        for key in 0..20 {
            text.push_str(&format!("  k{key:03}: \"{value}\"\n  r: \"{key}{value}\"\n"));
        }
        text.push_str("  Expenses:B 1 USD\n  Assets:A -1 USD\n");
    }
    let ledger = zhang_testkit::ledger::load_text(&text);
    let run = |sql: &str, limit: u64| {
        let options = ExecuteOptions {
            max_result_values: Some(limit),
            ..options(false)
        };
        Query::compile(sql).unwrap().execute_with_options(&ledger, &Params::new(), &options)
    };
    // 50 rows of 40 pairs of about 205 bytes: about 8,000 values, not 50
    for sql in [
        "SELECT metas FROM #transactions",
        "SELECT entry_metas WHERE account = 'Assets:A'",
        "SELECT entry_meta_values('r') FROM #transactions",
    ] {
        let err = run(sql, 1_000).unwrap_err();
        assert_eq!(err.kind, QueryErrorKind::TooLarge, "{sql}: {err}");
        assert_eq!(run(sql, 100_000).unwrap().rows.len(), 50, "{sql}");
    }
}

// ---------------------------------------------------------------------------------------
// CASE
// ---------------------------------------------------------------------------------------

/// `CASE WHEN ... THEN ... [ELSE ...] END` picks the value of the first condition that is
/// TRUE, else the ELSE value, else NULL. The postings are 4.50 and -4.50 USD, 20 and -20 USD,
/// 10 and -10 USD.
#[test]
fn case_picks_the_first_true_condition() {
    assert_eq!(
        rows("SELECT number, CASE WHEN number >= 10 THEN 'big' WHEN number > 0 THEN 'small' ELSE 'refund' END WHERE number > -5"),
        expected(&[&["4.50", "small"], &["-4.50", "refund"], &["20", "big"], &["10", "big"]])
    );
    // without ELSE, no TRUE condition gives NULL
    assert_eq!(
        rows("SELECT CASE WHEN number > 0 THEN account END WHERE year = 2024 AND month = 1"),
        expected(&[&["Expenses:Food:Coffee"], &["NULL"]])
    );
    // the keywords are case-insensitive and the expression nests
    assert_eq!(
        rows("select case when month = 1 then case when number > 0 then 'in' else 'out' end else 'later' end as kind WHERE month <= 2"),
        expected(&[&["in"], &["out"], &["later"], &["later"]])
    );
}

/// A NULL condition is not TRUE, as in WHERE: the CASE goes on to the next branch.
#[test]
fn case_treats_a_null_condition_as_not_true() {
    // payee = NULL is NULL for every row
    assert_eq!(
        rows("SELECT CASE WHEN payee = NULL THEN 'null' WHEN NULL THEN 'null again' ELSE 'else' END WHERE month = 2"),
        expected(&[&["else"], &["else"]])
    );
    // a NULL value is returned as it is
    assert_eq!(
        rows("SELECT CASE WHEN TRUE THEN NULL ELSE 'x' END, CASE WHEN FALSE THEN 'x' END LIMIT 1"),
        expected(&[&["NULL", "NULL"]])
    );
}

/// The values of a CASE have one type: NULL fits any, and an integer is widened to a decimal
/// when another value is a decimal. The conditions must be booleans.
#[test]
fn case_values_have_one_type() {
    assert_eq!(
        rows("SELECT CASE WHEN number > 0 THEN 1 ELSE 0.5 END WHERE month = 3"),
        expected(&[&["1"], &["0.5"]])
    );
    let columns = Query::compile("SELECT CASE WHEN TRUE THEN 1 ELSE 0.5 END, CASE WHEN TRUE THEN NULL ELSE date END")
        .unwrap()
        .columns();
    assert_eq!(columns.iter().map(|it| it.ty).collect::<Vec<_>>(), vec![DataType::Decimal, DataType::Date]);

    let err = Query::compile("SELECT CASE WHEN TRUE THEN 'x' ELSE 1 END").err().unwrap();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(37)), "{err}");
    assert_eq!(
        err.message,
        "the values of a CASE must have one type, but this one is int and an earlier one str"
    );
    let err = Query::compile("SELECT CASE WHEN number THEN 1 END").err().unwrap();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Compile, Some(18)), "{err}");
    assert_eq!(err.message, "a WHEN condition must be a boolean, got decimal");
    // END is required
    let err = Query::compile("SELECT CASE WHEN TRUE THEN 1").err().unwrap();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(29)), "{err}");
    assert_eq!(err.message, "expected WHEN, ELSE or END in CASE, found end of query");
    let err = Query::compile("SELECT CASE WHEN TRUE THEN 1 ELSE 2 FROM #prices").err().unwrap();
    assert_eq!(err.message, "expected END in CASE, found 'FROM'");
    let err = Query::compile("SELECT CASE WHEN TRUE 1 END").err().unwrap();
    assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(23)), "{err}");
    assert_eq!(err.message, "expected THEN after the WHEN condition, found '1'");
}

/// Only the chosen value is evaluated: a value that would fail (an integer overflow) is never
/// computed for the rows that do not choose it.
#[test]
fn case_evaluates_only_the_chosen_value() {
    assert_eq!(
        rows("SELECT CASE WHEN year < 1900 THEN 9223372036854775807 + year ELSE 1 END WHERE month = 1"),
        expected(&[&["1"], &["1"]])
    );
    let err = run_on(
        ledger(),
        "SELECT CASE WHEN year > 1900 THEN 9223372036854775807 + year ELSE 1 END",
        &Params::new(),
        false,
    )
    .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Eval, "{err}");
    // an aggregate adds up every row of its group, whichever branch the group chooses, as in
    // SQL: its argument is computed, and fails, on every row
    let sql = "SELECT account, CASE WHEN count(*) < 0 THEN sum(9223372036854775807 + year) ELSE count(*) END GROUP BY account";
    let err = run_on(ledger(), sql, &Params::new(), false).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Eval, "{err}");
    assert!(err.message.contains("overflow"), "{err}");
}

/// A CASE can choose between aggregates, the way a built-in query carries a budget over, and
/// between parameters; constant conditions are decided when the query is compiled.
#[test]
fn case_works_with_aggregates_and_parameters() {
    assert_eq!(
        rows("SELECT account, CASE WHEN count(*) > 1 THEN sum(number) ELSE max(number) * 100 END GROUP BY account ORDER BY account"),
        expected(&[
            &["Assets:Bank", "-450.00"],
            &["Assets:Bank:Savings", "-2000"],
            &["Assets:Banking", "-1000"],
            &["Expenses:Food", "30"],
            &["Expenses:Food:Coffee", "450.00"],
        ])
    );
    let sql = "SELECT CASE WHEN last(date) < :month THEN 'carried' ELSE 'own' END GROUP BY account ORDER BY account LIMIT 1";
    let at = |month: NaiveDate| rows_with(sql, &Params::new().bind("month", month));
    assert_eq!(at(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap()), expected(&[&["own"]]));
    assert_eq!(at(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap()), expected(&[&["carried"]]));
    // a constant TRUE after a condition of the row ends the CASE there, as its ELSE: the
    // condition of the row is still tested on every row
    assert_eq!(
        rows("SELECT account, CASE WHEN account ~ 'Bank' THEN 'bank' WHEN TRUE THEN 'other' WHEN account ~ 'Food' THEN 'never' END WHERE month = 2"),
        expected(&[&["Expenses:Food", "other"], &["Assets:Bank:Savings", "bank"]])
    );
    let explain = Query::compile("SELECT CASE WHEN account ~ 'Bank' THEN 'bank' WHEN TRUE THEN 'other' END")
        .unwrap()
        .explain();
    assert!(explain.contains("CASE WHEN (account ~ /Bank/i) THEN 'bank' ELSE 'other' END"), "{explain}");
    // a constant condition leaves the chosen value alone in the plan
    let explain = Query::compile("SELECT CASE WHEN 1 > 2 THEN 'a' WHEN year > 2000 THEN 'b' ELSE 'c' END")
        .unwrap()
        .explain();
    assert!(explain.contains("CASE WHEN (year > 2000) THEN 'b' ELSE 'c' END"), "{explain}");
    let explain = Query::compile("SELECT CASE WHEN 1 < 2 THEN 'a' ELSE account END").unwrap().explain();
    assert!(explain.contains("target 0: CASE WHEN 1 < 2 THEN 'a' ELSE account END = 'a'"), "{explain}");
}

/// `case` is a keyword only before WHEN, so it still names a target; CASE expressions nest to
/// the depth limit on a small stack, and deeper ones are a parse error.
#[test]
fn case_is_a_keyword_only_before_when_and_nests_to_the_limit() {
    assert_eq!(rows("SELECT month AS case ORDER BY case DESC LIMIT 1"), expected(&[&["3"]]));
    let on_small_stack = |sql: String| {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                Query::compile(&sql)
                    .and_then(|query| query.execute_with_options(ledger(), &Params::new(), &options(false)))
                    .map(|result| table(&result))
            })
            .unwrap()
            .join()
            .expect("the query thread crashed")
    };
    let nested = |depth: usize| format!("SELECT {}month{} LIMIT 1", "CASE WHEN month > 0 THEN ".repeat(depth), " END".repeat(depth));
    assert_eq!(on_small_stack(nested(zhang_query::MAX_DEPTH / 2 - 4)).unwrap(), expected(&[&["1"]]));
    let err = on_small_stack(nested(2_000)).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Parse, "{err}");
    assert!(err.message.contains("nested too deeply"), "{err}");
}
