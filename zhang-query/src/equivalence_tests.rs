//! The execution decisions of the optimizer and the projector (deferred running balances,
//! running sums for `units(balance)` / `cost(balance)`, LIMIT pushdown) change how much work
//! an execution does, never its result: every query here returns byte-identical rows
//! (decimal scales included) with [`Query::compile`] and with [`Query::compile_naive`].

use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDate;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use crate::{ExecuteOptions, Params, Query};

fn load(dir: PathBuf) -> Ledger {
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger")
}

fn load_text(content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").into_path();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    load(dir)
}

fn fava_demo_ledger() -> Ledger {
    load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger"))
}

/// A deterministic ledger exercising the running sums: lots in two commodities with costs in
/// two currencies, numbers of several scales, zero costs, sales of lots that are not open
/// (so negative lots appear next to positive ones) and cash without cost.
fn random_lots_ledger(seed: u64, transactions: usize) -> String {
    let mut state = seed;
    let mut next = move |bound: u64| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) % bound
    };
    let mut ledger = String::from(
        "option \"operating_currency\" \"USD\"\n\
         1970-01-01 commodity USD\n1970-01-01 commodity EUR\n1970-01-01 commodity STK\n1970-01-01 commodity ETF\n\
         1970-01-01 open Assets:Broker\n1970-01-01 open Assets:Short\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Fees\n\
         2020-01-01 price STK 12.5 USD\n2021-06-01 price ETF 3.25 EUR\n2021-06-01 price EUR 1.1 USD\n",
    );
    let numbers = ["1", "2", "0.5", "1.000", "3.25", "10", "0.125"];
    let costs = ["0", "1.5", "10.25", "11.50", "12", "0.333"];
    let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    for idx in 0..transactions {
        let date = start + chrono::Duration::days(next(700) as i64);
        let (commodity, currency) = if next(3) == 0 { ("ETF", "EUR") } else { ("STK", "USD") };
        let units = numbers[next(numbers.len() as u64) as usize];
        let cost = costs[next(costs.len() as u64) as usize];
        let total = crate::decimal::to_plain_string(&crate::decimal::mul(&units.parse().unwrap(), &cost.parse().unwrap()));
        let postings = match next(6) {
            0 | 1 => format!("  Assets:Broker {units} {commodity} {{{cost} {currency}}}\n  Assets:Cash -{total} {currency}"),
            // a sale of a lot that may not be open: the uncovered part opens a negative lot
            2 => format!("  Assets:Broker -{units} {commodity} {{{cost} {currency}}}\n  Assets:Cash {total} {currency}"),
            3 => format!("  Assets:Short -{units} {commodity} {{{cost} {currency}}}\n  Assets:Cash {total} {currency}"),
            _ => format!("  Assets:Cash -{units} {currency}\n  Expenses:Fees {units} {currency}"),
        };
        ledger.push_str(&format!("\n{date} * \"trade {idx}\"\n{postings}\n"));
    }
    ledger
}

/// Queries over every kind of decision: deferred targets and aggregates, eager running totals
/// (ORDER BY, DISTINCT and GROUP BY over balances, min/max/sum), the linear rewrites, top-k,
/// stopping the scan, the first groups, LIMIT 0, and plans without a balance.
const QUERIES: &[&str] = &[
    "JOURNAL",
    "JOURNAL AT units",
    "JOURNAL AT cost",
    "JOURNAL AT value",
    "JOURNAL 'Broker' AT cost",
    "JOURNAL 'Cash|Short' AT units FROM year >= 2016",
    "BALANCES",
    "BALANCES AT cost",
    "SELECT date, balance ORDER BY date DESC LIMIT 10",
    "SELECT date, account, position, balance, units(balance), cost(balance) WHERE account ~ 'Broker|Short'",
    "SELECT date, units(balance), cost(balance), value(balance) FROM year >= 2016 WHERE account ~ 'Vanguard|Broker'",
    "SELECT date, cost(balance) ORDER BY date DESC, account LIMIT 7",
    "SELECT account, last(balance), first(balance), count(*) GROUP BY account",
    "SELECT account, last(cost(balance)), first(units(balance)) GROUP BY account ORDER BY account LIMIT 3",
    "SELECT date, last(units(balance)) GROUP BY date ORDER BY date DESC LIMIT 5",
    "SELECT account, min(balance), max(cost(balance)), sum(units(balance)) GROUP BY account",
    "SELECT account, last(balance) GROUP BY account LIMIT 2",
    "SELECT year, count(*), last(cost(balance)) GROUP BY year LIMIT 1",
    "SELECT last(str(balance)), first(convert(balance, 'USD'))",
    "SELECT str(balance) AS b, count(*) GROUP BY b ORDER BY b LIMIT 4",
    "SELECT DISTINCT account, units(balance)",
    "SELECT DISTINCT account LIMIT 3",
    "SELECT DISTINCT currency ORDER BY currency DESC LIMIT 2",
    "SELECT str(balance), length(str(cost(balance))) LIMIT 7",
    "SELECT date, balance ORDER BY balance LIMIT 3",
    "SELECT date, balance ORDER BY 2 DESC, 1 LIMIT 3",
    "SELECT possign(balance, account), abs(balance), neg(cost(balance)) WHERE account ~ 'Liab|Income|Short'",
    "SELECT only('USD', balance), filter_currency(balance, 'STK') LIMIT 50",
    "SELECT units(balance) = cost(balance), balance IS NULL, 'x' IN tags FROM month = 1",
    "SELECT date, balance LIMIT 0",
    "SELECT date, balance ORDER BY date LIMIT 0",
    "SELECT account, last(balance) GROUP BY account LIMIT 0",
    "SELECT * ORDER BY date DESC, account LIMIT 25",
    "SELECT account, count(*) GROUP BY account LIMIT 2",
    "SELECT payee, sum(position) GROUP BY payee ORDER BY payee LIMIT 4",
    "SELECT account, balance FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ 'Assets' LIMIT 20",
    "SELECT date, position LIMIT 5",
    // HAVING drops finished groups, so LIMIT never keeps only the first groups
    "SELECT account, count(*) GROUP BY account HAVING count(*) > 20 LIMIT 3",
    "SELECT account, last(balance), count(*) GROUP BY account HAVING count(*) > 5 AND last(balance) IS NOT NULL LIMIT 2",
    "SELECT year, account, sum(position) AS total GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY account, year LIMIT 10",
    "SELECT account, year, last(cost(balance)) AS b, count(*) AS n GROUP BY 1, 2 ORDER BY n DESC PIVOT BY account, year",
    // the other tables: top-k, stopping the scan, first groups, DISTINCT, HAVING and PIVOT BY
    "SELECT * FROM #entries ORDER BY date DESC, type LIMIT 7",
    "SELECT type, count(*) FROM #entries GROUP BY type LIMIT 2",
    "SELECT date, payee FROM #transactions LIMIT 5",
    "SELECT DISTINCT currency FROM #prices LIMIT 3",
    "SELECT currency, last(amount) FROM prices GROUP BY currency HAVING count(*) > 1 ORDER BY currency",
    "SELECT account, open.date, close FROM #accounts ORDER BY open.date DESC, account LIMIT 5",
    "SELECT account, year(date) AS y, count(*) AS n FROM #balances GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY account, y",
    "SELECT name, date FROM #commodities ORDER BY name DESC LIMIT 2",
];

/// Run every query both ways and compare the rows by their `Debug` form, which shows
/// decimal scales.
fn assert_equivalent(ledger: &Ledger, queries: &[&str]) {
    let options = ExecuteOptions {
        today: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
        timeout: None,
        max_result_values: None,
    };
    for sql in queries {
        let run = |query: Query| query.execute_with_options(ledger, &Params::new(), &options);
        let naive = run(Query::compile_naive(sql).unwrap_or_else(|err| panic!("{sql}: {err}")));
        let optimized = run(Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}")));
        match (naive, optimized) {
            (Ok(naive), Ok(optimized)) => {
                assert_eq!(naive.columns, optimized.columns, "{sql}");
                assert_eq!(format!("{:?}", naive.rows), format!("{:?}", optimized.rows), "{sql}");
            }
            (Err(naive), Err(optimized)) => assert_eq!(naive, optimized, "{sql}"),
            (naive, optimized) => panic!("{sql}: {:?} vs {:?}", naive.map(|it| it.rows.len()), optimized.map(|it| it.rows.len())),
        }
    }
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger() {
    assert_equivalent(&fava_demo_ledger(), QUERIES);
}

#[test]
fn random_ledgers_hold_what_the_running_sums_must_handle() {
    let ledger = load_text(&random_lots_ledger(1, 120));
    let count = |sql: &str| {
        let rows = Query::compile(sql).unwrap().execute(&ledger, &Params::new()).unwrap().rows;
        rows[0][0].as_int().unwrap()
    };
    // 120 transactions of two postings; reductions across lots split into more rows
    assert!(count("SELECT count(*)") > 240);
    // zero costs, negative lots, reductions of open lots, several scales and currencies
    assert!(count("SELECT count(*) WHERE cost_number = 0") > 5);
    assert!(count("SELECT count(*) WHERE account ~ 'Short' AND cost_number IS NOT NULL") > 5);
    assert!(count("SELECT count(*) WHERE account ~ 'Broker' AND number < 0 AND cost_number IS NOT NULL") > 5);
    assert!(count("SELECT count(*) WHERE str(number) ~ '[.]000$'") > 5);
    assert!(count("SELECT count(*) WHERE cost_currency = 'EUR'") > 5);
    let mixed = Query::compile("SELECT units(balance) WHERE account ~ 'Broker|Short'").unwrap();
    let rows = mixed.execute(&ledger, &Params::new()).unwrap().rows;
    // the running balance holds lots of both signs in one currency at some point
    let balances = Query::compile("SELECT balance WHERE account ~ 'Broker|Short'")
        .unwrap()
        .execute(&ledger, &Params::new())
        .unwrap()
        .rows;
    assert!(balances.iter().any(|row| {
        let crate::Value::Inventory(inventory) = &row[0] else { return false };
        let signs = inventory
            .positions()
            .filter(|it| it.units.commodity == "STK")
            .map(|it| it.units.number > 0.into())
            .collect::<std::collections::HashSet<_>>();
        signs.len() == 2
    }));
    assert_eq!(rows.len(), balances.len());
}

#[test]
fn decisions_keep_results_with_mixed_lots_scales_and_zero_costs() {
    for seed in 1..=6 {
        assert_equivalent(&load_text(&random_lots_ledger(seed, 120)), QUERIES);
    }
}

/// The running sums give exactly what `units()` and `cost()` of the running balance give,
/// row by row, however the lots of a currency mix signs, scales and zero costs.
#[test]
fn running_sums_equal_the_functions_of_the_balance() {
    for seed in 1..=40 {
        let ledger = load_text(&random_lots_ledger(seed, 60));
        assert_equivalent(
            &ledger,
            &[
                "SELECT units(balance), cost(balance)",
                "SELECT units(balance), cost(balance) WHERE account ~ 'Broker'",
                "SELECT units(balance), cost(balance) WHERE account ~ 'Short|Cash'",
            ],
        );
    }
}

#[test]
fn explain_shows_the_decisions() {
    let explain = |sql: &str| Query::compile(sql).unwrap().explain();
    let journal = explain("JOURNAL 'Broker' AT cost");
    assert!(journal.contains("rewrite: cost(balance) -> running cost\n"), "{journal}");
    assert!(journal.contains("balance: deferred targets [6]\n"), "{journal}");
    let top = explain("SELECT date, balance ORDER BY date DESC LIMIT 10");
    assert!(top.contains("limit: 10 (top-k while scanning)\nbalance: deferred targets [1]\n"), "{top}");
    let grouped = explain("SELECT account, last(balance), min(balance) GROUP BY account LIMIT 2");
    assert!(
        grouped.contains("limit: 2 (first groups only)\nbalance: running while scanning, deferred agg#0\n"),
        "{grouped}"
    );
    assert!(explain("SELECT DISTINCT account LIMIT 3").contains("limit: 3 (stops the scan)\n"));
    assert!(explain("SELECT DISTINCT account ORDER BY account LIMIT 3").contains("limit: 3\n"));
    assert!(explain("SELECT date, balance ORDER BY balance").contains("balance: running while scanning\n"));
    // value() prices by date: it is not rewritten
    let value = explain("JOURNAL AT value");
    assert!(!value.contains("rewrite:") && value.contains("balance: deferred targets [6]\n"), "{value}");
}

/// A plan that does not read `balance` keeps no running total at all.
#[test]
fn plans_without_balance_keep_no_running_total() {
    for sql in [
        "SELECT *",
        "SELECT account, sum(position) GROUP BY account",
        "BALANCES",
        "SELECT date, position ORDER BY date LIMIT 3",
    ] {
        let query = Query::compile(sql).unwrap();
        assert!(!query.plan.execution.running.used(), "{sql}");
        assert!(query.plan.execution.running.totals.is_empty() && !query.plan.execution.running.eager, "{sql}");
        assert!(!query.explain().contains("balance"), "{sql}: {}", query.explain());
    }
    assert!(Query::compile("SELECT units(balance)").unwrap().plan.execution.running.used());
}

/// The data tables compute some columns only when a plan reads them (the true balance of
/// `#balances`, the figures of `#budgets`): top-k, stopping the scan, first groups, DISTINCT
/// and HAVING over them return what the naive plans return.
#[test]
fn decisions_keep_results_on_the_data_tables() {
    const DATA_QUERIES: &[&str] = &[
        "SELECT date, account, actual, passed, discrepancy FROM #balances ORDER BY date DESC LIMIT 5",
        "SELECT account, count(*) FROM #balances WHERE NOT passed GROUP BY account LIMIT 1",
        "SELECT DISTINCT passed FROM #balances LIMIT 1",
        "SELECT account, last(actual) FROM #balances GROUP BY account HAVING count(*) > 1 ORDER BY account",
        "SELECT * FROM #documents ORDER BY date DESC LIMIT 2",
        "SELECT source, count(*) FROM #documents GROUP BY source LIMIT 1",
        "SELECT DISTINCT transaction_id FROM #documents LIMIT 1",
        "SELECT kind, id, span_start, span_end FROM #errors ORDER BY span_start DESC LIMIT 1",
        "SELECT name, last(available), last(closed) FROM #budgets GROUP BY name LIMIT 1",
        "SELECT date, name, activity FROM #budgets ORDER BY activity DESC LIMIT 2",
        "SELECT DISTINCT name FROM #budgets WHERE number(available) < 0 LIMIT 1",
        "SELECT name, sum(added) FROM #budgets GROUP BY name HAVING count(*) > 2",
        "SELECT * FROM #budget_events ORDER BY timestamp DESC LIMIT 2",
        "SELECT type, count(*) FROM #budget_events GROUP BY type LIMIT 2",
    ];
    let ledger = load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Travel
  budget: travel
2024-01-01 price USD 7 CNY
2024-01-01 budget food CNY
2024-01-01 budget travel CNY
2024-01-01 budget-add food 100 CNY
2024-01-02 * "Shop" "lunch"
  document: "a.pdf"
  Expenses:Food 30 CNY
    document: "b.pdf"
  Assets:Bank
2024-01-03 balance Assets:Bank -30 CNY
2024-01-04 balance Assets:Bank -20 CNY
2024-02-01 budget-transfer food travel 20 CNY
2024-02-02 * "Airline" "flight"
  Expenses:Travel 10 USD
  Assets:Bank -70 CNY
2024-02-03 document Assets:Bank "statement.pdf"
2024-02-04 balance Assets:Bank 0 CNY with pad Equity:Open
2024-03-01 budget-close travel
2024-03-02 * "Shop" "unbalanced"
  Expenses:Food 10 CNY
  Assets:Bank -9 CNY
"#,
    );
    assert_equivalent(&ledger, DATA_QUERIES);
    assert_equivalent(&fava_demo_ledger(), &DATA_QUERIES[..4]);
}
