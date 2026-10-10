//! Oracle tests of the FROM period modifiers (`OPEN ON`, `CLOSE [ON]`, `CLEAR`): runs the
//! beanquery-generated fixtures in `tests/oracle/cases/period` against the engine on the shared fava
//! demo ledger, through `zhang_testkit::oracle` with the comparison rules of the conformance
//! suite (`tests/oracle/README.md`): columns by position and type, decimals numerically,
//! inventories and unordered results as multisets, errors by class.
//!
//! The fixtures come from `tests/oracle/generate.py --set period`, which runs the official
//! beanquery with the conformance generator's validation (determinism, zhang's balance-check
//! rows). Case 022 retains beanquery's zero-row fixture; #647 deliberately returns a single
//! count of zero after CLOSE removes every posting, checked here as the exact accepted
//! deviation ([`ACCEPTED_DEVIATIONS`]).

use std::path::PathBuf;

use chrono::NaiveDate;
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
    let fixtures = load_case_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/cases/period"));
    assert!(!fixtures.is_empty(), "no fixtures in tests/oracle/cases/period");
    check_lists(&fixtures, ACCEPTED_DEVIATIONS, &[]);
    let close_before = fixtures
        .iter()
        .find(|fixture| fixture.file == "022_close_before_the_ledger.json")
        .expect("case 022 of tests/oracle/cases/period");
    assert!(close_before.rows.is_empty(), "revisit the #647 deviation if the oracle changes");
    let ledger = zhang_testkit::fixtures::fava_demo();
    assert_no_failures(
        "period",
        &run_cases(
            &ledger,
            &fixtures,
            &Rules::on(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
            ACCEPTED_DEVIATIONS,
            &[],
        ),
    );
}
