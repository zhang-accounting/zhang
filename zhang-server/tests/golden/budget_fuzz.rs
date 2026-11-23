//! RFC #698's differential fuzzing of budgets: seeded generated ledgers ([`zhang_testkit::budget_fuzz`]) that use every
//! budget directive and break the budgets in every way the native code reports, each loaded natively and with the
//! bundled script `zhang:budget`, and compared as a client reads them: the errors id-exact, the three budget tables,
//! the `budgets` of the postings and the budget pages' named queries.
//!
//! A difference fails the test with the seed, the first difference, and the smallest ledger found that still shows one,
//! also written to `<target>/tmp/budget-fuzz/seed-<seed>/`. The test runs the seeds [`DEFAULT_SEEDS`]; another range
//! runs on demand, in release for speed:
//!
//! ```sh
//! ZHANG_BUDGET_FUZZ_SEEDS=1..=2000 cargo nextest run --release -p zhang-server --test golden -E 'test(budget_fuzz)'
//! ZHANG_BUDGET_FUZZ_SEEDS=17 cargo nextest run -p zhang-server --test golden -E 'test(budget_fuzz)'   # one seed
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use zhang_testkit::budget_fuzz::{check, seeds, FuzzLedger, FEATURES};

/// The seeds of every test run.
const DEFAULT_SEEDS: std::ops::RangeInclusive<u64> = 1..=40;

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
    let range = seeds(DEFAULT_SEEDS).unwrap_or_else(|error| panic!("{error}"));
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
