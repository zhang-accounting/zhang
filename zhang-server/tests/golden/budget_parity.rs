//! RFC #698's `both` mode on the budget baseline's ledgers ([`zhang_testkit::budget_ledger`]): each ledger's budgets
//! computed natively and by the bundled script `zhang:budget`, and what a client reads of them compared
//! ([`zhang_testkit::budget_parity`]). On demand, in release: a debug build takes minutes on 100,000 transactions. And
//! what the comparison allows, in every run.
//!
//! ```sh
//! cargo nextest run --release -p zhang-server --test golden --run-ignored only -E 'test(the_bench_ledgers)'
//! ```

use std::collections::BTreeMap;

use serde_json::json;
use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::budget_parity::{compare, differences, requests};

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

/// The testing guide allows the scale of a decimal to differ only in a value the answer types as a decimal: a
/// `decimal` cell, or the number of an `amount`, a `position` or an `inventory` cell. A text cell, a field outside a
/// table (an error's meta) and an amount's currency that read as the same number written otherwise still differ.
#[test]
fn the_scale_of_a_decimal_is_allowed_only_where_the_answer_types_a_decimal() {
    let table = |text: &str, number: &str| {
        json!({"data": {
            "columns": [
                {"name": "label", "type": "str"}, {"name": "activity", "type": "decimal"},
                {"name": "assigned", "type": "amount"}, {"name": "units", "type": "position"},
                {"name": "spent", "type": "inventory"}
            ],
            "rows": [[
                text, number, {"number": number, "currency": text},
                {"units": {"number": number, "currency": "CNY"}, "cost": {"number": number, "currency": "USD"}},
                {"positions": [{"units": {"number": number, "currency": "CNY"}, "cost": null}]}
            ]]
        }})
    };
    let found = differences("body", &table("500.00", "500.00"), &table("500", "500"));
    let verdicts = found.iter().map(|it| (it.path.as_str(), it.allowed.is_some())).collect::<BTreeMap<_, _>>();
    let expected = [
        ("body.data.rows[0][0]", false),
        ("body.data.rows[0][1]", true),
        ("body.data.rows[0][2].currency", false),
        ("body.data.rows[0][2].number", true),
        ("body.data.rows[0][3].cost.number", true),
        ("body.data.rows[0][3].units.number", true),
        ("body.data.rows[0][4].positions[0].units.number", true),
    ];
    assert_eq!(verdicts, BTreeMap::from(expected));
    // outside a table, an error's meta
    let meta = |value: &str| json!([{"kind": "BudgetCommodityMismatch", "metas": {"amount": value}}]);
    let found = differences("errors", &meta("500.00"), &meta("500"));
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].path.as_str(), found[0].allowed), ("errors[0].metas.amount", None));
    // a decimal cell of another value is no scale difference
    let found = differences("body", &table("x", "500.01"), &table("x", "500"));
    assert!(found.iter().all(|it| it.allowed.is_none()) && found.len() == 5, "{found:?}");
}
