//! `Query::inline_params`: a query with its parameters written in as BQL (what "Open in
//! Explore" shows for a built-in query) runs to the same result as the query with the
//! parameters bound, for every type a parameter can have, including strings with quotes and
//! backslashes, dates, sets and NULL.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::OnceLock;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, Interval, ParamRef, ParamTypes, Params, Query, QueryResult, Value};

const LEDGER: &str = r#"option "operating_currency" "USD"

1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel

2024-01-31 * "O'Brien" "Lunch" #food
  Expenses:Food 12.50 USD
  Assets:Bank -12.50 USD

2024-02-01 * "Café \"Lumière\"" "Coffee" #food #trip
  Expenses:Food 4.50 USD
  Assets:Bank -4.50 USD

2024-02-29 * "it's \"quoted\"" "both kinds" #trip
  Expenses:Travel 100 USD
  Assets:Bank:Savings -100 USD

2024-03-01 * "C:\\temp\\new" "a backslash"
  Expenses:Travel 7 USD
  Assets:Bank -7 USD

2024-03-02 * "'\"'\"" "only quotes"
  Expenses:Food 1 USD
  Assets:Bank -1 USD
"#;

fn ledger() -> &'static Ledger {
    static LEDGER_CELL: OnceLock<Ledger> = OnceLock::new();
    LEDGER_CELL.get_or_init(|| zhang_testkit::ledger::load_text(LEDGER))
}

fn options() -> ExecuteOptions {
    ExecuteOptions {
        today: NaiveDate::from_ymd_opt(2025, 1, 1),
        timeout: None,
        count_total: true,
        ..ExecuteOptions::default()
    }
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::from_str(text).unwrap()
}

fn decimal(text: &str) -> BigDecimal {
    BigDecimal::from_str(text).unwrap()
}

fn set(items: &[&str]) -> Value {
    Value::Set(items.iter().map(|item| item.to_string()).collect::<BTreeSet<_>>())
}

/// Run `sql` with `params` bound and with them written in, check both give the same result
/// (columns, rows and total), and return the text and the result.
fn round_trip(sql: &str, params: &Params) -> (String, QueryResult) {
    let query = Query::compile_with_params(sql, &params.types()).unwrap_or_else(|err| panic!("{}: {}", sql, err));
    let bound = query
        .execute_with_options(ledger(), params, &options())
        .unwrap_or_else(|err| panic!("{}: {}", sql, err));
    let text = query.inline_params(params).unwrap_or_else(|err| panic!("{}: {}", sql, err));
    let inlined = Query::compile(&text)
        .and_then(|query| query.execute_with_options(ledger(), &Params::new(), &options()))
        .unwrap_or_else(|err| panic!("{} -> {}: {}", sql, text, err));
    assert_eq!(bound, inlined, "{} -> {}", sql, text);
    (text, bound)
}

fn payees(result: &QueryResult) -> Vec<String> {
    result.rows.iter().map(|row| row[0].as_str().unwrap_or("NULL").to_owned()).collect()
}

#[test]
fn strings_with_quotes_and_backslashes_read_back_unchanged() {
    let sql = "SELECT DISTINCT payee WHERE payee = :payee";
    for (payee, written) in [
        ("O'Brien", r#""O'Brien""#),
        (r#"Café "Lumière""#, r#"'Café "Lumière"'"#),
        (r#"it's "quoted""#, r#"("it's " + '"quoted"')"#),
        (r"C:\temp\new", r"'C:\temp\new'"),
        (r#"'"'""#, r#"("'" + '"' + "'" + '"')"#),
    ] {
        let (text, result) = round_trip(sql, &Params::new().bind("payee", payee));
        assert_eq!(text, format!("SELECT DISTINCT payee WHERE payee = {}", written));
        assert_eq!(payees(&result), vec![payee.to_owned()], "{}", text);
    }
    // nothing matches the empty string, and the text says so too
    let (text, result) = round_trip(sql, &Params::new().bind("payee", ""));
    assert_eq!(text, "SELECT DISTINCT payee WHERE payee = ''");
    assert!(result.rows.is_empty());
}

#[test]
fn backslashes_of_a_pattern_stay_regular_expression_escapes() {
    let (text, result) = round_trip(
        "SELECT DISTINCT payee WHERE payee ~ :pattern ORDER BY payee",
        &Params::new().bind("pattern", r"^C:\\temp\\"),
    );
    assert_eq!(text, r"SELECT DISTINCT payee WHERE payee ~ '^C:\\temp\\' ORDER BY payee");
    assert_eq!(payees(&result), vec![r"C:\temp\new".to_owned()]);

    let (_, result) = round_trip("SELECT DISTINCT payee WHERE icontains(payee, :needle)", &Params::new().bind("needle", "\"LUMI"));
    assert_eq!(payees(&result), vec![r#"Café "Lumière""#.to_owned()]);
}

#[test]
fn dates_bound_and_written_select_the_same_days() {
    let params = Params::new().bind("from", date("2024-02-01")).bind("to", date("2024-02-29"));
    let (text, result) = round_trip(
        "SELECT date, payee, position WHERE date >= :from AND date <= :to AND account ~ '^Expenses' ORDER BY date",
        &params,
    );
    assert_eq!(
        text,
        "SELECT date, payee, position WHERE date >= 2024-02-01 AND date <= 2024-02-29 AND account ~ '^Expenses' ORDER BY date"
    );
    assert_eq!(result.rows.len(), 2);

    // the dates of the period clauses take a literal too
    let (text, _) = round_trip(
        "SELECT account, sum(position) AS total FROM OPEN ON :from CLOSE ON :to CLEAR GROUP BY account ORDER BY account",
        &params,
    );
    assert!(text.contains("FROM OPEN ON 2024-02-01 CLOSE ON 2024-02-29 CLEAR"), "{}", text);

    // BQL dates run from the year 1 to 9999, so another day has no literal
    let query = Query::compile_with_params("SELECT :day AS day", &ParamTypes::new().bind("day", DataType::Date)).unwrap();
    for year in [0, 10000] {
        let day = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
        let error = query.inline_params(&Params::new().bind("day", day)).unwrap_err();
        assert_eq!(error.message, "parameter :day is of type date, which has no BQL literal");
    }
    let (text, _) = round_trip("SELECT :day AS day LIMIT 1", &Params::new().bind("day", date("0012-03-04")));
    assert_eq!(text, "SELECT 0012-03-04 AS day LIMIT 1");
}

#[test]
fn sets_are_written_as_set_calls() {
    for (tags, expected) in [
        (set(&["trip"]), vec!["Café \"Lumière\"", "it's \"quoted\""]),
        (set(&["food", "nope"]), vec!["Café \"Lumière\"", "O'Brien"]),
        (set(&[]), vec![]),
        (set(&["it's \"x\"", r"back\slash"]), vec![]),
    ] {
        let (text, result) = round_trip(
            "SELECT DISTINCT payee WHERE intersects(tags, :tags) ORDER BY payee",
            &Params::new().bind("tags", tags.clone()),
        );
        assert!(text.contains("intersects(tags, set("), "{}", text);
        assert_eq!(payees(&result), expected, "{}", text);
    }
    let (text, result) = round_trip(
        "SELECT DISTINCT account WHERE account IN :accounts ORDER BY account",
        &Params::new().bind("accounts", set(&["Assets:Bank", "Expenses:Travel"])),
    );
    assert_eq!(
        text,
        "SELECT DISTINCT account WHERE account IN set('Assets:Bank', 'Expenses:Travel') ORDER BY account"
    );
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn null_is_written_as_null() {
    // an optional filter: NULL keeps every row
    let sql = "SELECT count(*) AS n WHERE (:payee IS NULL OR payee = :payee) AND (:tags IS NULL OR intersects(tags, :tags))";
    let types = ParamTypes::new().bind("payee", DataType::Str).bind("tags", DataType::Set);
    let query = Query::compile_with_params(sql, &types).unwrap();
    for params in [
        Params::new().bind("payee", Value::Null).bind("tags", Value::Null),
        Params::new().bind("payee", "O'Brien").bind("tags", Value::Null),
        Params::new().bind("payee", Value::Null).bind("tags", set(&["trip"])),
    ] {
        let bound = query.execute_with_options(ledger(), &params, &options()).unwrap();
        let text = query.inline_params(&params).unwrap();
        let inlined = Query::compile(&text)
            .unwrap()
            .execute_with_options(ledger(), &Params::new(), &options())
            .unwrap();
        assert_eq!(bound, inlined, "{}", text);
    }
    let text = query
        .inline_params(&Params::new().bind("payee", Value::Null).bind("tags", Value::Null))
        .unwrap();
    assert_eq!(
        text,
        "SELECT count(*) AS n WHERE (NULL IS NULL OR payee = NULL) AND (NULL IS NULL OR intersects(tags, NULL))"
    );
}

#[test]
fn numbers_keep_their_type_and_scale() {
    for value in [Value::Int(0), Value::Int(42), Value::Int(-3), Value::Int(i64::MAX), Value::Int(i64::MIN)] {
        let (_, result) = round_trip(
            "SELECT :n AS n, str(:n) AS text, :n = :n AS same LIMIT 1",
            &Params::new().bind("n", value.clone()),
        );
        assert_eq!(result.rows[0][0], value);
    }
    for (number, written) in [
        ("12", "12."),
        ("12.50", "12.50"),
        ("-0.5", "(-0.5)"),
        ("0", "0."),
        ("1E+3", "1000."),
        ("-7.000", "(-7.000)"),
    ] {
        let (text, result) = round_trip(
            "SELECT :d AS d, str(:d) AS text, :d * 2 AS twice LIMIT 1",
            &Params::new().bind("d", decimal(number)),
        );
        assert!(text.starts_with(&format!("SELECT {} AS d", written)), "{}", text);
        assert_eq!(result.columns[0].ty, DataType::Decimal);
        // `str` shows the scale, so the written decimal has the same scale as the bound one
        assert_eq!(result.rows[0][1], Value::Str(zhang_query::decimal::to_plain_string(&decimal(number))));
    }
    // `x - :n` with a negative `n` is not a `--` comment
    let (text, _) = round_trip("SELECT 1 -:n AS m", &Params::new().bind("n", -1i64));
    assert_eq!(text, "SELECT 1 -(-1) AS m");
}

#[test]
fn counts_booleans_intervals_and_patterns_round_trip() {
    let (text, result) = round_trip(
        "SELECT date, payee WHERE :all OR account ~ '^Expenses:Travel' ORDER BY date, payee LIMIT :size OFFSET :skip",
        &Params::new().bind("all", false).bind("size", 1i64).bind("skip", 1i64),
    );
    assert_eq!(
        text,
        "SELECT date, payee WHERE FALSE OR account ~ '^Expenses:Travel' ORDER BY date, payee LIMIT 1 OFFSET 1"
    );
    assert_eq!((result.rows.len(), result.total), (1, Some(2)));

    for interval in [Interval::new(1, 0), Interval::new(0, -3), Interval::new(14, 2), Interval::new(0, 0)] {
        round_trip(
            "SELECT date, date + :step AS later WHERE account = 'Assets:Bank' ORDER BY date",
            &Params::new().bind("step", Value::Interval(interval)),
        );
    }

    let (text, _) = round_trip("JOURNAL :account", &Params::new().bind("account", "Assets:Bank"));
    assert_eq!(text, "JOURNAL 'Assets:Bank'");
}

#[test]
fn only_parameters_are_replaced() {
    // a parameter used twice, one in a string and one in a comment
    let (text, _) = round_trip(
        "SELECT payee, ':payee' AS literal -- by :payee\nWHERE payee = :payee OR narration = :payee",
        &Params::new().bind("payee", "O'Brien"),
    );
    assert_eq!(
        text,
        "SELECT payee, ':payee' AS literal -- by :payee\nWHERE payee = \"O'Brien\" OR narration = \"O'Brien\""
    );
    // positional parameters too
    let (text, _) = round_trip("SELECT count(*) AS n WHERE payee = $1 OR $2", &Params::new().push("C:\\temp\\new").push(true));
    assert_eq!(text, r"SELECT count(*) AS n WHERE payee = 'C:\temp\new' OR TRUE");
}

#[test]
fn inline_params_checks_the_parameters_like_an_execution() {
    let query = Query::compile_with_params("SELECT date WHERE date >= :from", &ParamTypes::new().bind("from", DataType::Date)).unwrap();
    let error = query.inline_params(&Params::new()).unwrap_err();
    assert_eq!(error.message, "parameter :from is not bound");
    let error = query.inline_params(&Params::new().bind("from", "2024-01-01")).unwrap_err();
    assert_eq!(error.message, "parameter :from was compiled as date but is bound to a str");

    // an amount has no literal
    let query = Query::compile_with_params("SELECT :a AS a", &ParamTypes::new().bind("a", DataType::Amount)).unwrap();
    let amount = Value::Amount(zhang_query::Amount::new(decimal("1"), "USD"));
    let error = query.inline_params(&Params::new().bind("a", amount)).unwrap_err();
    assert_eq!(error.message, "parameter :a is of type amount, which has no BQL literal");
    assert_eq!(error.column, Some(8));
}

#[test]
fn params_lists_each_parameter_once_in_text_order() {
    let types = ParamTypes::new()
        .bind("from", DataType::Date)
        .bind("tags", DataType::Set)
        .bind("size", DataType::Int)
        .bind("unused", DataType::Str)
        .push(DataType::Str);
    let query = Query::compile_with_params(
        "SELECT date, payee WHERE date >= :from AND (intersects(tags, :tags) OR payee = $1) AND date >= :from LIMIT :size",
        &types,
    )
    .unwrap();
    assert_eq!(
        query.params(),
        vec![
            (ParamRef::Named("from".into()), DataType::Date),
            (ParamRef::Named("tags".into()), DataType::Set),
            (ParamRef::Positional(1), DataType::Str),
            (ParamRef::Named("size".into()), DataType::Int),
        ]
    );
    assert!(Query::compile("SELECT 1").unwrap().params().is_empty());
}
