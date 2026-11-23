//! RFC #698's differential fuzzing of budgets: seeded generated ledgers ([`zhang_testkit::budget_fuzz`]) that use every
//! budget directive and break the budgets in every way the native code reports, each loaded natively and with the
//! bundled script `zhang:budget`, and compared as a client reads them: the errors id-exact, the three budget tables,
//! the `budgets` of the postings and the budget pages' named queries.
//!
//! A difference fails the test with the seed, the first difference, and the smallest ledger found that still shows one,
//! also written to `<target>/tmp/budget-fuzz/seed-<seed>/`. Every test run checks the seeds [`DEFAULT_SEEDS`]; the long
//! run, [`LONG_SEEDS`], is an ignored test that runs every night (`.github/workflows/budget-fuzz.yml`) and on demand
//! through the cargo alias `budget-fuzz` (`.cargo/config.toml`), in release for speed; another range runs on demand:
//!
//! ```sh
//! cargo budget-fuzz                                                                                  # seeds 1 to 10,000
//! ZHANG_BUDGET_FUZZ_SEEDS=1..=2000 cargo nextest run --release -p zhang-server --test golden -E 'test(budget_fuzz)'
//! ZHANG_BUDGET_FUZZ_SEEDS=17 cargo nextest run -p zhang-server --test golden -E 'test(budget_fuzz)'   # one seed
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use zhang_testkit::budget_fuzz::{check, seeds, FuzzLedger, FEATURES};

/// The seeds of every test run.
const DEFAULT_SEEDS: RangeInclusive<u64> = 1..=40;
/// The seeds of the long run: every night, and `cargo budget-fuzz`.
const LONG_SEEDS: RangeInclusive<u64> = 1..=10_000;

/// How many seeds run at once.
fn parallel() -> usize {
    std::thread::available_parallelism().map_or(4, |it| it.get()).clamp(2, 16)
}

/// Write the files of `ledger` to `<target>/tmp/budget-fuzz/<name>/`; where.
fn write(ledger: &FuzzLedger, name: &str) -> String {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("budget-fuzz").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the directory of a ledger");
    for (file, text) in ledger.files() {
        std::fs::write(dir.join(&file), text).expect("a file of a ledger");
    }
    dir.display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_fuzz_finds_no_difference() {
    run(DEFAULT_SEEDS).await;
}

/// The long run, every night (in four shards) and on demand (`cargo budget-fuzz`): about 7 minutes on 4 cores in
/// release, after the build.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "the long run: every night, and `cargo budget-fuzz`"]
async fn budget_fuzz_long_run_finds_no_difference() {
    run(LONG_SEEDS).await;
}

/// The seeds `ZHANG_BUDGET_FUZZ_SEEDS` names, or `default`, each checked; the default seeds of every run must exercise
/// everything the generator can.
async fn run(default: RangeInclusive<u64>) {
    let range = seeds(default).unwrap_or_else(|error| panic!("{error}"));
    let started = Instant::now();
    // a few workers, each taking the next seed
    let next = Arc::new(Mutex::new(range.clone()));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..parallel() {
        let next = next.clone();
        workers.spawn(async move {
            let mut outcomes = vec![];
            loop {
                let Some(seed) = next.lock().unwrap_or_else(PoisonError::into_inner).next() else {
                    return outcomes;
                };
                outcomes.push((seed, check(seed).await));
            }
        });
    }
    let mut outcomes = vec![];
    while let Some(done) = workers.join_next().await {
        outcomes.extend(done.unwrap_or_else(|failure| std::panic::resume_unwind(failure.into_panic())));
    }
    let (mut findings, mut errors, mut requests) = (vec![], 0, 0);
    let mut kinds = BTreeSet::new();
    let mut allowed = BTreeMap::<&str, usize>::new();
    let mut exercised = BTreeSet::new();
    for (seed, outcome) in outcomes {
        let ledger = FuzzLedger::generate(seed);
        // ZHANG_BUDGET_FUZZ_KEEP=1 keeps every seed's ledger, to read what the seeds generate
        if std::env::var_os("ZHANG_BUDGET_FUZZ_KEEP").is_some() {
            write(&ledger, &format!("ledgers/seed-{seed}"));
        }
        exercised.extend(ledger.exercised);
        match outcome {
            Ok(report) => {
                errors += report.errors;
                requests += report.requests;
                kinds.extend(report.kinds);
                for why in report.differences.iter().filter_map(|it| it.allowed) {
                    *allowed.entry(why).or_default() += 1;
                }
            }
            Err(finding) => findings.push(finding),
        }
    }
    eprintln!(
        "budget fuzz: seeds {}..={} in {:.1?}: {errors} errors and {requests} requests compared, {} seeds differ; error kinds: {}; allowed differences: {}",
        range.start(),
        range.end(),
        started.elapsed(),
        findings.len(),
        kinds.iter().cloned().collect::<Vec<_>>().join(", "),
        allowed.iter().map(|(why, count)| format!("{count} ({why})")).collect::<Vec<_>>().join(", ")
    );
    findings.sort_by_key(|finding| finding.seed);
    if let Some(first) = findings.first() {
        let kept = findings
            .iter()
            .map(|finding| format!("seed {}: {}", finding.seed, write(&finding.smallest, &format!("seed-{}", finding.seed))))
            .collect::<Vec<_>>();
        panic!(
            "{} of the seeds differ ({}); the first:\n{first}\nreproductions written to:\n{}",
            findings.len(),
            findings.iter().map(|it| it.seed.to_string()).collect::<Vec<_>>().join(", "),
            kept.join("\n")
        );
    }
    // the default seeds exercise everything the generator can, and every budget error the native code reports
    if range == DEFAULT_SEEDS {
        let missing = FEATURES.iter().filter(|it| !exercised.contains(*it)).collect::<Vec<_>>();
        assert!(missing.is_empty(), "the default seeds never exercise {missing:?}");
        for kind in [
            "BudgetDoesNotExist",
            "DefineDuplicatedBudget",
            "BudgetClosed",
            "BudgetCommodityMismatch",
            "PluginError",
        ] {
            assert!(kinds.contains(kind), "the default seeds never report {kind}: {kinds:?}");
        }
    }
}
