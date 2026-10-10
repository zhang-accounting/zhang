//! Golden tests: the representative queries of issue #434 over
//! `integration-tests/fava-demo-ledger`, compared with results produced by beanquery
//! (`tests/oracle/cases/golden/fava_demo.json`, regenerated with `tests/oracle/generate.py --set golden`)
//! through `zhang_testkit::oracle`.
//!
//! Numbers are compared numerically (`4.0 = 4.00`), inventory positions as sets, and the
//! column names and types exactly.

use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_testkit::oracle::{load_query_list, run_case, Rules};

fn golden_case(index: usize) {
    let mut cases = load_query_list(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/golden/fava_demo.json"));
    let case = &mut cases[index];
    // beanquery's column names are part of these results
    case.strict_names = true;
    let ledger = zhang_testkit::fixtures::fava_demo();
    run_case(&ledger, case, &Rules::on(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()), &[], &[]).assert_ok();
}

#[test]
fn monthly_expenses_by_category() {
    golden_case(0);
}

#[test]
fn spending_by_payee_top_20() {
    golden_case(1);
}

#[test]
fn postings_with_tag() {
    golden_case(2);
}

#[test]
fn holdings_at_cost_and_market_value() {
    golden_case(3);
}

#[test]
fn count_and_sum_of_food_expenses() {
    golden_case(4);
}
