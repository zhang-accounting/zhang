//! Compare two sets of runs of the budget baseline's end-to-end timers (`examples/budget_baseline.rs`), the native
//! budget implementation against a candidate (the budget plugin), by the paired rule of the "Budget Baseline" page of
//! the developer docs (`docs/src/content/docs/developers/budget-baseline.md`, "Comparing a build with the baseline").
//!
//! ```sh
//! cargo run --release -p zhang --example budget_compare -- <native-dir> <candidate-dir> \
//!     [--expect-analyzer zhang:budget] [--include-invalid]
//! ```
//!
//! Each directory holds the JSON reports of runs (what `ZHANG_BUDGET_BASELINE_OUT` writes, one directory per run:
//! `<dir>/01/100000tx-30b-60m.json`, `<dir>/02/...`). The reports are read in the order of their paths and paired by
//! that order, ledger by ledger: the first native run with the first candidate run, and so on, so run them in pairs,
//! alternating the builds.
//!
//! The tool refuses a comparison it cannot trust: runs of one side that disagree with each other (their build, commit,
//! analyzers, CPUs or worker threads), sides that ran the ledgers on other CPUs or another ledger, a candidate whose
//! runs do not show the analyzer `--expect-analyzer` names publishing, and a timer that a run measured with fewer than
//! its 20 clean rounds. It skips a pair with a run marked invalid (a build running, throttling or CPU pressure) and says
//! why; `--include-invalid` keeps them, for a diagnosis only.
//!
//! For every timer it takes the difference of the two runs' p50 in every pair (candidate − native), and an
//! order-statistic confidence interval of the median difference: with `n` pairs sorted, `[d(k), d(n+1-k)]` covers the
//! median with probability `1 − 2·P(Binomial(n, ½) < k)`, whatever the distribution of the differences; `k` is the
//! largest with a coverage of at least [`COVERAGE`], 95% (no interval reaches it with 5 pairs; with 6 to 8 it is from
//! the lowest to the highest difference). The allowance is max(5% of the median native p50, 50 ms). The timer
//! **passes** when the interval's upper end is below the allowance, **fails** when its lower end is above it, and is
//! **inconclusive** otherwise: add pairs, up to [`MOST_PAIRS`], past which it stays inconclusive. With this coverage a
//! false pass is at most 5% per timer, optional stopping included. A verdict needs at least [`min_pairs`] pairs; the
//! tool prints how many more a timer needs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

/// the most pairs a timer gets before an undecided comparison is reported inconclusive
const MOST_PAIRS: usize = 15;
/// the coverage the interval of the median difference has at least
const COVERAGE: f64 = 0.95;
/// the timers, in the order the baseline lists them
const TIMERS: [&str; 6] = [
    "write_then_budgets",
    "write_then_errors",
    "reload_then_budgets",
    "reload_then_errors",
    "start_then_budgets",
    "start_then_errors",
];

struct Options {
    expect_analyzer: Option<String>,
    include_invalid: bool,
}

fn main() -> ExitCode {
    let mut dirs = vec![];
    let mut options = Options {
        expect_analyzer: None,
        include_invalid: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expect-analyzer" => match args.next() {
                Some(name) => options.expect_analyzer = Some(name),
                None => return usage(),
            },
            "--include-invalid" => options.include_invalid = true,
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
        failed |= compare(ledger, natives, candidates, &options);
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: budget_compare <native-dir> <candidate-dir> [--expect-analyzer <provider>] [--include-invalid]");
    ExitCode::FAILURE
}

type Run = (PathBuf, Value);

/// The reports with end-to-end timers under `dir`, by ledger, each ledger's in the order of their paths.
fn reports(dir: &Path) -> BTreeMap<String, Vec<Run>> {
    let mut paths = vec![];
    collect(dir, &mut paths);
    paths.sort();
    let mut by_ledger = BTreeMap::<String, Vec<Run>>::new();
    for path in paths {
        let Some(report) = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
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

/// what must be the same in every run of one side: the build, the commit, the analyzers and what they published,
/// the ledger, the CPUs and the worker threads
fn setup(report: &Value) -> Vec<(&'static str, String)> {
    let text = |value: &Value| match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    vec![
        ("build", text(&report["identity"]["build"]).chars().take(16).collect()),
        ("commit", text(&report["identity"]["commit"])),
        ("analyzers after the load", text(&report["analyzers"]["after_load"])),
        ("analyzers at the end", text(&report["analyzers"]["at_end"])),
        ("table providers", text(&report["analyzers"]["schema_providers"])),
        ("ledger", text(&report["sha256"]).chars().take(16).collect()),
        ("CPUs", text(&report["validity"]["cpus_allowed"])),
        ("worker threads", text(&report["validity"]["workers"])),
    ]
}

/// why a run is not to be compared: the reasons its run marked it invalid with
fn invalid(report: &Value) -> Option<String> {
    match (&report["valid"], &report["validity"]["reasons"]) {
        (Value::Bool(true), _) => None,
        (_, Value::Array(reasons)) if !reasons.is_empty() => Some(reasons.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")),
        _ => Some("no validity recorded".to_owned()),
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

/// the `k` of the narrowest order-statistic interval with a coverage of at least [`COVERAGE`], if any
fn interval_rank(n: usize) -> Option<usize> {
    (1..=n / 2).take_while(|k| coverage(n, *k) >= COVERAGE).last()
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

/// What makes the runs of `ledger` incomparable: the runs of a side that disagree, sides on other CPUs, worker threads
/// or ledgers, and a candidate without the expected analyzer. Each side's setup is printed.
fn refusals(natives: &[Run], candidates: &[Run], options: &Options) -> Vec<String> {
    let mut refusals = vec![];
    let mut setups = vec![];
    for (side, runs) in [("native", natives), ("candidate", candidates)] {
        let first = setup(&runs[0].1);
        for (path, report) in &runs[1..] {
            for ((what, expected), (_, found)) in first.iter().zip(setup(report)) {
                if *expected != found {
                    refusals.push(format!("the {side} runs disagree on the {what}: {expected} in {}, {found} in {}", runs[0].0.display(), path.display()));
                }
            }
        }
        println!("   {side}: {} runs; {}", runs.len(), first.iter().map(|(what, value)| format!("{what} {value}")).collect::<Vec<_>>().join(", "));
        setups.push(first);
    }
    for what in ["ledger", "CPUs", "worker threads"] {
        let value = |setup: &Vec<(&str, String)>| setup.iter().find(|(it, _)| *it == what).map(|(_, value)| value.clone());
        if value(&setups[0]) != value(&setups[1]) {
            refusals.push(format!("the native and the candidate runs differ in the {what}"));
        }
    }
    if let Some(expected) = &options.expect_analyzer {
        let published = format!("{expected}: ok");
        for (path, report) in candidates {
            for when in ["after_load", "at_end"] {
                let shows = report["analyzers"][when]
                    .as_array()
                    .is_some_and(|analyzers| analyzers.iter().any(|it| it.as_str() == Some(published.as_str())));
                if !shows {
                    refusals.push(format!("{} does not show the analyzer {expected} publishing ({when})", path.display()));
                }
            }
        }
    }
    refusals
}

/// Compare the runs of one ledger and print a line per timer; whether a timer failed or the comparison was refused.
fn compare(ledger: &str, natives: &[Run], candidates: &[Run], options: &Options) -> bool {
    println!("== {ledger}");
    let refused = refusals(natives, candidates, options);
    if !refused.is_empty() {
        for reason in &refused {
            println!("   REFUSED: {reason}");
        }
        return true;
    }
    let mut pairs = vec![];
    for (index, (native, candidate)) in natives.iter().zip(candidates).enumerate() {
        let skip = [native, candidate]
            .iter()
            .filter_map(|(path, report)| invalid(report).map(|why| format!("{} is invalid: {why}", path.display())))
            .collect::<Vec<_>>();
        if skip.is_empty() {
            pairs.push((native, candidate));
        } else if options.include_invalid {
            println!("   pair {} kept although {} (--include-invalid: a diagnosis, not a verdict for G1)", index + 1, skip.join("; "));
            pairs.push((native, candidate));
        } else {
            println!("   pair {} skipped: {}", index + 1, skip.join("; "));
        }
    }
    if natives.len() != candidates.len() {
        println!(
            "   {} native and {} candidate runs: the runs past the shorter list are not paired",
            natives.len(),
            candidates.len()
        );
    }
    println!("   {} pairs", pairs.len());
    if pairs.is_empty() {
        println!("   nothing to compare");
        return false;
    }
    println!(
        "{:<20} {:>5} {:>10} {:>11} {:>25} {:>8} {:>11} {:>11}  verdict",
        "timer", "pairs", "native p50", "median diff", "interval of the median", "allowed", "excluded", "during"
    );
    let mut failed = false;
    for timer in TIMERS {
        let at = |report: &Value| report["end_to_end"][timer].clone();
        let timers = pairs.iter().map(|((native_path, native), (candidate_path, candidate))| ((native_path, at(native)), (candidate_path, at(candidate))));
        let timers = timers.filter(|((_, native), (_, candidate))| native.is_object() && candidate.is_object()).collect::<Vec<_>>();
        if timers.is_empty() {
            continue;
        }
        // a timer measured with fewer clean rounds than it needs is not compared
        let short = timers
            .iter()
            .flat_map(|(native, candidate)| [native, candidate])
            .filter(|(_, timer)| timer["valid"] == false)
            .map(|(path, timer)| format!("{} has {} clean rounds", path.display(), timer["clean_rounds"]))
            .collect::<Vec<_>>();
        let count = |side: usize, key: &str| -> u64 {
            timers
                .iter()
                .map(|pair| if side == 0 { &pair.0 .1 } else { &pair.1 .1 })
                .filter_map(|timer| timer[key].as_u64())
                .sum()
        };
        let excluded = format!("{}/{}", count(0, "watcher_before_excluded"), count(1, "watcher_before_excluded"));
        let during = format!("{}/{}", count(0, "watcher_during_kept"), count(1, "watcher_during_kept"));
        let p50 = |timer: &Value| timer["total"]["p50_ms"].as_f64();
        let measured = timers.iter().filter_map(|(native, candidate)| Some((p50(&native.1)?, p50(&candidate.1)?))).collect::<Vec<_>>();
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
            _ if !short.is_empty() => {
                failed = true;
                format!("REFUSED: fewer than 20 clean rounds ({})", short.join(", "))
            }
            _ if n < needed => format!("INCONCLUSIVE: {} more pairs for a verdict (at least {needed})", needed - n),
            None => format!("INCONCLUSIVE: add a pair ({n} pairs reach no {:.0}% interval)", 100.0 * COVERAGE),
            Some((_, high, _)) if high < allowance => "PASS".to_owned(),
            Some((low, _, _)) if low > allowance => {
                failed = true;
                "FAIL".to_owned()
            }
            _ if n < MOST_PAIRS => format!("INCONCLUSIVE: add pairs, up to {} more", MOST_PAIRS - n),
            _ => format!("INCONCLUSIVE after {n} pairs"),
        };
        println!(
            "{:<20} {:>5} {:>10.1} {:>+11.1} {:>25} {:>8.1} {:>11} {:>11}  {verdict}",
            timer,
            n,
            native_p50,
            median(&differences),
            shown,
            allowance,
            excluded,
            during
        );
    }
    failed
}
