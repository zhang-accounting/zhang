//! RFC #698's `both` mode on the budget baseline's ledgers ([`zhang_testkit::budget_ledger`]): each ledger's budgets
//! computed natively and by the bundled script `zhang:budget`, and what a client reads of them compared
//! ([`zhang_testkit::budget_parity`]). On demand, in release: a debug build takes minutes on 100,000 transactions.
//!
//! ```sh
//! cargo nextest run --release -p zhang-server --test golden --run-ignored only -E 'test(the_bench_ledgers)'
//! ```

use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::budget_parity::{compare, requests};

/// The first day of every month of `spec`.
fn months(spec: &Spec) -> Vec<String> {
    (0..spec.months)
        .map(|index| format!("{:04}-{:02}-01", 2021 + (index / 12) as i32, index % 12 + 1))
        .collect()
}

/// The two ledgers with budgets of the budget baseline, 10,000 and 100,000 transactions: every budget as of every
/// month, the pages of three budgets (one with the most postings, one sharing an account with it, one in JPY) in three
/// months, the three tables whole, the `budgets` of every posting and the errors, alike both ways.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "minutes in a debug build: run on demand, in release"]
async fn the_bench_ledgers_answer_alike_both_ways() {
    for spec in [Spec::new(10_000, 30), Spec::new(100_000, 30)] {
        let scratch = spec.generate().scratch();
        let budgets = ["Groceries", "Household", "Japan"].map(str::to_owned);
        let pages = ["2021-01-01", "2023-06-01", "2025-12-01"].map(str::to_owned);
        let report = compare(scratch.dir(), MAIN, spec.clock(), &requests(&months(&spec), &budgets, &pages)).await;
        let regressions = report.regressions().map(ToString::to_string).collect::<Vec<_>>();
        eprintln!(
            "{}: {} errors and {} requests compared, {} differences, {} not allowed",
            spec.name(),
            report.errors,
            report.requests,
            report.differences.len(),
            regressions.len()
        );
        assert!(regressions.is_empty(), "{}: the script differs:\n{}", spec.name(), regressions.join("\n"));
    }
}
