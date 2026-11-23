//! RFC #698's `both` mode on generated ledgers: each ledger's budgets computed by zhang's native code and by the bundled
//! script `zhang:budget` (`plugins/budget/budget.lua`), and what a client reads of them compared
//! ([`zhang_testkit::budget_parity`]): the errors id-exact, the budget tables, the `budgets` of `#postings`, every
//! budget as of every month, and budget pages. A difference the testing guide allows (the scale of a decimal) is listed
//! as allowed; any other fails the run.
//!
//! ```sh
//! cargo run --release -p zhang --example budget_parity -- bench           # the budget baseline's two ledgers
//! cargo run --release -p zhang --example budget_parity -- seeds 1 40      # seeded ledgers with budget errors
//! cargo run --release -p zhang --example budget_parity -- profile 5 [10000]  # loads and first pages, timed, 5 rounds
//! ```
//!
//! `seeds` generates small ledgers ([`zhang_testkit::budget_ledger`], 600 transactions, 12 budgets, 18 months) from
//! each seed and breaks each in a few ways the seed picks, so that every budget error occurs: a definition removed or
//! moved later, a duplicate definition, a budget closed at a time, prices removed. `profile` is not the G1 measurement
//! (that is `budget_baseline` in pairs): it shows where an analysis spends its time, with the host's log lines.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::json;
use zhang_server::analysis::BudgetImpl;
use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::budget_parity::{answer, compare, requests, serve, Report};
use zhang_testkit::XorShift;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    let number = |at: usize, default: u64| args.get(at).and_then(|it| it.parse().ok()).unwrap_or(default);
    let ok = match args.first().map(String::as_str) {
        Some("bench") => runtime.block_on(bench()),
        Some("seeds") => runtime.block_on(seeds(number(1, 1), number(2, 20))),
        Some("memory") => runtime.block_on(memory(args.get(1).map(String::as_str).unwrap_or("native"), number(2, 10_000) as usize)),
        Some("queries") => runtime.block_on(queries(number(1, 100_000) as usize)),
        Some("profile") => {
            // BUDGET_PARITY_LOG=all logs everything at the info level: a script's `print` too, which extism logs
            let all = std::env::var("BUDGET_PARITY_LOG").is_ok_and(|it| it == "all");
            env_logger::Builder::new()
                .filter_level(if all { log::LevelFilter::Info } else { log::LevelFilter::Off })
                .filter_module(
                    "zhang_core::plugin",
                    if std::env::var_os("BUDGET_PARITY_QUERIES").is_some() {
                        log::LevelFilter::Debug
                    } else {
                        log::LevelFilter::Info
                    },
                )
                .filter_module(
                    "zhang_server::analysis",
                    if std::env::var_os("BUDGET_PARITY_QUERIES").is_some() {
                        log::LevelFilter::Debug
                    } else {
                        log::LevelFilter::Info
                    },
                )
                .format_timestamp_millis()
                .init();
            runtime.block_on(profile(number(1, 5) as usize, args.get(2).and_then(|it| it.parse().ok())))
        }
        _ => {
            eprintln!("usage: budget_parity bench | seeds <first> <last> | profile <rounds> [<transactions>]");
            false
        }
    };
    // the runtime stays: the servers' tasks may still hold the ledgers
    std::mem::forget(runtime);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The first day of every month from `first` (`YYYY-MM`) for `months` months.
fn months(first_year: i32, months: u32) -> Vec<String> {
    (0..months)
        .map(|index| format!("{:04}-{:02}-01", first_year + (index / 12) as i32, index % 12 + 1))
        .collect()
}

/// Print `report` of the ledger `name`; whether it has no regression.
fn print(name: &str, report: &Report, took: Duration) -> bool {
    let allowed = report.differences.iter().filter(|it| it.allowed.is_some()).count();
    let regressions = report.regressions().count();
    println!(
        "{name}: {} errors, {} requests compared in {took:.1?}: {} differences, {allowed} allowed, {regressions} not",
        report.errors,
        report.requests,
        report.differences.len()
    );
    for difference in report.differences.iter().take(40) {
        println!("  {difference}");
    }
    if report.differences.len() > 40 {
        println!("  … and {} more", report.differences.len() - 40);
    }
    regressions == 0
}

/// The two ledgers with budgets of the budget baseline.
async fn bench() -> bool {
    let mut ok = true;
    for spec in [Spec::new(10_000, 30), Spec::new(100_000, 30)] {
        let generated = spec.generate();
        let scratch = generated.scratch();
        drop(generated);
        let budgets = ["Groceries", "Household", "Japan"].map(str::to_owned);
        let pages = ["2021-01-01", "2023-06-01", "2025-12-01"].map(str::to_owned);
        let requests = requests(&months(2021, spec.months), &budgets, &pages);
        let started = Instant::now();
        let report = compare(scratch.dir(), MAIN, spec.clock(), &requests).await;
        ok &= print(&spec.name(), &report, started.elapsed());
    }
    ok
}

/// The ledger of `seed`, broken in the ways the seed picks: its files, and what was done.
fn broken(seed: u64) -> (Vec<(String, String)>, Vec<String>) {
    let spec = Spec::new(600, 12).with_months(18).with_seed(seed);
    let mut files = spec.generate().files().to_vec();
    let mut random = XorShift(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut done = vec![];
    // the definitions, as (file, line) of their first line
    let definitions = |files: &[(String, String)]| {
        files
            .iter()
            .enumerate()
            .flat_map(|(at, (_, text))| {
                text.lines()
                    .enumerate()
                    .filter(|(_, line)| line.split_whitespace().nth(1) == Some("budget"))
                    .map(move |(line, text)| (at, line, text.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    // a definition removed, with its metadata: the budget is never defined, or defined later
    let remove = |files: &mut Vec<(String, String)>, at: usize, line: usize| {
        let text = &files[at].1;
        let lines = text.lines().collect::<Vec<_>>();
        let end = (line + 1..lines.len()).find(|it| !lines[*it].starts_with("  ")).unwrap_or(lines.len());
        let kept = lines[..line].iter().chain(&lines[end..]).copied().collect::<Vec<_>>();
        files[at].1 = kept.join("\n") + "\n";
    };
    let file = |files: &[(String, String)], name: &str| files.iter().position(|(path, _)| path == name).expect("a file of the ledger");
    let (y2021, y2022) = (file(&files, "2021.zhang"), file(&files, "2022.zhang"));
    if random.chance(50) {
        let all = definitions(&files);
        let (at, line, text) = random.pick(&all).clone();
        remove(&mut files, at, line);
        if random.chance(50) {
            // defined again later: what came before is a budget that does not exist yet
            let name = text.split_whitespace().nth(2).unwrap_or_default().to_owned();
            let commodity = text.split_whitespace().nth(3).unwrap_or_default().to_owned();
            files[y2021].1.push_str(&format!("2021-07-15 budget {name} {commodity}\n"));
            done.push(format!("{name} defined on 2021-07-15 only"));
        } else {
            done.push(format!("the definition `{text}` removed"));
        }
    }
    if random.chance(50) {
        let all = definitions(&files);
        let (_, _, text) = random.pick(&all).clone();
        let name = text.split_whitespace().nth(2).unwrap_or_default().to_owned();
        files[y2022].1.push_str(&format!("2022-03-03 budget {name} GBP\n"));
        done.push(format!("{name} defined again in GBP"));
    }
    if random.chance(60) {
        let all = definitions(&files);
        let (_, _, text) = random.pick(&all).clone();
        let name = text.split_whitespace().nth(2).unwrap_or_default().to_owned();
        let time = format!("{:02}:{:02}:00", random.below(24), random.below(60));
        files[y2021].1.push_str(&format!("2021-10-12 {time} budget-close {name}\n"));
        done.push(format!("{name} closed on 2021-10-12 at {time}"));
    }
    if random.chance(60) {
        // the prices of a commodity in the first months: what is in it then converts to nothing
        let commodity = *random.pick(&["EUR", "GBP", "JPY", "GOLD"]);
        let pattern = format!(" price {commodity} ");
        let text = &files[y2021].1;
        let kept = text
            .lines()
            .filter(|line| !(line.contains(&pattern) && line.as_bytes().get(5..7).is_some_and(|month| month < b"05".as_slice())))
            .collect::<Vec<_>>();
        files[y2021].1 = kept.join("\n") + "\n";
        done.push(format!("the prices of {commodity} before May 2021 removed"));
    }
    (files, done)
}

/// Seeded ledgers, broken: every seed from `first` to `last`.
async fn seeds(first: u64, last: u64) -> bool {
    let mut ok = true;
    let mut errors = 0;
    for seed in first..=last {
        let (files, done) = broken(seed);
        let paths = files.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect::<Vec<_>>();
        let scratch = zhang_testkit::ledger::Scratch::with_files(MAIN, &paths);
        let spec = Spec::new(600, 12).with_months(18).with_seed(seed);
        let names = files
            .iter()
            .flat_map(|(_, text)| text.lines())
            .filter(|line| line.split_whitespace().nth(1) == Some("budget"))
            .filter_map(|line| line.split_whitespace().nth(2).map(str::to_owned))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let months = months(2021, spec.months);
        let pages = [months[0].clone(), months[9].clone(), months[months.len() - 1].clone()];
        let requests = requests(&months, &names, &pages);
        let started = Instant::now();
        let report = compare(scratch.dir(), MAIN, spec.clock(), &requests).await;
        errors += report.errors;
        ok &= print(&format!("seed {seed} ({})", done.join("; ")), &report, started.elapsed());
    }
    println!("{} ledgers, {errors} errors in all", last + 1 - first);
    ok
}

/// The durations of a timer.
type Samples = Vec<Duration>;

/// The median of `samples`.
fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Each bench ledger loaded `rounds` times natively and with the plugin, alternating, then its first budget page and
/// its errors page: the medians.
async fn profile(rounds: usize, transactions: Option<usize>) -> bool {
    let specs = [Spec::new(10_000, 30), Spec::new(100_000, 30)];
    for spec in IntoIterator::into_iter(specs).filter(|spec| transactions.is_none_or(|it| it == spec.transactions)) {
        let generated = spec.generate();
        let scratch = generated.scratch();
        drop(generated);
        let page = json!({"builtin": "budgets.month", "params": {"month": "2025-12-01"}});
        let errors = json!({"builtin": "journals.errors", "params": {"size": 100, "offset": 0}});
        // per implementation: the loads, the budget pages and the errors pages after them
        let mut times: Vec<(BudgetImpl, Samples, Samples, Samples)> = [BudgetImpl::Native, BudgetImpl::Plugin].map(|it| (it, vec![], vec![], vec![])).into();
        for round in 0..rounds + 1 {
            for (budgets, load, budget_page, errors_page) in &mut times {
                let started = Instant::now();
                let (router, _ledger) = serve(scratch.dir(), MAIN, spec.clock(), *budgets).await.expect("the ledger loads");
                let loaded = started.elapsed();
                let started = Instant::now();
                let (status, _) = answer(&router, &page).await;
                assert_eq!(status, 200);
                let paged = started.elapsed();
                let started = Instant::now();
                let (status, _) = answer(&router, &errors).await;
                assert_eq!(status, 200);
                // the first round warms up
                if round > 0 {
                    load.push(loaded);
                    budget_page.push(paged);
                    errors_page.push(started.elapsed());
                }
            }
        }
        for (budgets, load, budget_page, errors_page) in &mut times {
            let ms = |it: Duration| it.as_secs_f64() * 1000.0;
            println!(
                "{} {}: load {:.1} ms, then budgets.month {:.1} ms, then journals.errors {:.2} ms (medians of {rounds})",
                spec.name(),
                budgets.name(),
                ms(median(load)),
                ms(median(budget_page)),
                ms(median(errors_page))
            );
        }
    }
    true
}

/// The budget analysis's queries, and variants of them, timed on a bench ledger loaded natively: where a query spends
/// its time, for the profile of G1.
async fn queries(transactions: usize) -> bool {
    let spec = Spec::new(transactions, 30);
    let generated = spec.generate();
    let scratch = generated.scratch();
    drop(generated);
    let (router, _ledger) = serve(scratch.dir(), MAIN, spec.clock(), BudgetImpl::Native).await.expect("the ledger loads");
    let member = "account IN ('Expenses:Food:Groceries', 'Expenses:Food:Market') AND open_meta_has(account, 'budget', 'Groceries', date, time)";
    let variants = [
        ("scan only", "SELECT count(*) WHERE account IN ('Expenses:Food:Groceries', 'Expenses:Food:Market')".to_owned()),
        ("member", format!("SELECT count(*) WHERE {member}")),
        ("member, month", format!("SELECT yearmonth(date) AS month, count(*) WHERE {member} GROUP BY month")),
        ("+ kind", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, count(*) WHERE {member} GROUP BY kind, month")),
        ("+ sum(number)", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(number) WHERE {member} GROUP BY kind, month")),
        ("+ possign", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(number, account)) WHERE {member} GROUP BY kind, month")),
        ("+ convert", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(number(convert(position, 'USD', date)), account)) WHERE {member} GROUP BY kind, month")),
        ("+ convert foreign only", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(CASE WHEN currency = 'USD' THEN number ELSE number(convert(position, 'USD', date)) END, account)) WHERE {member} GROUP BY kind, month")),
        ("+ convert to EUR", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(number(convert(position, 'EUR', date)), account)) WHERE {member} GROUP BY kind, month")),
        ("+ convert to JPY", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(number(convert(position, 'JPY', date)), account)) WHERE {member} GROUP BY kind, month")),
        ("inventory", format!("SELECT seq < 5 AS kind, yearmonth(date) AS month, sum(possign(convert(position, 'USD', date), account)) WHERE {member} GROUP BY kind, month")),
        ("held pairs", "SELECT account, currency, cost_currency, min(date) AS since WHERE account IN ('Expenses:Food:Groceries', 'Expenses:Food:Market') GROUP BY account, currency, cost_currency".to_owned()),
        ("last transaction", "SELECT seq, date FROM #transactions ORDER BY date DESC, seq DESC LIMIT 1".to_owned()),
        ("last transaction, last()", "SELECT last(seq), last(date) FROM #transactions".to_owned()),
        ("last via #entries", "SELECT last(seq), last(date) FROM #entries WHERE type = 'transaction'".to_owned()),
        ("opens", "SELECT accounts, meta_values('budget') AS budgets FROM #entries WHERE type = 'open'".to_owned()),
    ];
    // the first query builds the cache
    let _ = answer(&router, &json!({"bql": "SELECT count(*)"})).await;
    for (name, bql) in variants {
        let mut times = vec![];
        for _ in 0..7 {
            let started = Instant::now();
            let (status, body) = answer(&router, &json!({ "bql": bql })).await;
            assert_eq!(status, 200, "{name}: {body}");
            times.push(started.elapsed());
        }
        println!("{:<28} {:>10.2?}", name, median(&mut times));
    }
    true
}

/// A field of `/proc/self/status` in MiB: `VmRSS`, `VmHWM`.
fn status_mib(field: &str) -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or_default();
    kib / 1024.0
}

/// The resident memory of this process around one load of a bench ledger with `budgets` (`native`, `plugin`), its
/// first budget page and a reload, each in a process of its own.
async fn memory(budgets: &str, transactions: usize) -> bool {
    let budgets = match budgets {
        "plugin" => BudgetImpl::Plugin,
        _ => BudgetImpl::Native,
    };
    let spec = Spec::new(transactions, 30);
    let generated = spec.generate();
    let scratch = generated.scratch();
    drop(generated);
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let before = status_mib("VmRSS");
    let (router, ledger) = serve(scratch.dir(), MAIN, spec.clock(), budgets).await.expect("the ledger loads");
    let (loaded, peak_load) = (status_mib("VmRSS"), status_mib("VmHWM"));
    let (status, _) = answer(&router, &json!({"builtin": "budgets.month", "params": {"month": "2025-12-01"}})).await;
    assert_eq!(status, 200);
    let (paged, peak_page) = (status_mib("VmRSS"), status_mib("VmHWM"));
    let reloader = ledger.read().await.reloader();
    let again = tokio::task::spawn_blocking(reloader)
        .await
        .expect("the reload runs")
        .expect("the ledger reloads");
    *ledger.write().await = again;
    let (status, _) = answer(&router, &json!({"builtin": "budgets.month", "params": {"month": "2025-12-01"}})).await;
    assert_eq!(status, 200);
    println!(
        "{} {}: RSS before {before:.0} MiB, after the load {loaded:.0} (peak {peak_load:.0}), after the page {paged:.0} (peak {peak_page:.0}), after a reload and a page {:.0} (peak {:.0})",
        spec.name(),
        budgets.name(),
        status_mib("VmRSS"),
        status_mib("VmHWM")
    );
    true
}
