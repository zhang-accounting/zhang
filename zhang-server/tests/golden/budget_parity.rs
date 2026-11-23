//! RFC #698's `both` mode on generated ledgers: each ledger's budgets computed natively and by the bundled script
//! `zhang:budget`, and what a client reads of them compared ([`zhang_testkit::budget_parity`]). The budget baseline's
//! ledgers ([`zhang_testkit::budget_ledger`]) on demand, in release: a debug build takes minutes on 100,000
//! transactions. In every run, ledgers of many closed budgets, which also count the script's queries, and what the
//! comparison allows.
//!
//! ```sh
//! cargo nextest run --release -p zhang-server --test golden --run-ignored only -E 'test(the_bench_ledgers)'
//! ```

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Months, NaiveDate};
use serde_json::{json, Value};
use zhang_core::analysis::{AnalysisCatalog, AnalysisHost, AnalyzerTables, LentLedger};
use zhang_core::clock::Clock;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::router::{Conversion, ConvertedAmount, QueryFailure, RawQueryParam, RouterHost};
use zhang_core::plugin_policy::PluginPolicy;
use zhang_server::analysis::{analysis_host, load_options_for, BudgetImpl};
use zhang_shared::plugin_abi::{analysis as abi, HostError};
use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::budget_parity::{compare, differences, published, requests};
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::ledger::Scratch;

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

/// The server's host of analyses ([`zhang_server::analysis::analysis_host`]), which counts the queries the analysis of a
/// load runs and times it, from the loan of the ledger to the analyzer to the publication of its output.
#[derive(Default)]
struct Measured {
    queries: Arc<AtomicUsize>,
    since: Mutex<Option<Instant>>,
    took: Mutex<Option<Duration>>,
}

impl Measured {
    fn published(&self) {
        let since = self.since.lock().unwrap().take();
        *self.took.lock().unwrap() = since.map(|since| since.elapsed());
    }
}

impl AnalysisHost for Measured {
    fn catalog(&self, analyzers: &[AnalyzerTables<'_>]) -> AnalysisCatalog {
        analysis_host().catalog(analyzers)
    }

    fn queries(&self, ledger: LentLedger) -> Arc<dyn RouterHost> {
        *self.since.lock().unwrap() = Some(Instant::now());
        Arc::new(Counted {
            host: analysis_host().queries(ledger),
            queries: self.queries.clone(),
        })
    }

    fn publish(&self, ledger: &Ledger, provider: &str, output: &abi::Output) -> Result<Arc<dyn Any + Send + Sync>, String> {
        self.published();
        analysis_host().publish(ledger, provider, output)
    }

    fn publish_raw(&self, ledger: &Ledger, provider: &str, output: &abi::RawOutput<'_>) -> Option<Arc<dyn Any + Send + Sync>> {
        self.published();
        analysis_host().publish_raw(ledger, provider, output)
    }
}

/// The queries of an analysis, counted.
struct Counted {
    host: Arc<dyn RouterHost>,
    queries: Arc<AtomicUsize>,
}

impl RouterHost for Counted {
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        self.host.query(bql)
    }

    fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        self.host.query_with_params(bql, params)
    }

    fn schema(&self) -> Result<Value, QueryFailure> {
        self.host.schema()
    }

    fn convert(&self, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
        self.host.convert(conversions)
    }
}

/// What the script's analysis of a ledger cost: how many queries it ran, and how long it took when it published.
struct Cost {
    queries: usize,
    took: Option<Duration>,
}

/// The ledger `dir/main.zhang` loaded with the bundled script `zhang:budget` computing its budgets, or with `script`
/// in its place: what its analysis cost, and whether the script published (an error when it did not).
fn load_with_the_script(dir: &Path, clock: Clock, script: Option<&Arc<[u8]>>) -> (Cost, Result<(), String>) {
    let measured = Arc::new(Measured::default());
    let mut options = load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Plugin);
    options.analysis = Some(measured.clone());
    if let Some(content) = script {
        for plugin in &mut options.default_plugins {
            plugin.content = content.clone();
        }
    }
    let outcome = Ledger::load_with_options(dir.to_path_buf(), MAIN.to_owned(), data_source_for(MAIN), clock, options)
        .map_err(|error| error.to_string())
        .and_then(|ledger| published(&ledger));
    let cost = Cost {
        queries: measured.queries.load(Ordering::Relaxed),
        took: *measured.took.lock().unwrap(),
    };
    (cost, outcome)
}

/// The second Sunday of March of `year`, when the clocks of New York skip from 02:00 to 03:00.
fn spring_forward(year: i32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, 3, 1).expect("a date");
    first + Days::new((7 - u64::from(first.weekday().num_days_from_sunday())) % 7 + 7)
}

/// RFC #698's review probe of the script's queries: a ledger of `count` budgets, one defined a day and closed some
/// days later, as a long-lived ledger that closes a budget per category every month. Each budget has an account of its
/// own, which every fourth budget shares with the next one and every tenth names a budget never defined in. The
/// postings come before, in and after each budget's window, in its commodity, in a priced one and in one no price
/// converts, on debit- and credit-normal accounts, around closes with and without a time, and around closes at a time
/// the timezone skips.
fn closed_budgets(count: usize) -> String {
    let mut text = String::from(
        "option \"operating_currency\" \"CNY\"\noption \"timezone\" \"America/New_York\"\n\
         1970-01-01 commodity CNY\n1970-01-01 commodity USD\n1970-01-01 commodity XYZ\n\
         2000-01-01 open Assets:Bank\n2000-01-01 price USD 7.10 CNY\n",
    );
    let start = NaiveDate::from_ymd_opt(2015, 1, 1).expect("a date");
    let posting = |text: &mut String, stamp: String, account: &str, amount: &str| {
        writeln!(text, "{stamp} * \"P\" \"\"\n  {account} {amount}\n  Assets:Bank").expect("a string");
    };
    for i in 0..count {
        let name = format!("B{i:04}");
        let defined = start + Days::new(i as u64);
        let account = format!("{}:B{i:04}", if i % 7 == 3 { "Income" } else { "Expenses" });
        let dst = spring_forward(defined.year());
        let (closed, time) = if i % 2 == 0 && dst > defined + Days::new(3) && dst <= defined + Days::new(20) {
            (dst, Some("02:30:00"))
        } else if i % 3 == 1 {
            (defined + Days::new(12 + (i % 9) as u64), Some("12:00:00"))
        } else {
            (defined + Days::new(12 + (i % 9) as u64), None)
        };
        writeln!(text, "{} open {account}\n  budget: \"{name}\"", defined - Days::new(40)).expect("a string");
        if i % 4 == 0 {
            writeln!(text, "  budget: \"B{:04}\"", i + 1).expect("a string");
        }
        if i % 10 == 9 {
            writeln!(text, "  budget: \"Nowhere{i}\"").expect("a string");
        }
        if i % 3 == 0 {
            posting(&mut text, (defined - Days::new(20 + (i % 11) as u64)).to_string(), &account, "5 CNY");
        }
        if i % 5 == 1 {
            // folded before the definition, on its day
            posting(&mut text, defined.to_string(), &account, "1.5 CNY");
        }
        writeln!(text, "{defined} budget {name} CNY\n{defined} budget-add {name} 100.00 CNY").expect("a string");
        posting(&mut text, (defined + Days::new(1)).to_string(), &account, "10.50 CNY");
        posting(&mut text, (defined + Days::new(2)).to_string(), &account, "2.50 USD");
        posting(&mut text, (defined + Days::new(3)).to_string(), &account, "-10.50 CNY");
        if i % 5 == 2 {
            posting(&mut text, (defined + Days::new(3)).to_string(), &account, "7 XYZ");
        }
        match time {
            Some("02:30:00") => {
                // the close is read at 03:30: a posting at 03:15 comes after it as written but before it in the fold
                posting(&mut text, format!("{closed} 01:00:00"), &account, "3 CNY");
                writeln!(text, "{closed} 02:30:00 budget-close {name}").expect("a string");
                posting(&mut text, format!("{closed} 03:15:00"), &account, "4 CNY");
                posting(&mut text, format!("{closed} 03:15:00"), &account, "2 XYZ");
                posting(&mut text, format!("{closed} 03:45:00"), &account, "5 CNY");
            }
            Some(time) => {
                posting(&mut text, format!("{closed} 09:00:00"), &account, "3.25 CNY");
                writeln!(text, "{closed} {time} budget-close {name}").expect("a string");
                posting(&mut text, format!("{closed} 15:00:00"), &account, "4 CNY");
            }
            None => {
                writeln!(text, "{closed} budget-close {name}").expect("a string");
                posting(&mut text, closed.to_string(), &account, "3.25 CNY");
            }
        }
        posting(&mut text, (closed + Days::new(2)).to_string(), &account, "4 XYZ");
        posting(&mut text, (closed + Days::new(3)).to_string(), &account, "1 CNY");
    }
    text
}

/// What a client reads of a generated ledger's budgets: the definitions and events whole, the `budgets` of every
/// posting, the errors page, the months of the budgets whose names are in the ranges `names`, every budget as of
/// `months`, and the pages of `budgets` in `pages`.
fn requests_of(names: &[(String, String)], months: &[String], budgets: &[String], pages: &[&str]) -> Vec<Value> {
    let mut requests = vec![
        json!({"bql": "SELECT name, date, currency, alias, category, accounts, close, close_time FROM #budget_definitions"}),
        json!({"bql": "SELECT name, date, time, timestamp, type, amount FROM #budget_events"}),
        json!({"bql": "SELECT seq, posting_index, date, time, account, units(position) AS units, budgets FROM #postings WHERE budgets IS NOT NULL"}),
        json!({"builtin": "journals.errors", "params": {"size": 1000, "offset": 0}}),
    ];
    for (from, to) in names {
        requests.push(json!({"bql": format!(
            "SELECT name, date, assigned, added, activity, available, closed FROM #budgets WHERE name >= '{from}' AND name < '{to}'"
        )}));
    }
    for month in months {
        requests.push(json!({"builtin": "budgets.month", "params": {"month": month}}));
    }
    for name in budgets {
        for month in pages {
            requests.push(json!({"batch": [
                {"name": "budgets.budget", "params": {"name": name}},
                {"name": "budgets.budget_month", "params": {"name": name, "month": month}},
                {"name": "budgets.events", "params": {"name": name, "month": month}},
                {"name": "budgets.postings", "params": {"name": name, "month": month}},
            ]}));
        }
    }
    requests
}

/// What a client reads of the budgets of [`closed_budgets`]: the months of the first, middle and last hundred budgets,
/// every budget as of a month a year, and the pages of a few budgets.
fn probe_requests(count: usize) -> Vec<Value> {
    let names = [0, count / 2, count - 100].map(|from| (format!("B{from:04}"), format!("B{:04}", from + 100)));
    let months = (0..count / 365 + 1).map(|year| format!("{}-06-01", 2015 + year)).collect::<Vec<_>>();
    let budgets = [0, 1, 3, 4, count / 2, count - 1].map(|i| format!("B{i:04}"));
    requests_of(&names, &months, &budgets, &["2015-01-01", "2015-02-01", "2017-03-01"])
}

/// The other shape of a long-lived ledger that closes a budget per category every month: an account per category,
/// whose one `open` names every monthly budget of its category, each defined on the first of its month and closed on
/// its last day (at noon every third month, between postings of that day). Every posting is then in all the budgets
/// of its account: before the definition of the later ones, and after the close of the earlier ones.
fn monthly_budgets(categories: usize, months: u32) -> String {
    let mut text = String::from(
        "option \"operating_currency\" \"CNY\"\noption \"timezone\" \"Asia/Shanghai\"\n\
         1970-01-01 commodity CNY\n1970-01-01 commodity USD\n2000-01-01 open Assets:Bank\n2000-01-01 price USD 7.10 CNY\n",
    );
    let start = NaiveDate::from_ymd_opt(2015, 1, 1).expect("a date");
    for category in 0..categories {
        writeln!(text, "2014-12-01 open Expenses:C{category}").expect("a string");
        for month in 0..months {
            writeln!(text, "  budget: \"C{category}-{month:03}\"").expect("a string");
        }
    }
    let posting = |text: &mut String, stamp: String, account: &str, amount: &str| {
        writeln!(text, "{stamp} * \"P\" \"\"\n  {account} {amount}\n  Assets:Bank").expect("a string");
    };
    for month in 0..months {
        let first = start + Months::new(month);
        let last = first + Months::new(1) - Days::new(1);
        for category in 0..categories {
            let (name, account) = (format!("C{category}-{month:03}"), format!("Expenses:C{category}"));
            writeln!(text, "{first} budget {name} CNY\n{first} budget-add {name} 300 CNY").expect("a string");
            posting(&mut text, (first + Days::new(4)).to_string(), &account, "12.50 CNY");
            posting(&mut text, (first + Days::new(14)).to_string(), &account, "3.00 USD");
            posting(&mut text, (first + Days::new(24)).to_string(), &account, "-2.5 CNY");
            if month % 3 == 0 && category % 2 == 0 {
                posting(&mut text, format!("{last} 09:00:00"), &account, "1.25 CNY");
                writeln!(text, "{last} 12:00:00 budget-close {name}").expect("a string");
                posting(&mut text, format!("{last} 18:00:00"), &account, "2 CNY");
            } else {
                writeln!(text, "{last} budget-close {name}").expect("a string");
            }
        }
    }
    text
}

/// `dir`'s budgets alike natively and with the script for every request of `requests`, but for the differences the
/// testing guide allows; what was compared, and the kinds of the errors, as `what`.
async fn assert_alike(what: &str, dir: &Path, clock: Clock, requests: &[Value]) -> std::collections::BTreeSet<String> {
    let report = compare(dir, MAIN, clock, requests).await;
    let regressions = report.regressions().map(ToString::to_string).collect::<Vec<_>>();
    eprintln!(
        "{what}: {} errors ({:?}) and {} requests compared, {} differences, {} not allowed",
        report.errors,
        report.kinds,
        report.requests,
        report.differences.len(),
        regressions.len()
    );
    assert!(regressions.is_empty(), "{what}: the script differs:\n{}", regressions.join("\n"));
    report.kinds
}

/// The ledger `dir/main.zhang` loaded with the script, which publishes: how many queries it ran.
fn queries_of(what: &str, dir: &Path, clock: Clock) -> usize {
    let (cost, outcome) = load_with_the_script(dir, clock, None);
    outcome.unwrap_or_else(|error| panic!("{what}, after {} queries: {error}", cost.queries));
    eprintln!("{what}: {} queries, analysed in {:?}", cost.queries, cost.took.unwrap_or_default());
    cost.queries
}

/// The clock of [`closed_budgets`]
fn probe_clock() -> Clock {
    Clock::Fixed(chrono::DateTime::parse_from_rfc3339("2025-03-01T12:00:00Z").unwrap().to_utc())
}

/// RFC #698's review probe, small: the script used to run up to three queries per budget it could not aggregate with
/// the others (closed, with postings before its definition or that no price converts), so a ledger of closed budgets
/// ran a few hundred queries more for 200 budgets more. It now runs a few queries per budget commodity, as many for
/// 400 closed budgets as for 200, and answers as the native budgets do.
#[tokio::test(flavor = "multi_thread")]
async fn the_budget_script_runs_as_many_queries_on_400_closed_budgets_as_on_200() {
    let (small, large) = (Scratch::zhang(&closed_budgets(200)), Scratch::zhang(&closed_budgets(400)));
    let queries = [
        queries_of("200 closed budgets", small.dir(), probe_clock()),
        queries_of("400 closed budgets", large.dir(), probe_clock()),
    ];
    assert!(
        queries[0] == queries[1] && queries[1] <= 10,
        "the queries grow with the closed budgets: {queries:?}"
    );
    let kinds = assert_alike("400 closed budgets", large.dir(), probe_clock(), &probe_requests(400)).await;
    for kind in ["BudgetDoesNotExist", "BudgetClosed", "BudgetCommodityMismatch"] {
        assert!(kinds.contains(kind), "the probe reports no {kind}: {kinds:?}");
    }
}

/// RFC #698's review probe at its size: 3,600 closed budgets, past the 10,000 queries of an analysis
/// (`zhang_server::analysis::MAX_QUERIES`) the script used to run at about 3,500. It publishes in a few queries (one
/// more page of the postings read one by one than on 40 budgets), and answers as the native budgets do. On demand,
/// about 15 s in a debug build:
///
/// ```sh
/// cargo nextest run --release -p zhang-server --test golden --run-ignored only -E 'test(thousands_of_closed_budgets)'
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "about 15 s in a debug build: run on demand, in release"]
async fn the_budget_script_reads_thousands_of_closed_budgets_in_a_few_queries() {
    let (small, large) = (Scratch::zhang(&closed_budgets(40)), Scratch::zhang(&closed_budgets(3_600)));
    let queries = [
        queries_of("40 closed budgets", small.dir(), probe_clock()),
        queries_of("3,600 closed budgets", large.dir(), probe_clock()),
    ];
    assert!(
        queries[1] <= queries[0] + 1 && queries[1] <= 10,
        "the queries grow with the closed budgets: {queries:?}"
    );
    assert_alike("3,600 closed budgets", large.dir(), probe_clock(), &probe_requests(3_600)).await;
}

/// [`monthly_budgets`] of 4 categories over 110 months: 440 budgets, each with postings in all 110 months, so that the
/// script's groupings by budget and month have more groups (48,400) than the 40,000 it reads at once: it reads them by
/// ranges of months, which a census of the postings sizes, and answers as the native budgets do.
#[tokio::test(flavor = "multi_thread")]
async fn the_budget_script_reads_budgets_of_every_month_on_one_account_by_ranges_of_months() {
    let clock = Clock::Fixed(chrono::DateTime::parse_from_rfc3339("2025-01-15T12:00:00Z").unwrap().to_utc());
    let scratch = Scratch::zhang(&monthly_budgets(4, 110));
    let queries = queries_of("440 monthly budgets", scratch.dir(), clock);
    // a census of the postings, then each grouping by budget in two ranges of months
    assert!((10..=20).contains(&queries), "{queries} queries");
    let names = [("C0-".to_owned(), "C0.".to_owned()), ("C3-".to_owned(), "C3.".to_owned())];
    let months = ["2015-01-01", "2019-06-01", "2024-02-01"].map(str::to_owned);
    let budgets = ["C0-000", "C0-060", "C1-109", "C3-033"].map(str::to_owned);
    let requests = requests_of(&names, &months, &budgets, &["2015-01-01", "2020-01-01"]);
    assert_alike("440 monthly budgets", scratch.dir(), clock, &requests).await;
}

/// What the script's analysis costs, on the probe of 3,600 closed budgets, on the 1,200 monthly budgets of one account
/// per category and on the budget baseline's ledgers of 10,000 and 100,000 transactions: its queries and its time, and the time of the whole load with the script and
/// natively, the median of `ZHANG_BUDGET_COST_RUNS` loads (5) of each. `ZHANG_BUDGET_COST_LEDGERS` chooses the ledgers
/// (`probe,monthly,10k,100k`), and `ZHANG_BUDGET_COST_SCRIPT` a script to measure in place of the bundled one, such as an earlier
/// `budget.lua`. On demand, in release:
///
/// ```sh
/// cargo nextest run --release -p zhang-server --test golden --run-ignored only -E 'test(the_budget_script_costs)' --no-capture
/// ```
#[test]
#[ignore = "a measurement: run on demand, in release"]
fn the_budget_script_costs() {
    let runs = std::env::var("ZHANG_BUDGET_COST_RUNS").ok().and_then(|it| it.parse().ok()).unwrap_or(5usize);
    let only = std::env::var("ZHANG_BUDGET_COST_LEDGERS").unwrap_or_else(|_| "probe,monthly,10k,100k".to_owned());
    let script = std::env::var_os("ZHANG_BUDGET_COST_SCRIPT")
        .map(|path| Arc::<[u8]>::from(std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.to_string_lossy()))));
    let median = |mut list: Vec<Duration>| {
        list.sort();
        list[list.len() / 2]
    };
    for ledger in only.split(',') {
        let (scratch, clock) = match ledger {
            "probe" => (
                Scratch::zhang(&closed_budgets(3_600)),
                Clock::Fixed(chrono::DateTime::parse_from_rfc3339("2025-03-01T12:00:00Z").unwrap().to_utc()),
            ),
            "monthly" => (
                Scratch::zhang(&monthly_budgets(10, 120)),
                Clock::Fixed(chrono::DateTime::parse_from_rfc3339("2025-01-15T12:00:00Z").unwrap().to_utc()),
            ),
            "10k" => (Spec::new(10_000, 30).generate().scratch(), Spec::new(10_000, 30).clock()),
            "100k" => (Spec::new(100_000, 30).generate().scratch(), Spec::new(100_000, 30).clock()),
            other => panic!("no ledger {other}: probe, monthly, 10k or 100k"),
        };
        let (mut analyses, mut loads, mut natives, mut queries, mut failure) = (vec![], vec![], vec![], 0, None);
        for _ in 0..runs {
            let started = Instant::now();
            let native = Ledger::load_with_options(
                scratch.dir().to_path_buf(),
                MAIN.to_owned(),
                data_source_for(MAIN),
                clock,
                load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Native),
            );
            natives.push(started.elapsed());
            drop(native.expect("the ledger loads natively"));
            let started = Instant::now();
            let (cost, outcome) = load_with_the_script(scratch.dir(), clock, script.as_ref());
            loads.push(started.elapsed());
            analyses.extend(cost.took);
            queries = cost.queries;
            failure = outcome.err();
        }
        let analysis = if analyses.is_empty() {
            "-".to_owned()
        } else {
            format!("{:.1?}", median(analyses))
        };
        println!(
            "COST {ledger}: {queries} queries, analysis {analysis}, load with the script {:.1?}, native load {:.1?}, {runs} runs{}",
            median(loads),
            median(natives),
            failure
                .map(|it| format!(", FAILED: {}", it.chars().take(300).collect::<String>()))
                .unwrap_or_default()
        );
    }
}
