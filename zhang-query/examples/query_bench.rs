//! Query-level timings on the budget baseline's generated ledgers ([`zhang_testkit::budget_ledger`]): what one query
//! of the engine costs, without the server around it. Build it in release, and time it on a quiet machine:
//!
//! ```sh
//! cargo run --release -p zhang-query --example query_bench -- cache 100000 7     # the first query of a load
//! cargo run --release -p zhang-query --example query_bench -- budgets 100000 7   # a budget analysis's aggregations
//! ```
//!
//! - `cache`: each round loads the ledger anew and times its first query, which builds the cache of the booked
//!   postings, then the same query again: the difference is the cache's build.
//! - `budgets`: the aggregation a budget analysis runs per budget (`plugins/budget/budget.lua`), the activity of its
//!   accounts per month summed in its commodity, for every budget in name order: the total of the 30 queries,
//!   compiled and executed as the analysis does, on a warm cache.
//!
//! The figures are the medians of the rounds, after one round that warms up.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use zhang_ast::Directive;
use zhang_core::ledger::Ledger;
use zhang_query::{Params, Query, Value};
use zhang_testkit::budget_ledger::Spec;

// the allocator of the `zhang` binary (`zhang-cli/src/allocator.rs`), which the timings depend on
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let number = |at: usize, default: usize| args.get(at).and_then(|it| it.parse().ok()).unwrap_or(default);
    let (transactions, rounds) = (number(1, 100_000), number(2, 7));
    let spec = Spec::new(transactions, 30);
    let generated = spec.generate();
    let scratch = generated.scratch();
    drop(generated);
    let load = || scratch.ledger_at(spec.clock()).expect("the ledger loads");
    match args.first().map(String::as_str) {
        Some("cache") => cache(&spec, load, rounds),
        Some("budgets") => {
            let ledger = load();
            // the cache and the price map, built
            let _ = zhang_query::execute(&ledger, "SELECT count(*) WHERE number(convert(position, 'USD', date)) > 0").expect("the query runs");
            budgets(&spec, &ledger, rounds)
        }
        Some("sql") => {
            let ledger = load();
            for bql in &args[3..] {
                let started = Instant::now();
                let rows = zhang_query::execute(&ledger, bql).expect("the query runs").rows;
                println!("{:.2} ms: {bql}", ms(started.elapsed()));
                for row in rows {
                    println!("  {}", row.iter().map(ToString::to_string).collect::<Vec<_>>().join(" | "));
                }
            }
        }
        _ => eprintln!("usage: query_bench cache|budgets [<transactions> [<rounds>]] | sql <transactions> - <bql>..."),
    }
}

fn ms(it: Duration) -> f64 {
    it.as_secs_f64() * 1000.0
}

/// The median of `samples`.
fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn run(ledger: &Ledger, query: &Query, params: &Params) -> Vec<Vec<Value>> {
    query.execute(ledger, params).expect("the query runs").rows
}

/// The first query of a load, which builds the cache, and the same query again.
fn cache(spec: &Spec, load: impl Fn() -> Ledger, rounds: usize) {
    let query = Query::compile("SELECT count(*) WHERE account = 'Expenses:Food:Groceries'").expect("the query compiles");
    let (mut first, mut again) = (vec![], vec![]);
    for round in 0..=rounds {
        let ledger = load();
        let started = Instant::now();
        let rows = run(&ledger, &query, &Params::new());
        let cold = started.elapsed();
        let started = Instant::now();
        assert_eq!(run(&ledger, &query, &Params::new()), rows);
        let warm = started.elapsed();
        if round > 0 {
            first.push(cold);
            again.push(warm);
        }
    }
    let (first, again) = (median(&mut first), median(&mut again));
    println!(
        "{} cache: first query {:.2} ms, again {:.3} ms, the cache's build {:.2} ms (medians of {rounds})",
        spec.name(),
        ms(first),
        ms(again),
        ms(first.saturating_sub(again))
    );
}

/// A budget as its first definition defines it.
struct Budget {
    name: String,
    commodity: String,
    seq: u32,
    accounts: BTreeSet<String>,
}

fn defined(ledger: &Ledger) -> Vec<Budget> {
    let mut accounts = ledger.budget_accounts();
    let mut budgets: Vec<Budget> = vec![];
    for (_, directive, outcome) in ledger.entries() {
        if let Directive::Budget(budget) = &directive.data {
            if budgets.iter().all(|it| it.name != budget.name) {
                budgets.push(Budget {
                    name: budget.name.clone(),
                    commodity: budget.commodity.clone(),
                    seq: outcome.seq.expect("an entry has its place"),
                    accounts: accounts.remove(&budget.name).unwrap_or_default(),
                });
            }
        }
    }
    budgets.sort_by(|a, b| a.name.cmp(&b.name));
    budgets
}

/// Time `queries` (each compiled and executed, as an analysis does) `rounds` times: the median of their total, and
/// the rows they return.
fn timed(ledger: &Ledger, queries: &[(&str, Params)], rounds: usize) -> (Duration, usize) {
    let mut totals = vec![];
    let mut rows = 0;
    for round in 0..=rounds {
        let started = Instant::now();
        rows = 0;
        for (bql, params) in queries {
            let query = Query::compile_with_params(bql, &params.types()).expect("the query compiles");
            rows += run(ledger, &query, params).len();
        }
        if round > 0 {
            totals.push(started.elapsed());
        } else if std::env::var_os("QUERY_BENCH_COLD").is_some() {
            // the first round, in memory the process has not used yet, as an analysis right after a load runs
            println!("  cold: {:.2} ms", ms(started.elapsed()));
        }
    }
    (median(&mut totals), rows)
}

/// The aggregations of a budget analysis (`budget.lua`) on a warm cache: the activity of every budget per month, one
/// aggregation per budget or one per commodity grouped with unnest(), and the commodities held on the budgets'
/// accounts. `QUERY_BENCH_ONLY=<text>` times only the measurements whose label has the text.
#[inline(never)]
fn budgets(spec: &Spec, ledger: &Ledger, rounds: usize) {
    let only = std::env::var("QUERY_BENCH_ONLY").unwrap_or_default();
    let measure = |label: &str, queries: &[(&str, Params)]| {
        if label.contains(only.as_str()) {
            let (took, rows) = timed(ledger, queries, rounds);
            println!(
                "{} {label}: {} queries, {rows} rows, {:.2} ms (median of {rounds})",
                spec.name(),
                queries.len(),
                ms(took)
            );
        }
    };
    let budgets = defined(ledger);
    let activity = "SELECT seq < :defined_seq AS kind, yearmonth(date) AS month, \
        sum(CASE WHEN currency = :commodity THEN number ELSE number(convert(position, :commodity, date)) END) AS activity \
        WHERE account IN :accounts AND open_meta_has(account, 'budget', :label, date, time) GROUP BY kind, month";
    let per_budget = budgets
        .iter()
        .map(|budget| {
            let params = Params::new()
                .bind("accounts", Value::Set(budget.accounts.clone()))
                .bind("label", budget.name.as_str())
                .bind("defined_seq", budget.seq as i64)
                .bind("commodity", budget.commodity.as_str());
            (activity, params)
        })
        .collect::<Vec<_>>();
    measure("budgets one by one", &per_budget);

    let accounts = Params::new().bind("accounts", Value::Set(budgets.iter().flat_map(|it| it.accounts.iter().cloned()).collect()));
    let held = "SELECT account, currency, cost_currency, min(date) AS since WHERE account IN :accounts GROUP BY account, currency, cost_currency";
    measure("held commodities", &[(held, accounts.clone())]);

    // the activity of every budget per month in one pass per commodity, grouped by the budgets of each posting, with
    // the first seq and the postings no price converts, as budget.lua aggregates it
    let grouped = "SELECT unnest(open_meta_values(account, 'budget', date, time)) AS budget, yearmonth(date) AS month, \
        min(seq) AS first, \
        count(CASE WHEN currency != :commodity AND currency(convert(position, :commodity, date)) != :commodity THEN 1 END) AS unconverted, \
        sum(CASE WHEN currency = :commodity THEN number ELSE number(convert(position, :commodity, date)) END) AS activity \
        WHERE account IN :accounts GROUP BY budget, month";
    let mut commodities = budgets.iter().map(|budget| budget.commodity.as_str()).collect::<Vec<_>>();
    commodities.sort_unstable();
    commodities.dedup();
    let per_commodity = commodities
        .iter()
        .map(|commodity| {
            let accounts = budgets
                .iter()
                .filter(|budget| budget.commodity == *commodity)
                .flat_map(|budget| budget.accounts.iter().cloned())
                .collect();
            (grouped, Params::new().bind("accounts", Value::Set(accounts)).bind("commodity", *commodity))
        })
        .collect::<Vec<_>>();
    measure("budgets grouped by unnest()", &per_commodity);

    // the rows alone: what assembling the rows of the budgets' accounts costs, as every query over them pays it
    measure(
        "rows of every budget's accounts",
        &[("SELECT count(*) WHERE account IN :accounts", accounts.clone())],
    );

    // what one pass over the budgets' accounts costs, without any budget's own membership
    let one_pass = "SELECT account, yearmonth(date) AS month, \
        sum(CASE WHEN currency = 'USD' THEN number ELSE number(convert(position, 'USD', date)) END) AS activity \
        WHERE account IN :accounts GROUP BY account, month";
    measure("one pass over every budget's accounts", &[(one_pass, accounts)]);
}
