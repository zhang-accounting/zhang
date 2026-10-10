//! The BALANCES and JOURNAL statements, the `AT` clause and the running `balance` column, against
//! beanquery (the engine tests of the statements are in `tests/engine/statements.rs`).
//!
//! The oracle tests compare whole results with beanquery's over
//! `integration-tests/fava-demo-ledger` (`tests/oracle/cases/statements/statements.json`, regenerated with
//! `tests/oracle/generate.py --set statements`) through `zhang_testkit::oracle`. Numbers
//! are compared numerically and inventory positions as sets; column types must match exactly
//! and column names up to the spelling documented in [`zhang_name`]. The plain `BALANCES`
//! statement is the conformance case `061_balances_plain` (the same query over the same
//! ledger and oracle) and is not repeated here.

use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_testkit::oracle::{load_query_list, run_case, Rules};

/// The name zhang gives a column beanquery calls `name`: zhang names the targets of BALANCES
/// and JOURNAL like the equivalent hand-written SELECT, in lower case and without the empty
/// `AT` function (beanquery: `SUM((position))`, `MAXWIDTH(payee, 48)`).
fn zhang_name(name: &str) -> String {
    name.to_lowercase().replace("((position))", "(position)")
}

fn oracle_case(index: usize) {
    let mut cases = load_query_list(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/statements/statements.json"));
    let case = &mut cases[index];
    case.strict_names = true;
    case.column_names = case.column_names.iter().map(|name| zhang_name(name)).collect();
    run_case(
        &zhang_testkit::fixtures::fava_demo(),
        case,
        &Rules::on(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
        &[],
        &[],
    )
    .assert_ok();
}

#[test]
fn oracle_balances_at_cost_from_where() {
    oracle_case(0);
}

#[test]
fn oracle_balances_at_value() {
    oracle_case(1);
}

#[test]
fn oracle_journal_from() {
    oracle_case(2);
}

#[test]
fn oracle_journal_collapses_whitespace_like_maxwidth() {
    oracle_case(3);
}

#[test]
fn oracle_journal_at_cost_over_lots() {
    oracle_case(4);
}

#[test]
fn oracle_journal_at_units_double_quoted() {
    oracle_case(5);
}

#[test]
fn oracle_balance_is_accumulated_before_order_by() {
    oracle_case(6);
}

#[test]
fn oracle_balance_in_an_aggregate() {
    oracle_case(7);
}
