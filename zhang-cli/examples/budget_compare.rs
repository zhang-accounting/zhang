//! Compare two sets of runs of the budget baseline (`examples/budget_baseline.rs`), a baseline build against a candidate
//! build, by the paired rule of the "Budget Baseline" page of the developer docs
//! (`docs/src/content/docs/developers/budget-baseline.md`, "Comparing a build with the baseline"): the end-to-end
//! timers and the memory of the runs with them, and the named-query p95 of the runs with warm requests.
//!
//! ```sh
//! cargo run --release -p zhang --example budget_compare -- <baseline-dir> <candidate-dir> \
//!     [--expect-analyzer zhang:budget] [--include-invalid] [--summary <file>] [--p95-rule difference|ratio] \
//!     [--p95-floor-ms 1]
//! ```
//!
//! Each directory holds the JSON reports of runs (what `ZHANG_BUDGET_BASELINE_OUT` writes, one directory per run:
//! `<dir>/01/100000tx-30b-60m.json`, `<dir>/02/...`; `budget_pairs` writes `<dir>/<mode>/<ledger>/<pair>/`). The reports
//! are read in the order of their paths and paired by that order, ledger by ledger and criterion by criterion: the first
//! native run with the first candidate run, and so on, so run them in pairs, alternating the builds or, as
//! `budget_pairs` does, the two sides of a pair at the same time. A report with end-to-end timers (the `e2e` mode, or the
//! full one) counts for the timers and the memory, a report with warm requests (the `requests` mode, or the full one)
//! for the named-query p95.
//!
//! The tool refuses a comparison it cannot trust: runs of one side that disagree with each other (their build, commit,
//! mode, analyzers, number of CPUs or worker threads, rounds of warm requests), sides that ran in another mode, on
//! another number of CPUs or worker threads, other rounds or another ledger, a candidate whose runs do not show the
//! analyzer `--expect-analyzer` names publishing, and a timer that a run measured with fewer than its 20 clean rounds.
//! It skips a pair with a run marked invalid (a build running, throttling or CPU pressure) and says why;
//! `--include-invalid` keeps them, for a diagnosis only.
//!
//! **The timers.** For every timer it takes the difference of the two runs' p50 in every pair (candidate − native), and
//! an order-statistic confidence interval of the median difference: with `n` pairs sorted, `[d(k), d(n+1-k)]` covers
//! the median with probability `1 − 2·P(Binomial(n, ½) < k)`, whatever the distribution of the differences; `k` is the
//! largest with a coverage of at least [`COVERAGE`], 95% (no interval reaches it with 5 pairs; with 6 to 8 it is from
//! the lowest to the highest difference). The allowance is max(5% of the median native p50, 50 ms). The timer
//! **passes** when the interval's upper end is below the allowance, **fails** when its lower end is above it, and is
//! **inconclusive** otherwise: add pairs, up to [`MOST_PAIRS`], past which it stays inconclusive. With this coverage a
//! false pass is at most 5% per timer, optional stopping included. A verdict needs at least [`min_pairs`] pairs; the
//! tool prints how many more a timer needs.
//!
//! **The memory**, on the same pairs: for each figure of [`MEMORY`], the median of the candidate's runs against the
//! median of the native runs may grow by at most max(10%, 40 MiB).
//!
//! **The named-query p95.** For every kind of warm request, the difference of the two runs' p95 in every pair
//! (candidate − native), and the same order-statistic interval of the median difference: it **passes** when the
//! interval's upper end is at most max(10% of the median native p95, `--p95-floor-ms`, 1 ms by default), **fails**
//! when its lower end is above it, and is inconclusive otherwise. `--p95-rule ratio` judges the ratio of the two p95s
//! instead, against [`P95_BAR`], 1.1. Either way the other figures are printed for information, with G1's reading, the
//! ratio of the medians of the runs' p95. It is judged on [`P95_PAIRS`] pairs, 60, run before it is judged.
//!
//! **To confirm.** When the two runs of the pairs ran at the same time (their reports' wall clocks overlap), a PASS
//! whose interval's upper end comes within a margin of what is allowed is reported `CONFIRM`: the two sides share the
//! memory bandwidth and the caches, which can narrow a difference, so it is to be confirmed with pairs run one after
//! the other (`budget_pairs` runs them). The margin is [`CONFIRM_MARGIN`], 30%, of the allowance, or twice the gap the
//! calibration measured between the two ways for the timer and ledger (`budget_calibration.json`), whichever is larger.
//!
//! **The calibration.** `budget_compare --gaps <concurrent-out> <sequential-out>` reads two `budget_pairs` outputs of the
//! same known heavier candidate, one with its pairs at the same time and one with `--sequential`, and prints the gaps
//! as `budget_calibration.json` holds them: for each ledger and timer, how far apart the two median differences are.
//!
//! The baseline is the side the layout and the messages call `native`: until RFC #698 P5 deleted zhang's native budget
//! code, P4 compared the budget plugin with it, and a baseline that showed the budget analyzer was refused. Both sides
//! now run the bundled script `zhang:budget`: `--expect-analyzer` checks that the candidate's runs show it publishing.
//!
//! `--summary <file>` writes every verdict as JSON too, for `budget_pairs`, which adds pairs while one is inconclusive
//! and confirms the `CONFIRM`s.
//! The tool exits with a failure when a timer, a memory figure or a p95 fails, or a comparison is refused.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};

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
/// the figures of a report's `memory_mib` the memory bar applies to (step 6 of the rule)
const MEMORY: [&str; 5] = ["rss_after_load", "peak_load", "peak_after_reloads", "peak_final", "rss_final"];
/// what the memory may grow by: max(10% of the native median, 40 MiB)
const MEMORY_SHARE: f64 = 0.10;
const MEMORY_FLOOR_MIB: f64 = 40.0;
/// the kinds of warm request, in the order the baseline lists them
const REQUESTS: [&str; 6] = ["month", "month_first", "detail", "interval", "bql_year", "errors"];
/// the most the candidate's p95 of a request may be, as a multiple of the native p95 (the ratio rule), or what it may
/// grow by as a share of the native p95 (the difference rule, with a floor)
const P95_BAR: f64 = 1.1;
/// the floor of what the difference rule allows a p95 to grow by, in milliseconds, unless `--p95-floor-ms` says
/// otherwise: the bar is max(+10%, +1 ms) (RFC #698, decision 16, revised 2026-11-23)
const P95_FLOOR_MS: f64 = 1.0;
/// A PASS from pairs whose two runs ran at the same time is to be confirmed with pairs run one after the other when
/// the upper end of its interval is within a margin of what is allowed: the two sides share the memory bandwidth and
/// the caches, so a candidate that moves more memory also slows the native side and narrows the difference. The
/// margin is this share of the allowance, or twice the gap the calibration measured for the timer and ledger
/// ([`CALIBRATION`]), whichever is larger.
const CONFIRM_MARGIN: f64 = 0.3;
/// The calibration of pairs run at the same time against pairs run one after the other: for each ledger and timer, how
/// far apart the two ways put the median difference of a known heavier candidate (`gaps_ms`). Refreshed with
/// `budget_compare --gaps <concurrent-out> <sequential-out>` ("Budget Baseline" page, "Calibration").
const CALIBRATION: &str = include_str!("budget_calibration.json");
/// The pairs the named-query p95 is judged on, all run before it is judged (no optional stopping). A `requests` run
/// costs a few seconds, and the p95 of a request of a fraction of a millisecond moves by 10 to 50% from process to
/// process, so the interval of the median ratio needs many pairs to be narrower than the bar.
const P95_PAIRS: usize = 60;

struct Options {
    expect_analyzer: Option<String>,
    include_invalid: bool,
    summary: Option<PathBuf>,
    p95_rule: P95Rule,
    /// the floor of what the difference rule allows the p95 to grow by, in milliseconds
    p95_floor_ms: f64,
}

/// How the named-query p95 is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P95Rule {
    /// the interval of the median of the pairs' differences (candidate − native) against max(10% of the median
    /// native p95, the floor): the default
    Difference,
    /// the interval of the median of the pairs' ratios (candidate / native) against 1.1
    Ratio,
}

/// What a run measured that a criterion reads: the end-to-end timers (and the memory of those runs), or the warm
/// requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    EndToEnd,
    Requests,
}

impl Section {
    /// the key of the report that holds it
    fn key(self) -> &'static str {
        match self {
            Section::EndToEnd => "end_to_end",
            Section::Requests => "requests",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::EndToEnd => "end-to-end timers and memory",
            Section::Requests => "named-query p95",
        }
    }
}

/// A verdict, as `--summary` writes it: the ledger, the criterion (`end_to_end`, `memory`, `named_query_p95`), the
/// section of the reports it read, the timer, figure or request, the pairs, the verdict (`PASS`, `FAIL`,
/// `INCONCLUSIVE`, `REFUSED`, or `CONFIRM`: a PASS to confirm with pairs run one after the other) and how many more
/// pairs an inconclusive one needs at least (0 when no pair would help).
struct Verdict {
    ledger: String,
    criterion: &'static str,
    section: Section,
    item: String,
    pairs: usize,
    verdict: &'static str,
    needs: usize,
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if let [flag, concurrent, sequential] = arguments.as_slice() {
        if flag == "--gaps" {
            return gaps(Path::new(concurrent), Path::new(sequential));
        }
    }
    let mut dirs = vec![];
    let mut options = Options {
        expect_analyzer: None,
        include_invalid: false,
        summary: None,
        p95_rule: P95Rule::Difference,
        p95_floor_ms: P95_FLOOR_MS,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expect-analyzer" => match args.next() {
                Some(name) => options.expect_analyzer = Some(name),
                None => return usage(),
            },
            "--summary" => match args.next() {
                Some(path) => options.summary = Some(PathBuf::from(path)),
                None => return usage(),
            },
            "--include-invalid" => options.include_invalid = true,
            "--p95-rule" => match args.next().as_deref() {
                Some("difference") => options.p95_rule = P95Rule::Difference,
                Some("ratio") => options.p95_rule = P95Rule::Ratio,
                _ => return usage(),
            },
            "--p95-floor-ms" => match args.next().and_then(|it| it.parse::<f64>().ok()).filter(|it| *it >= 0.0) {
                Some(floor) => options.p95_floor_ms = floor,
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(PathBuf::from(arg)),
        }
    }
    let [native, candidate] = dirs.as_slice() else {
        return usage();
    };
    let (native, candidate) = (reports(native), reports(candidate));
    if native.is_empty() || candidate.is_empty() {
        eprintln!("no report with end-to-end timers or warm requests in one of the directories");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    let mut verdicts = vec![];
    for (ledger, natives) in &native {
        let Some(candidates) = candidate.get(ledger) else {
            println!("== {ledger}: no candidate run");
            continue;
        };
        for section in [Section::EndToEnd, Section::Requests] {
            let of = |runs: &[Run]| runs.iter().filter(|(_, report)| report[section.key()].is_object()).cloned().collect::<Vec<_>>();
            let (natives, candidates) = (of(natives), of(candidates));
            match (natives.is_empty(), candidates.is_empty()) {
                (true, true) => {}
                (false, false) => failed |= compare(ledger, section, &natives, &candidates, &options, &mut verdicts),
                (native_missing, _) => println!(
                    "== {ledger}, {}: no {} run",
                    section.title(),
                    if native_missing { "native" } else { "candidate" }
                ),
            }
        }
    }
    if let Some(path) = &options.summary {
        let verdicts = verdicts
            .iter()
            .map(|it| {
                json!({
                    "ledger": it.ledger,
                    "criterion": it.criterion,
                    "section": it.section.key(),
                    "item": it.item,
                    "pairs": it.pairs,
                    "verdict": it.verdict,
                    "needs": it.needs,
                })
            })
            .collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&json!({ "failed": failed, "verdicts": verdicts })).expect("JSON");
        if let Err(error) = std::fs::write(path, text + "\n") {
            eprintln!("{}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: budget_compare <native-dir> <candidate-dir> [--expect-analyzer <provider>] [--include-invalid] [--summary <file>] \
         [--p95-rule difference|ratio] [--p95-floor-ms {P95_FLOOR_MS}]\n       budget_compare --gaps <concurrent-out> <sequential-out>"
    );
    ExitCode::FAILURE
}

type Run = (PathBuf, Value);

/// by ledger, the number of pairs and the median difference of each timer
type Medians = BTreeMap<String, (usize, BTreeMap<&'static str, f64>)>;

/// The median difference (candidate − native) of each timer's p50 over the valid pairs of `out` (a `budget_pairs`
/// output, `<out>/native` and `<out>/candidate`), by ledger, with the number of pairs; and the commit and burden of the
/// candidate's runs, and the day the first of them started.
fn medians(out: &Path) -> (Medians, Value) {
    let (natives, candidates) = (reports(&out.join("native")), reports(&out.join("candidate")));
    let mut medians = BTreeMap::new();
    let mut setup = Value::Null;
    for (ledger, natives) in &natives {
        let Some(candidates) = candidates.get(ledger) else { continue };
        let pairs = natives
            .iter()
            .zip(candidates)
            .filter(|((_, native), (_, candidate))| native["end_to_end"].is_object() && candidate["end_to_end"].is_object())
            .filter(|((_, native), (_, candidate))| invalid(native).is_none() && invalid(candidate).is_none())
            .collect::<Vec<_>>();
        if let Some((_, (_, candidate))) = pairs.first() {
            let day = candidate["validity"]["started_unix_ms"]
                .as_i64()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|it| it.date_naive().to_string());
            setup = json!({"commit": candidate["identity"]["commit"], "burden_mib": candidate["identity"]["burden_mib"], "measured": day});
        }
        let mut timers = BTreeMap::new();
        for timer in TIMERS {
            let p50 = |report: &Value| report["end_to_end"][timer]["total"]["p50_ms"].as_f64();
            let differences = pairs
                .iter()
                .filter_map(|((_, native), (_, candidate))| Some(p50(candidate)? - p50(native)?))
                .collect::<Vec<_>>();
            if !differences.is_empty() {
                timers.insert(timer, median(&differences));
            }
        }
        medians.insert(ledger.clone(), (pairs.len(), timers));
    }
    (medians, setup)
}

/// The calibration's gaps, as `budget_calibration.json` holds them: for each ledger and timer, how far apart the
/// median differences of the pairs at the same time (`concurrent`) and one after the other (`sequential`) are, in
/// milliseconds, of the same known heavier candidate.
fn gaps(concurrent: &Path, sequential: &Path) -> ExitCode {
    let ((at_once, setup), (one_by_one, _)) = (medians(concurrent), medians(sequential));
    let mut gaps = serde_json::Map::new();
    let mut pairs = serde_json::Map::new();
    for (ledger, (n, timers)) in &at_once {
        let Some((m, others)) = one_by_one.get(ledger) else { continue };
        let ledger_gaps = timers
            .iter()
            .filter_map(|(timer, at_once)| Some((timer.to_string(), json!(((others.get(timer)? - at_once).abs() * 10.0).round() / 10.0))))
            .collect::<serde_json::Map<_, _>>();
        gaps.insert(ledger.clone(), Value::Object(ledger_gaps));
        pairs.insert(ledger.clone(), json!({"concurrent": n, "sequential": m}));
    }
    if gaps.is_empty() {
        eprintln!("no ledger with valid end-to-end pairs both ways");
        return ExitCode::FAILURE;
    }
    let calibration = json!({
        "about": "the gap, in ms, between the median differences of a known heavier candidate measured with pairs at the same time and with pairs one after the other; budget_compare confirms a PASS within twice it of the allowance",
        "candidate": setup,
        "pairs": pairs,
        "gaps_ms": gaps,
    });
    println!("{}", serde_json::to_string_pretty(&calibration).expect("JSON"));
    ExitCode::SUCCESS
}

/// The reports with end-to-end timers or warm requests under `dir`, by ledger, each ledger's in the order of their
/// paths.
fn reports(dir: &Path) -> BTreeMap<String, Vec<Run>> {
    let mut paths = vec![];
    collect(dir, &mut paths);
    paths.sort();
    let mut by_ledger = BTreeMap::<String, Vec<Run>>::new();
    for path in paths {
        let Some(report) = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
            continue;
        };
        let measured = [Section::EndToEnd, Section::Requests].iter().any(|it| report[it.key()].is_object());
        if let (Some(ledger), true) = (report["ledger"].as_str(), measured) {
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

/// the number of CPUs of a `Cpus_allowed_list` such as `16-23` or `0,2,4-7`
fn cpu_count(list: &str) -> Option<usize> {
    list.split(',').map(str::trim).filter(|it| !it.is_empty()).try_fold(0, |count, part| {
        let (low, high) = part.split_once('-').unwrap_or((part, part));
        let (low, high) = (low.parse::<usize>().ok()?, high.parse::<usize>().ok()?);
        (high >= low).then(|| count + high - low + 1)
    })
}

/// What must be the same in every run of one side: the build, the commit, the mode, the plugin level, the analyzers
/// and what they published, the ledger, the number of CPUs (concurrent pairs run on disjoint sets of the same size),
/// the worker threads and the rounds of warm requests.
fn setup(report: &Value) -> Vec<(&'static str, String)> {
    let text = |value: &Value| match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let cpus = report["validity"]["cpus_allowed"]
        .as_str()
        .and_then(cpu_count)
        .map(|it| it.to_string())
        .unwrap_or_else(|| "-".to_owned());
    vec![
        ("build", text(&report["identity"]["build"]).chars().take(16).collect()),
        ("commit", text(&report["identity"]["commit"])),
        ("mode", text(&report["mode"])),
        ("plugin level", text(&report["identity"]["plugins"])),
        ("analyzers after the load", text(&report["analyzers"]["after_load"])),
        ("analyzers at the end", text(&report["analyzers"]["at_end"])),
        ("table providers", text(&report["analyzers"]["schema_providers"])),
        ("ledger", text(&report["sha256"]).chars().take(16).collect()),
        ("CPUs", cpus),
        ("worker threads", text(&report["validity"]["workers"])),
        ("warm rounds", text(&report["requests"]["month"]["n"])),
    ]
}

/// what the native and the candidate runs must share
const SHARED: [&str; 5] = ["ledger", "mode", "CPUs", "worker threads", "warm rounds"];

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

/// the order-statistic interval of the median of `sorted` values, with its coverage, if any reaches [`COVERAGE`]
fn interval(sorted: &[f64]) -> Option<(f64, f64, f64)> {
    let n = sorted.len();
    interval_rank(n).map(|k| (sorted[k - 1], sorted[n - k], coverage(n, k)))
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

/// What makes the runs of `ledger` incomparable: the runs of a side that disagree, sides in another mode, on another
/// number of CPUs or worker threads, other rounds or ledgers, and a candidate without the expected analyzer. Each
/// side's setup is printed.
fn refusals(natives: &[Run], candidates: &[Run], options: &Options) -> Vec<String> {
    let mut refusals = vec![];
    let mut setups = vec![];
    for (side, runs) in [("native", natives), ("candidate", candidates)] {
        let first = setup(&runs[0].1);
        for (path, report) in &runs[1..] {
            for ((what, expected), (_, found)) in first.iter().zip(setup(report)) {
                if *expected != found {
                    refusals.push(format!(
                        "the {side} runs disagree on the {what}: {expected} in {}, {found} in {}",
                        runs[0].0.display(),
                        path.display()
                    ));
                }
            }
        }
        println!(
            "   {side}: {} runs; {}",
            runs.len(),
            first.iter().map(|(what, value)| format!("{what} {value}")).collect::<Vec<_>>().join(", ")
        );
        setups.push(first);
    }
    for what in SHARED {
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

type Pair<'a> = (&'a Run, &'a Run);

/// Compare the runs of one ledger for a section and print a line per timer, memory figure or request; whether one
/// failed or the comparison was refused.
fn compare(ledger: &str, section: Section, natives: &[Run], candidates: &[Run], options: &Options, verdicts: &mut Vec<Verdict>) -> bool {
    println!("== {ledger}: {}", section.title());
    let refused = refusals(natives, candidates, options);
    if !refused.is_empty() {
        for reason in &refused {
            println!("   REFUSED: {reason}");
        }
        verdicts.push(Verdict {
            ledger: ledger.to_owned(),
            criterion: "comparison",
            section,
            item: refused.join("; "),
            pairs: 0,
            verdict: "REFUSED",
            needs: 0,
        });
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
            println!(
                "   pair {} kept although {} (--include-invalid: a diagnosis, not a verdict for G1 or P4)",
                index + 1,
                skip.join("; ")
            );
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
    let concurrent = pairs.iter().any(|((_, native), (_, candidate))| overlap(native, candidate));
    println!("   {} pairs{}", pairs.len(), apart(&pairs));
    println!("   {}", cpu_sets(&pairs));
    if pairs.is_empty() {
        println!("   nothing to compare");
        return false;
    }
    let mut verdict = |criterion: &'static str, item: &str, pairs: usize, verdict: &'static str, needs: usize| {
        verdicts.push(Verdict {
            ledger: ledger.to_owned(),
            criterion,
            section,
            item: item.to_owned(),
            pairs,
            verdict,
            needs,
        });
    };
    match section {
        Section::EndToEnd => timers(ledger, &pairs, concurrent, &mut verdict) | memory(&pairs, &mut verdict),
        Section::Requests => p95(&pairs, concurrent, options, &mut verdict),
    }
}

/// Whether the two runs of a pair ran at the same time, at least in part, by the wall clock their reports record (a
/// report without it is taken for a run one after the other).
fn overlap(native: &Value, candidate: &Value) -> bool {
    let at = |report: &Value, key: &str| report["validity"][key].as_u64();
    match (
        at(native, "started_unix_ms"),
        at(native, "ended_unix_ms"),
        at(candidate, "started_unix_ms"),
        at(candidate, "ended_unix_ms"),
    ) {
        (Some(native_start), Some(native_end), Some(candidate_start), Some(candidate_end)) => native_start < candidate_end && candidate_start < native_end,
        _ => false,
    }
}

/// The CPU sets each side's runs were pinned to, with how many pairs used each: `budget_pairs` gives each set of a
/// slot to either side in turn.
fn cpu_sets(pairs: &[Pair]) -> String {
    let mut sides = vec![];
    for (side, at) in [("native", 0), ("candidate", 1)] {
        let mut sets = BTreeMap::<String, usize>::new();
        for pair in pairs {
            let report = if at == 0 { &pair.0 .1 } else { &pair.1 .1 };
            let set = report["validity"]["cpus_allowed"].as_str().unwrap_or("-").to_owned();
            *sets.entry(set).or_default() += 1;
        }
        let sets = sets.iter().map(|(set, count)| format!("{set} x{count}")).collect::<Vec<_>>();
        sides.push(format!("{side} {}", sets.join(", ")));
    }
    format!("CPU sets: {}", sides.join("; "))
}

/// The gap the calibration measured for `timer` on `ledger`, in milliseconds, if any ([`CALIBRATION`]).
fn calibrated_gap(ledger: &str, timer: &str) -> Option<f64> {
    let calibration = serde_json::from_str::<Value>(CALIBRATION).expect("budget_calibration.json is JSON");
    calibration["gaps_ms"][ledger][timer].as_f64()
}

/// The margin below its allowance that a PASS of pairs run at the same time must keep not to be confirmed:
/// [`CONFIRM_MARGIN`] of the allowance, or twice the calibrated gap, whichever is larger.
fn confirm_margin(allowance: f64, gap: Option<f64>) -> f64 {
    (CONFIRM_MARGIN * allowance).max(2.0 * gap.unwrap_or(0.0))
}

/// A PASS of pairs run at the same time whose interval's upper end (`high`) is past `bar − margin` is to be confirmed
/// with pairs run one after the other: `CONFIRM`.
fn confirm(decided: (&'static str, usize, String), high: f64, bar: f64, margin: f64, concurrent: bool) -> (&'static str, usize, String) {
    match decided {
        ("PASS", needs, _) if concurrent && high > bar - margin => (
            "CONFIRM",
            needs,
            format!(
                "PASS, to CONFIRM with pairs one after the other (upper end within {:.2} of the bar, margin {:.2})",
                bar - high,
                margin
            ),
        ),
        other => other,
    }
}

/// How far apart the two runs of each pair started, when the reports say: `budget_pairs` starts them together.
fn apart(pairs: &[Pair]) -> String {
    let started = |report: &Value| report["validity"]["started_unix_ms"].as_f64();
    let gaps = pairs
        .iter()
        .filter_map(|((_, native), (_, candidate))| Some((started(native)? - started(candidate)?).abs() / 1000.0))
        .collect::<Vec<_>>();
    if gaps.len() < pairs.len() || gaps.is_empty() {
        return String::new();
    }
    let most = gaps.iter().copied().fold(0.0, f64::max);
    format!("; a pair's two runs started {:.1} s apart at the median, {most:.1} s at most", median(&gaps))
}

/// the verdict of an interval against a bar it must stay below (`at_most`: or at)
fn decide(interval: Option<(f64, f64, f64)>, bar: f64, at_most: bool, n: usize, needed: usize, most: usize) -> (&'static str, usize, String) {
    match interval {
        _ if n < needed => (
            "INCONCLUSIVE",
            needed - n,
            format!("INCONCLUSIVE: {} more pairs for a verdict (at least {needed})", needed - n),
        ),
        None => (
            "INCONCLUSIVE",
            1,
            format!("INCONCLUSIVE: add a pair ({n} pairs reach no {:.0}% interval)", 100.0 * COVERAGE),
        ),
        Some((_, high, _)) if high < bar || (at_most && high <= bar) => ("PASS", 0, "PASS".to_owned()),
        Some((low, _, _)) if low > bar => ("FAIL", 0, "FAIL".to_owned()),
        _ if n < most => ("INCONCLUSIVE", 1, format!("INCONCLUSIVE: add pairs, up to {} more", most - n)),
        _ => ("INCONCLUSIVE", 0, format!("INCONCLUSIVE after {n} pairs")),
    }
}

/// The end-to-end timers of the pairs, a line each; whether one failed or was refused.
fn timers(ledger: &str, pairs: &[Pair], concurrent: bool, verdict: &mut impl FnMut(&'static str, &str, usize, &'static str, usize)) -> bool {
    println!(
        "{:<20} {:>5} {:>10} {:>11} {:>25} {:>8} {:>11} {:>11}  verdict",
        "timer", "pairs", "native p50", "median diff", "interval of the median", "allowed", "excluded", "during"
    );
    let mut failed = false;
    for timer in TIMERS {
        let at = |report: &Value| report["end_to_end"][timer].clone();
        let timers = pairs
            .iter()
            .map(|((native_path, native), (candidate_path, candidate))| ((native_path, at(native)), (candidate_path, at(candidate))));
        let timers = timers
            .filter(|((_, native), (_, candidate))| native.is_object() && candidate.is_object())
            .collect::<Vec<_>>();
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
        let measured = timers
            .iter()
            .filter_map(|(native, candidate)| Some((p50(&native.1)?, p50(&candidate.1)?)))
            .collect::<Vec<_>>();
        let natives = measured.iter().map(|(native, _)| *native).collect::<Vec<_>>();
        let mut differences = measured.iter().map(|(native, candidate)| candidate - native).collect::<Vec<_>>();
        differences.sort_by(f64::total_cmp);
        let native_p50 = median(&natives);
        let allowance = (0.05 * native_p50).max(50.0);
        let spread = natives.iter().copied().fold(f64::MIN, f64::max) - natives.iter().copied().fold(f64::MAX, f64::min);
        let (n, needed) = (differences.len(), min_pairs(ledger, timer, spread, allowance));
        let interval = interval(&differences);
        let shown = interval
            .map(|(low, high, coverage)| format!("[{low:+.1}, {high:+.1}] {:.1}%", 100.0 * coverage))
            .unwrap_or_else(|| "-".to_owned());
        let (outcome, needs, text) = match short.is_empty() {
            true => {
                let high = interval.map(|(_, high, _)| high).unwrap_or(f64::MIN);
                let margin = confirm_margin(allowance, calibrated_gap(ledger, timer));
                confirm(decide(interval, allowance, false, n, needed, MOST_PAIRS), high, allowance, margin, concurrent)
            }
            false => ("REFUSED", 0, format!("REFUSED: fewer than 20 clean rounds ({})", short.join(", "))),
        };
        failed |= matches!(outcome, "FAIL" | "REFUSED");
        verdict("end_to_end", timer, n, outcome, needs);
        println!(
            "{:<20} {:>5} {:>10.1} {:>+11.1} {:>25} {:>8.1} {:>11} {:>11}  {text}",
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

/// The memory of the pairs (step 6 of the rule), a line per figure of [`MEMORY`]: the median of each side's runs, and
/// what the candidate's may add, max(10% of the native median, 40 MiB); whether a figure is over.
fn memory(pairs: &[Pair], verdict: &mut impl FnMut(&'static str, &str, usize, &'static str, usize)) -> bool {
    println!(
        "{:<20} {:>5} {:>10} {:>10} {:>18} {:>8}  verdict",
        "memory (MiB)", "pairs", "native", "candidate", "difference", "allowed"
    );
    let mut failed = false;
    for figure in MEMORY {
        let of = |report: &Value| report["memory_mib"][figure].as_f64();
        let values = pairs
            .iter()
            .filter_map(|((_, native), (_, candidate))| Some((of(native)?, of(candidate)?)))
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }
        let native = median(&values.iter().map(|it| it.0).collect::<Vec<_>>());
        let candidate = median(&values.iter().map(|it| it.1).collect::<Vec<_>>());
        let (difference, allowed) = (candidate - native, (MEMORY_SHARE * native).max(MEMORY_FLOOR_MIB));
        let outcome = if difference <= allowed { "PASS" } else { "FAIL" };
        failed |= outcome == "FAIL";
        verdict("memory", figure, values.len(), outcome, 0);
        let share = format!("{difference:+.1} ({:+.1}%)", 100.0 * difference / native.max(f64::MIN_POSITIVE));
        println!(
            "{figure:<20} {:>5} {native:>10.1} {candidate:>10.1} {share:>18} {allowed:>8.1}  {outcome}",
            values.len()
        );
    }
    failed
}

/// The named-query p95 of the pairs, a line per kind of warm request. By the difference rule (the default), the
/// difference of the two runs' p95 in each pair and the interval of its median against max(10% of the median native
/// p95, `--p95-floor-ms`); by the ratio rule, the ratio of the two runs' p95 and the interval of its median against
/// [`P95_BAR`]. The other figures are printed for information, with G1's reading, the ratio of the medians of the
/// runs' p95. Whether one failed.
fn p95(pairs: &[Pair], concurrent: bool, options: &Options, verdict: &mut impl FnMut(&'static str, &str, usize, &'static str, usize)) -> bool {
    match options.p95_rule {
        P95Rule::Difference => println!(
            "   rule: the median difference, allowed max({:.0}% of the native p95, {} ms); ratios for information",
            100.0 * (P95_BAR - 1.0),
            options.p95_floor_ms
        ),
        P95Rule::Ratio => println!("   rule: the median ratio, allowed {P95_BAR}; differences for information"),
    }
    println!(
        "{:<12} {:>5} {:>10} {:>10} {:>10} {:>12} {:>12} {:>27} {:>8}  verdict",
        "request", "pairs", "native p95", "cand. p95", "G1's ratio", "median ratio", "median diff", "interval of the median", "allowed"
    );
    let mut kinds = REQUESTS.map(str::to_owned).to_vec();
    if let Some(requests) = pairs[0].0 .1["requests"].as_object() {
        kinds.extend(requests.keys().filter(|it| !REQUESTS.contains(&it.as_str())).cloned());
    }
    let mut failed = false;
    for kind in &kinds {
        let of = |report: &Value| report["requests"][kind]["p95_ms"].as_f64();
        let values = pairs
            .iter()
            .filter_map(|((_, native), (_, candidate))| Some((of(native)?, of(candidate)?)))
            .filter(|(native, _)| *native > 0.0)
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }
        let native = median(&values.iter().map(|it| it.0).collect::<Vec<_>>());
        let candidate = median(&values.iter().map(|it| it.1).collect::<Vec<_>>());
        let mut ratios = values.iter().map(|(native, candidate)| candidate / native).collect::<Vec<_>>();
        ratios.sort_by(f64::total_cmp);
        let mut differences = values.iter().map(|(native, candidate)| candidate - native).collect::<Vec<_>>();
        differences.sort_by(f64::total_cmp);
        let n = ratios.len();
        let (judged, bar, allowed) = match options.p95_rule {
            P95Rule::Difference => {
                let allowed = ((P95_BAR - 1.0) * native).max(options.p95_floor_ms);
                (interval(&differences), allowed, format!("{allowed:.3}"))
            }
            P95Rule::Ratio => (interval(&ratios), P95_BAR, format!("x{P95_BAR}")),
        };
        let shown = judged
            .map(|(low, high, coverage)| match options.p95_rule {
                P95Rule::Difference => format!("[{low:+.3}, {high:+.3}] {:.1}%", 100.0 * coverage),
                P95Rule::Ratio => format!("[{low:.2}, {high:.2}] {:.1}%", 100.0 * coverage),
            })
            .unwrap_or_else(|| "-".to_owned());
        // the p95 has no calibrated gap: its margin is a share of what it may grow by
        let high = judged.map(|(_, high, _)| high).unwrap_or(f64::MIN);
        let margin = match options.p95_rule {
            P95Rule::Difference => confirm_margin(bar, None),
            P95Rule::Ratio => confirm_margin(bar - 1.0, None),
        };
        let (outcome, needs, text) = confirm(decide(judged, bar, true, n, P95_PAIRS, P95_PAIRS), high, bar, margin, concurrent);
        failed |= outcome == "FAIL";
        verdict("named_query_p95", kind, n, outcome, needs);
        println!(
            "{kind:<12} {n:>5} {native:>10.3} {candidate:>10.3} {:>10.2} {:>12.2} {:>+12.3} {shown:>27} {allowed:>8}  {text}",
            candidate / native,
            median(&ratios),
            median(&differences)
        );
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpus_are_counted_from_a_list() {
        assert_eq!(cpu_count("16-23"), Some(8));
        assert_eq!(cpu_count("0,2,4-7"), Some(6));
        assert_eq!(cpu_count("7-3"), None);
        assert_eq!(cpu_count("x"), None);
    }

    /// a report with the analyzers it shows after its first load and at its end, and what the setup reads
    fn report(analyzers: &[&str]) -> Value {
        json!({
            "mode": "e2e",
            "sha256": "ledger",
            "identity": {"build": "build", "commit": "commit"},
            "analyzers": {"after_load": analyzers, "at_end": analyzers},
            "validity": {"cpus_allowed": "16-23", "workers": 8},
        })
    }

    #[test]
    fn a_candidate_that_does_not_show_the_expected_analyzer_is_refused() {
        let options = Options {
            expect_analyzer: Some("zhang:budget".to_owned()),
            include_invalid: false,
            summary: None,
            p95_rule: P95Rule::Difference,
            p95_floor_ms: P95_FLOOR_MS,
        };
        let run = |path: &str, analyzers: &[&str]| vec![(PathBuf::from(path), report(analyzers))];
        let baseline = run("native/01.json", &["zhang:budget: ok"]);
        // a build of the script against another
        assert_eq!(
            refusals(&baseline, &run("candidate/01.json", &["zhang:budget: ok"]), &options),
            Vec::<String>::new()
        );
        // a candidate whose script did not publish
        let refused = refusals(&baseline, &run("candidate/01.json", &["zhang:budget: failed (timeout)"]), &options);
        assert!(
            refused
                .iter()
                .any(|it| it == "candidate/01.json does not show the analyzer zhang:budget publishing (after_load)"),
            "{refused:?}"
        );
        // without --expect-analyzer, anything goes
        let options = Options {
            expect_analyzer: None,
            ..options
        };
        assert_eq!(refusals(&baseline, &run("candidate/01.json", &[]), &options), Vec::<String>::new());
    }

    #[test]
    fn a_pass_near_its_bar_is_confirmed_within_the_calibrated_margin() {
        // 30% of the allowance, where no calibration measured a larger gap
        assert!((confirm_margin(50.0, None) - 15.0).abs() < 1e-9);
        assert!((confirm_margin(50.0, calibrated_gap("10000tx-30b-60m", "reload_then_errors")) - 15.0).abs() < 1e-9);
        // twice the calibrated gap, where that is larger
        let gap = calibrated_gap("100000tx-30b-60m", "reload_then_errors").expect("a calibrated gap");
        assert!((confirm_margin(66.0, Some(gap)) - 2.0 * gap).abs() < 1e-9 && 2.0 * gap > 0.3 * 66.0);
        let pass = || ("PASS", 0, "PASS".to_owned());
        assert_eq!(confirm(pass(), 40.0, 66.0, 32.6, true).0, "CONFIRM");
        assert_eq!(confirm(pass(), 30.0, 66.0, 32.6, true).0, "PASS");
        // pairs run one after the other need no confirmation
        assert_eq!(confirm(pass(), 40.0, 66.0, 32.6, false).0, "PASS");
    }

    #[test]
    fn the_interval_ranks_are_the_documented_ones() {
        let ranks = (5..=15).map(interval_rank).collect::<Vec<_>>();
        assert_eq!(
            ranks,
            [None, Some(1), Some(1), Some(1), Some(2), Some(2), Some(2), Some(3), Some(3), Some(3), Some(4)]
        );
    }
}
