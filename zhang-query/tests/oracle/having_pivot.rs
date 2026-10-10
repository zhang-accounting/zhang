//! Oracle tests of `HAVING` and `PIVOT BY`: runs the beanquery-generated fixtures in
//! `tests/oracle/cases/having_pivot` against the engine on the shared fava demo ledger, through
//! `zhang_testkit::oracle` with the comparison rules of the conformance suite
//! (`tests/oracle/README.md`): column types by position, decimals numerically,
//! inventories and unordered results as multisets, errors by class, CSV cells trimmed and
//! compared numerically when they are numbers.
//!
//! Unlike the conformance suite, column **names** are compared too: the columns of a pivot
//! are named after the data, so their names are part of the result. Every computed target of
//! these fixtures has an alias, so names never depend on how an engine spells an expression.
//!
//! The fixtures come from `tests/oracle/generate.py --set having_pivot`, which runs the
//! official beanquery with the conformance generator's validation (determinism, zhang's
//! balance-check rows).

use std::path::PathBuf;

use chrono::NaiveDate;
use zhang_testkit::oracle::{assert_no_failures, load_case_files, run_cases, Rules};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
}

#[test]
fn having_and_pivot_match_beanquery() {
    let mut fixtures = load_case_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/having_pivot"));
    assert!(fixtures.len() >= 20, "fixtures missing from tests/oracle/cases/having_pivot");
    // the names of pivoted columns are data, so every case compares them
    for fixture in &mut fixtures {
        fixture.strict_names = true;
    }
    let ledger = zhang_testkit::fixtures::fava_demo();
    assert_no_failures("having_pivot", &run_cases(&ledger, &fixtures, &Rules::on(today()), &[], &[]));
}
