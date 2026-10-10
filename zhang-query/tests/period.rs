//! Oracle tests of the FROM period modifiers (`OPEN ON`, `CLOSE [ON]`, `CLEAR`): runs the
//! beanquery-generated fixtures in `tests/period/cases` against the engine on the shared fava
//! demo ledger, through `zhang_testkit::oracle` with the comparison rules of the conformance
//! suite (`tests/conformance/README.md`): columns by position and type, decimals numerically,
//! inventories and unordered results as multisets, errors by class.
//!
//! The fixtures come from `tests/conformance/generate.py --set period`, which runs the official
//! beanquery with the conformance generator's validation (determinism, zhang's balance-check
//! rows). Case 022 retains beanquery's zero-row fixture; #647 deliberately returns a single
//! count of zero after CLOSE removes every posting, checked here as the exact accepted
//! deviation ([`ACCEPTED_DEVIATIONS`]).

use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_query::{DataType, ParamTypes, Params, Query, QueryErrorKind, QueryResult, Value};
use zhang_testkit::oracle::{assert_no_failures, check_lists, load_case_files, run_cases, Accepted, Deviation, Rules};

/// Deliberate differences between the engine and beanquery, checked as in `conformance.rs`.
const ACCEPTED_DEVIATIONS: &[Deviation] = &[Deviation {
    case: Some("close_before_the_ledger"),
    reason: "an aggregate without group keys returns one row over empty input (#647): a count of zero after CLOSE removes \
             every posting; beanquery 0.2.0 returns no rows",
    accepted: Accepted::Rows("[[0]]"),
}];

#[test]
fn period_modifiers_match_beanquery() {
    let fixtures = load_case_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/period/cases"));
    assert!(!fixtures.is_empty(), "no fixtures in tests/period/cases");
    check_lists(&fixtures, ACCEPTED_DEVIATIONS, &[]);
    let close_before = fixtures
        .iter()
        .find(|fixture| fixture.file == "022_close_before_the_ledger.json")
        .expect("case 022 of tests/period/cases");
    assert!(close_before.rows.is_empty(), "revisit the #647 deviation if the oracle changes");
    let ledger = zhang_testkit::fixtures::fava_demo();
    assert_no_failures("period", &run_cases(&ledger, &fixtures, &Rules::on(date(2025, 1, 1)), ACCEPTED_DEVIATIONS, &[]));
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn run(ledger: &zhang_core::ledger::Ledger, query: &str) -> QueryResult {
    Query::compile(query)
        .and_then(|query| query.execute_at(ledger, &Params::default(), date(2025, 1, 1)))
        .unwrap_or_else(|err| panic!("{}: {}", query, err))
}

fn cells(result: &QueryResult) -> Vec<Vec<String>> {
    result.rows.iter().map(|row| row.iter().map(ToString::to_string).collect()).collect()
}

#[test]
fn period_dates_can_be_parameters() {
    let ledger = zhang_testkit::ledger::fava_demo_ledger();
    let literal = run(
        &ledger,
        "SELECT account, sum(position) FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ '^Income' GROUP BY 1 ORDER BY 1",
    );
    let query = Query::compile_with_params(
        "SELECT account, sum(position) FROM OPEN ON :from CLOSE ON $1 WHERE account ~ '^Income' GROUP BY 1 ORDER BY 1",
        &ParamTypes::new().bind("from", DataType::Date).push(DataType::Date),
    )
    .unwrap();
    let params = Params::new().bind("from", date(2016, 1, 1)).push(date(2017, 1, 1));
    let bound = query.execute_at(&ledger, &params, date(2025, 1, 1)).unwrap();
    assert_eq!(bound, literal);
    assert!(query.explain().contains("period: OPEN ON :from CLOSE ON $1\n"), "{}", query.explain());

    // a NULL date and a CLOSE before OPEN are execution errors
    let null = query.execute_at(&ledger, &Params::new().bind("from", Value::Null).push(date(2017, 1, 1)), date(2025, 1, 1));
    assert!(null.unwrap_err().message.contains("OPEN ON needs a date"));
    let reversed = query.execute_at(&ledger, &Params::new().bind("from", date(2017, 1, 1)).push(date(2016, 1, 1)), date(2025, 1, 1));
    assert!(reversed.unwrap_err().message.contains("before the OPEN date"));

    // a parameter of another type is rejected when compiling
    let err = Query::compile_with_params("SELECT count(*) FROM OPEN ON :from", &ParamTypes::new().bind("from", DataType::Str))
        .err()
        .expect("a str parameter is rejected");
    assert_eq!(err.kind, QueryErrorKind::Compile);
}

/// Lots, costs, price conversions and the equity options on a small ledger whose expected
/// rows are worked out by hand from beancount's summarize, truncate and clear operations.
#[test]
fn period_modifiers_on_a_small_ledger() {
    let ledger = zhang_testkit::ledger::load_text(
        r#"
option "operating_currency" "USD"
option "account_previous_earnings" "Retained"
option "conversion_currency" "ZERO"
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Euro
1970-01-01 open Assets:Stock
1970-01-01 open Income:Salary
1970-01-01 open Expenses:Food
1970-01-01 open Equity:Opening-Balances

2023-01-10 * "salary"
  Assets:Cash  1000 USD
  Income:Salary  -1000 USD

2023-02-01 * "buy"
  Assets:Stock  10 AAA {10 USD}
  Assets:Cash  -100 USD

2023-03-01 * "buy more"
  Assets:Stock  5 AAA {12 USD}
  Assets:Cash  -60 USD

2023-06-01 * "exchange"
  Assets:Euro  90 EUR @ 1.1 USD
  Assets:Cash  -99 USD

2024-01-05 * "lunch"
  Expenses:Food  20 USD
  Assets:Cash  -20 USD

2024-02-01 * "sell"
  Assets:Stock  -10 AAA {10 USD}
  Assets:Cash  100 USD
"#,
    );

    // OPEN: per-lot opening balances against Equity:Opening-Balances at cost, earnings moved to
    // Equity:Retained (option), conversions to Equity:Conversions:Previous
    let opened = run(
        &ledger,
        "SELECT date, flag, account, position, other_accounts FROM OPEN ON 2024-01-01 WHERE flag = 'S' AND account !~ 'Opening'",
    );
    assert_eq!(
        cells(&opened),
        vec![
            vec!["2023-12-31", "S", "Assets:Cash", "741 USD", "Equity:Opening-Balances"],
            vec!["2023-12-31", "S", "Assets:Euro", "90 EUR", "Equity:Opening-Balances"],
            // other_accounts are the accounts of the entry's other postings, so a second lot shows
            vec![
                "2023-12-31",
                "S",
                "Assets:Stock",
                "10 AAA {10 USD, 2023-02-01}",
                "Assets:Stock, Equity:Opening-Balances"
            ],
            vec![
                "2023-12-31",
                "S",
                "Assets:Stock",
                "5 AAA {12 USD, 2023-03-01}",
                "Assets:Stock, Equity:Opening-Balances"
            ],
            // the conversions follow the order in which their currencies first appeared
            vec![
                "2023-12-31",
                "S",
                "Equity:Conversions:Previous",
                "99 USD",
                "Equity:Conversions:Previous, Equity:Opening-Balances"
            ],
            vec![
                "2023-12-31",
                "S",
                "Equity:Conversions:Previous",
                "-90 EUR",
                "Equity:Conversions:Previous, Equity:Opening-Balances"
            ],
            vec!["2023-12-31", "S", "Equity:Retained", "-1000 USD", "Equity:Opening-Balances"],
        ]
    );

    // the sale after OPEN reduces the summarized lot
    let holdings = run(&ledger, "SELECT sum(position) FROM OPEN ON 2024-01-01 WHERE account = 'Assets:Stock'");
    assert_eq!(cells(&holdings), vec![vec!["5 AAA {12 USD, 2023-03-01}"]]);

    // CLOSE: the conversion entry of the period, priced at zero in the conversion currency
    let closed = run(
        &ledger,
        "SELECT date, flag, account, position, price, weight FROM CLOSE ON 2024-01-01 WHERE flag = 'C' ORDER BY currency",
    );
    assert_eq!(
        cells(&closed),
        vec![
            vec!["2023-12-31", "C", "Equity:Conversions:Current", "-90 EUR", "0 ZERO", "0 ZERO"],
            vec!["2023-12-31", "C", "Equity:Conversions:Current", "99 USD", "0 ZERO", "0 ZERO"],
        ]
    );

    // CLEAR: transfers dated like the last entry, to Equity:Earnings:Current
    let cleared = run(
        &ledger,
        "SELECT date, flag, account, position FROM CLEAR WHERE flag = 'T' ORDER BY account, number",
    );
    assert_eq!(
        cells(&cleared),
        vec![
            vec!["2024-02-01", "T", "Equity:Earnings:Current", "-1000 USD"],
            vec!["2024-02-01", "T", "Equity:Earnings:Current", "20 USD"],
            vec!["2024-02-01", "T", "Expenses:Food", "-20 USD"],
            vec!["2024-02-01", "T", "Income:Salary", "1000 USD"],
        ]
    );

    // the whole period balances by weight: the conversion postings weigh 0 ZERO
    let net = run(&ledger, "SELECT sum(weight) FROM OPEN ON 2024-01-01 CLOSE ON 2024-12-31 CLEAR");
    assert_eq!(net.rows, vec![vec![Value::Inventory(Default::default())]]);
}
