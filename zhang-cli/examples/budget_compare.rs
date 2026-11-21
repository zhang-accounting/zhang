//! Compare two sets of runs of the budget baseline's end-to-end timers (`examples/budget_baseline.rs`), the native
//! budget implementation against a candidate (the budget plugin), by the paired rule of the "Budget Baseline" page of
//! the developer docs (`docs/src/content/docs/developers/budget-baseline.md`, "Comparing a build with the baseline").
//!
//! ```sh
//! cargo run --release -p zhang --example budget_compare -- <native-dir> <candidate-dir> [--max-load 4]
//! ```
//!
//! Each directory holds the JSON reports of runs (what `ZHANG_BUDGET_BASELINE_OUT` writes, one directory per run:
//! `<dir>/01/100000tx-30b-60m.json`, `<dir>/02/...`). The reports are read in the order of their paths and paired by
//! that order, ledger by ledger: the first native run with the first candidate run, and so on, so run them in pairs,
//! alternating the builds. A pair with a run whose load average went above `--max-load` (4 by default) when it started
//! or ended is skipped, and the tool says which.
//!
//! For every timer it takes the difference of the two runs' p50 in every pair (candidate − native), and an
//! order-statistic confidence interval of the median difference: with `n` pairs sorted, `[d(k), d(n+1-k)]` covers the
//! median with probability `1 − 2·P(Binomial(n, ½) < k)`, whatever the distribution of the differences; `k` is the
//! largest with a coverage of at least 90% (at 5 to 7 pairs the lowest and the highest difference, 93.8% to 98.4%).
//! The allowance is max(5% of the median native p50, 50 ms). The timer **passes** when the interval's upper end is
//! below the allowance, **fails** when its lower end is above it, and is **inconclusive** otherwise: add pairs, up to
//! [`MOST_PAIRS`], past which it stays inconclusive. A verdict needs at least [`min_pairs`] pairs; the tool prints how
//! many more a timer needs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

/// the most pairs a timer gets before an undecided comparison is reported inconclusive
const MOST_PAIRS: usize = 15;
/// the coverage the interval of the median difference has at least
const COVERAGE: f64 = 0.90;
/// the timers, in the order the baseline lists them
const TIMERS: [&str; 6] = [
    "write_then_budgets",
    "write_then_errors",
    "reload_then_budgets",
    "reload_then_errors",
    "start_then_budgets",
    "start_then_errors",
];

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut dirs = vec![];
    let mut max_load = 4.0;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--max-load" => match args.next().and_then(|it| it.parse::<f64>().ok()) {
                Some(value) => max_load = value,
                None => return usage(),
            },
            _ => dirs.push(PathBuf::from(arg)),
        }
    }
    let [native, candidate] = dirs.as_slice() else {
        return usage();
    };
    let (native, candidate) = (reports(native), reports(candidate));
    if native.is_empty() || candidate.is_empty() {
        eprintln!("no report with end-to-end timers in one of the directories");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    for (ledger, natives) in &native {
        let Some(candidates) = candidate.get(ledger) else {
            println!("== {ledger}: no candidate run");
            continue;
        };
        failed |= compare(ledger, natives, candidates, max_load);
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: budget_compare <native-dir> <candidate-dir> [--max-load <1-minute load average>]");
    ExitCode::FAILURE
}

/// The reports with end-to-end timers under `dir`, by ledger, each ledger's in the order of their paths.
fn reports(dir: &Path) -> BTreeMap<String, Vec<(PathBuf, Value)>> {
    let mut paths = vec![];
    collect(dir, &mut paths);
    paths.sort();
    let mut by_ledger = BTreeMap::<String, Vec<(PathBuf, Value)>>::new();
    for path in paths {
        let Ok(report) = std::fs::read_to_string(&path)
            .map_err(|_| ())
            .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|_| ()))
        else {
            continue;
        };
        if let (Some(ledger), true) = (report["ledger"].as_str(), report["end_to_end"].is_object()) {
            by_ledger.entry(ledger.to_owned()).or_default().push((path, report));
        }
    }
    by_ledger
}

fn collect(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, paths);
        } else if path.extension().is_some_and(|it| it == "json") {
            paths.push(path);
        }
    }
}

/// Why a run is not to be compared: a load average above `max_load` when it started or ended, or none recorded. The
/// run's own verdict (`valid`, against `ZHANG_BUDGET_BASELINE_MAX_LOAD` when it ran) is not read: `--max-load` decides.
fn invalid(report: &Value, max_load: f64) -> Option<String> {
    let load = &report["load_average"];
    let one_minute = |key: &str| {
        load[key]
            .as_str()
            .and_then(|it| it.split_whitespace().next())
            .and_then(|it| it.parse::<f64>().ok())
    };
    let recorded = one_minute("before").into_iter().chain(one_minute("after")).collect::<Vec<_>>();
    let highest = recorded.iter().copied().fold(f64::MIN, f64::max);
    if recorded.is_empty() {
        Some("no load average recorded".to_owned())
    } else if highest > max_load {
        Some(format!("load average {highest} > {max_load}"))
    } else {
        None
    }
}

/// The coverage of the interval `[d(k), d(n+1-k)]` of `n` sorted differences for their median: `1 − 2·P(X < k)`, `X`
/// binomial with `n` trials of ½.
fn coverage(n: usize, k: usize) -> f64 {
    let mut below = 0.0;
    let mut choose = 1.0_f64;
    for i in 0..k {
        if i > 0 {
            choose = choose * (n - i + 1) as f64 / i as f64;
        }
        below += choose;
    }
    1.0 - 2.0 * below / 2f64.powi(n as i32)
}

/// the `k` of the narrowest order-statistic interval with a coverage of at least [`COVERAGE`]; `None` below 5 pairs
fn interval_rank(n: usize) -> Option<usize> {
    (1..=n / 2).take_while(|k| coverage(n, *k) >= COVERAGE).last().filter(|_| n >= 5)
}

/// The pairs a timer of `ledger` needs at least before a verdict: 10 where the noise of the 2026-11-21 baseline (the
/// spread of its runs' p50) was wider than the allowance, 5 elsewhere; 10 also when the native runs compared here spread
/// wider than the allowance.
fn min_pairs(ledger: &str, timer: &str, native_spread: f64, allowance: f64) -> usize {
    let wide = matches!(
        (ledger.starts_with("100000tx"), timer),
        (false, "reload_then_budgets" | "start_then_budgets") | (true, "write_then_budgets" | "reload_then_budgets")
    );
    if wide || native_spread > allowance {
        10
    } else {
        5
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Compare the runs of one ledger and print a line per timer; whether a timer failed.
fn compare(ledger: &str, natives: &[(PathBuf, Value)], candidates: &[(PathBuf, Value)], max_load: f64) -> bool {
    let mut pairs = vec![];
    let mut notes = vec![];
    for (index, (native, candidate)) in natives.iter().zip(candidates).enumerate() {
        let skip = [native, candidate]
            .iter()
            .filter_map(|(path, report)| invalid(report, max_load).map(|why| format!("{} ({why})", path.display())))
            .collect::<Vec<_>>();
        if skip.is_empty() {
            pairs.push((&native.1, &candidate.1));
        } else {
            notes.push(format!("pair {} skipped: {}", index + 1, skip.join(", ")));
        }
    }
    if natives.len() != candidates.len() {
        notes.push(format!(
            "{} native and {} candidate runs: the runs past the shorter list are not paired",
            natives.len(),
            candidates.len()
        ));
    }
    println!("== {ledger}: {} pairs", pairs.len());
    for note in &notes {
        println!("   {note}");
    }
    if pairs.is_empty() {
        println!("   nothing to compare");
        return false;
    }
    println!(
        "{:<20} {:>5} {:>11} {:>12} {:>26} {:>9}  verdict",
        "timer", "pairs", "native p50", "median diff", "interval of the median", "allowed"
    );
    let mut failed = false;
    for timer in TIMERS {
        let p50 = |report: &Value| report["end_to_end"][timer]["total"]["p50_ms"].as_f64();
        let measured = pairs
            .iter()
            .filter_map(|(native, candidate)| Some((p50(native)?, p50(candidate)?)))
            .collect::<Vec<_>>();
        if measured.is_empty() {
            continue;
        }
        let natives = measured.iter().map(|(native, _)| *native).collect::<Vec<_>>();
        let mut differences = measured.iter().map(|(native, candidate)| candidate - native).collect::<Vec<_>>();
        differences.sort_by(f64::total_cmp);
        let native_p50 = median(&natives);
        let allowance = (0.05 * native_p50).max(50.0);
        let spread = natives.iter().copied().fold(f64::MIN, f64::max) - natives.iter().copied().fold(f64::MAX, f64::min);
        let (n, needed) = (differences.len(), min_pairs(ledger, timer, spread, allowance));
        let interval = interval_rank(n).map(|k| (differences[k - 1], differences[n - k], coverage(n, k)));
        let shown = interval
            .map(|(low, high, coverage)| format!("[{low:+.1}, {high:+.1}] {:.1}%", 100.0 * coverage))
            .unwrap_or_else(|| "-".to_owned());
        let verdict = match interval {
            _ if n < needed => format!("INCONCLUSIVE: {} more pairs for a verdict (at least {needed})", needed - n),
            Some((_, high, _)) if high < allowance => "PASS".to_owned(),
            Some((low, _, _)) if low > allowance => {
                failed = true;
                "FAIL".to_owned()
            }
            _ if n < MOST_PAIRS => format!("INCONCLUSIVE: add pairs, up to {} more", MOST_PAIRS - n),
            _ => format!("INCONCLUSIVE after {n} pairs"),
        };
        println!(
            "{:<20} {:>5} {:>11.1} {:>+12.1} {:>26} {:>9.1}  {verdict}",
            timer,
            n,
            native_p50,
            median(&differences),
            shown,
            allowance
        );
    }
    failed
}
