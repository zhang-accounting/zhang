//! The performance baseline of the native budget implementation, which #698 P4 compares the budget plugin against:
//! on generated ledgers of 10,000 and 100,000 transactions ([`zhang_testkit::budget_ledger`]), the load and reload
//! times with and without budgets, the time of the budget page's requests through the server's router, the peak
//! RSS, the bytes of the responses and of what the budget computation reads, and the time of a transaction written
//! through `POST /api/transactions`, which reloads the ledger before it answers. Peak RSS (`VmHWM`) is
//! process-global, so this bench has a binary of its own, and each ledger is a test of its own: nextest runs each in
//! a process of its own, and a test that finds another one ran in its process before does not report memory.
//!
//! Ignored, and skipped in a debug build. The method, the numbers and how to read them are on the "Budget baseline"
//! page of the developer docs (`docs/src/content/docs/developers/budget-baseline.md`). To run it:
//!
//! ```sh
//! cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture
//! ```
//!
//! `--no-capture` runs the tests one after the other, so that they do not time each other. Each prints its figures
//! and a `BASELINE {...}` JSON line; with `ZHANG_BUDGET_BASELINE_OUT=<dir>` it also writes the JSON to
//! `<dir>/<ledger>.json`, and with `ZHANG_BUDGET_BASELINE_KEEP=<dir>` it writes the ledger to `<dir>/<ledger>/`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use chrono::Datelike;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_ast::error::ErrorKind;
use zhang_ast::Directive;
use zhang_core::ledger::Ledger;
use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::http::{app, json_request, router_of, serve_config, Settings};
use zhang_testkit::ledger::Scratch;

/// the budgets of the baseline's ledgers
const BUDGETS: usize = 30;
/// the requests of each kind before the timed rounds
const WARM_UP: usize = 3;
/// the timed rounds: each times one request of every kind, in an order that rotates from round to round
const ROUNDS: usize = 30;
/// the reloads timed after the first load
const RELOADS: usize = 8;
/// the cold requests of each kind: the first request after a reload, which also fills the ledger's query cache
const COLD: usize = 2;
/// the transactions written before the timed ones; the first also adds the file it goes to and its `include`
const WRITE_WARM_UP: usize = 2;
/// the transactions written and timed
const WRITES: usize = 20;
/// the budget of the detail and interval requests: the one with the most postings
const DETAIL_BUDGET: &str = "Groceries";

/// The digests of the ledgers ([`zhang_testkit::budget_ledger::GeneratedLedger::sha256`]), as the baseline page
/// records them: the numbers there are for these ledgers.
const DIGESTS: &[(&str, &str)] = &[
    ("10000tx-30b-60m", "b46b01f46a38a81a4650f480bb141b4f4e2589b5a40d9e3f03d5b0bd7b1907d2"),
    ("10000tx-60m-no-budgets", "e1cc6c76b2b925d2c12b602b6b725fb76d8f95bacc85382423eb70c554914962"),
    ("100000tx-30b-60m", "7f4efc770c0b1bb22c47460e7f32dd91a5f8d72016b03d3165a810b4f92005a7"),
    ("100000tx-60m-no-budgets", "bf4c0b58d72b67b13870540fb0b7c458f089d579453dd9c66fb1987f948e5e07"),
];

#[test]
#[ignore = "performance baseline: cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture"]
fn budget_baseline_10k_transactions_with_budgets() {
    baseline(Spec::new(10_000, BUDGETS));
}

#[test]
#[ignore = "performance baseline: cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture"]
fn budget_baseline_10k_transactions_without_budgets() {
    baseline(Spec::new(10_000, BUDGETS).without_budgets());
}

#[test]
#[ignore = "performance baseline: cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture"]
fn budget_baseline_100k_transactions_with_budgets() {
    baseline(Spec::new(100_000, BUDGETS));
}

#[test]
#[ignore = "performance baseline: cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture"]
fn budget_baseline_100k_transactions_without_budgets() {
    baseline(Spec::new(100_000, BUDGETS).without_budgets());
}

// ---- the requests -----------------------------------------------------------------------------------------------

/// The requests the budget pages send, through the built-in queries that replaced the retired `/api/budgets`
/// endpoints, and a year of `#budgets` in BQL.
#[derive(Debug, Clone, Copy)]
enum Scenario {
    /// the budget list of the current month: `budgets.month`
    Month,
    /// the budget list of the ledger's first month: `budgets.month`
    MonthFirst,
    /// a budget's page: `budgets.budget` and `budgets.budget_month` in one batch
    Detail,
    /// a budget's month: `budgets.events` and `budgets.postings` in one batch
    Interval,
    /// a year of every budget: `SELECT name, date, assigned, activity, available FROM #budgets WHERE year = <year>`
    Bql,
}

const SCENARIOS: [Scenario; 5] = [Scenario::Month, Scenario::MonthFirst, Scenario::Detail, Scenario::Interval, Scenario::Bql];

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::Month => "month",
            Scenario::MonthFirst => "month_first",
            Scenario::Detail => "detail",
            Scenario::Interval => "interval",
            Scenario::Bql => "bql_year",
        }
    }

    /// the URI and the JSON body of the request, for the current month of `spec`'s clock unless it says otherwise
    fn request(self, spec: &Spec) -> (&'static str, Value) {
        let month = spec.current_month().to_string();
        let budget = DETAIL_BUDGET;
        match self {
            Scenario::Month => ("/api/query/builtins/budgets.month", json!({"params": {"month": month}})),
            Scenario::MonthFirst => ("/api/query/builtins/budgets.month", json!({"params": {"month": spec.first_day().to_string()}})),
            Scenario::Detail => (
                "/api/query/builtins",
                json!([
                    {"name": "budgets.budget", "params": {"name": budget}},
                    {"name": "budgets.budget_month", "params": {"name": budget, "month": month}},
                ]),
            ),
            Scenario::Interval => (
                "/api/query/builtins",
                json!([
                    {"name": "budgets.events", "params": {"name": budget, "month": month}},
                    {"name": "budgets.postings", "params": {"name": budget, "month": month}},
                ]),
            ),
            Scenario::Bql => (
                "/api/query",
                json!({"query": format!("SELECT name, date, assigned, activity, available FROM #budgets WHERE year = {}", spec.current_month().year())}),
            ),
        }
    }
}

/// a request's answer and how long the router took to give it, the body read whole
struct Timed {
    elapsed: Duration,
    status: StatusCode,
    bytes: bytes::Bytes,
}

impl Timed {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or_else(|_| panic!("expect JSON, got {}", String::from_utf8_lossy(&self.bytes)))
    }
}

/// `request` through the router, timed from sending it until its body is read; the request is built before
async fn timed(router: &Router, request: Request<Body>) -> Timed {
    let router = router.clone();
    let started = Instant::now();
    let response = router.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body reads");
    let elapsed = started.elapsed();
    Timed { elapsed, status, bytes }
}

async fn send(router: &Router, spec: &Spec, scenario: Scenario) -> Timed {
    let (uri, body) = scenario.request(spec);
    let answer = timed(router, json_request(Method::POST, uri, &[], Some(&body))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}: {}", scenario.name(), String::from_utf8_lossy(&answer.bytes));
    answer
}

/// `query` through `POST /api/query`
async fn query(router: &Router, query: &str) -> Timed {
    let answer = timed(router, json_request(Method::POST, "/api/query", &[], Some(&json!({"query": query})))).await;
    assert_eq!(answer.status, StatusCode::OK, "{query}: {}", String::from_utf8_lossy(&answer.bytes));
    answer
}

/// the cell of the first row and column of a query's answer, as an integer
fn first_cell(answer: &Timed) -> u64 {
    let cell = &answer.json()["data"]["rows"][0][0];
    cell.as_u64()
        .or_else(|| cell.as_str().and_then(|it| it.parse().ok()))
        .unwrap_or_else(|| panic!("a number, got {cell}"))
}

// ---- statistics and memory --------------------------------------------------------------------------------------

/// the nearest-rank percentile `p` of sorted samples
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank - 1]
}

fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// the number of samples, the minimum, the nearest-rank p50 and p95, the maximum and the samples in the order they
/// were taken, in milliseconds
fn summary(samples: &[Duration]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort();
    json!({
        "n": sorted.len(),
        "min_ms": ms(sorted[0]),
        "p50_ms": ms(percentile(&sorted, 50.0)),
        "p95_ms": ms(percentile(&sorted, 95.0)),
        "max_ms": ms(sorted[sorted.len() - 1]),
        "samples_ms": samples.iter().copied().map(ms).collect::<Vec<_>>(),
    })
}

/// a `kB` field of `/proc/self/status` (`VmRSS`, `VmHWM`), in MiB; `None` off Linux
fn status_mib(field: &str) -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(&format!("{field}:")))?;
    let kb = line.split_whitespace().nth(1)?.parse::<f64>().ok()?;
    Some((kb / 1024.0 * 10.0).round() / 10.0)
}

/// Reset the peak RSS of the process to its current RSS (Linux 4.0 and later); whether it could.
fn reset_peak_rss() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

/// the load average of the machine over 1, 5 and 15 minutes (`/proc/loadavg`): how busy the shared machine was
fn load_average() -> Option<String> {
    let text = std::fs::read_to_string("/proc/loadavg").ok()?;
    Some(text.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
}

/// the machine, as far as a test can read it without a shell
fn machine() -> Value {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpuinfo
        .lines()
        .find(|line| line.starts_with("model name"))
        .and_then(|line| line.split(':').nth(1));
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let memory = meminfo
        .lines()
        .find(|line| line.starts_with("MemTotal:"))
        .map(|line| line.split_whitespace().nth(1).unwrap_or_default());
    json!({
        "cpu": model.map(str::trim),
        "available_parallelism": std::thread::available_parallelism().map(|it| it.get()).ok(),
        "mem_total_kb": memory,
        "kernel": std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|it| it.trim().to_owned()),
    })
}

// ---- the bench --------------------------------------------------------------------------------------------------

/// the tests of this binary run one at a time
static SERIAL: Mutex<()> = Mutex::new(());
/// whether a test ran in this process before: its peak RSS would mask the next one's
static RAN: AtomicBool = AtomicBool::new(false);

/// What the budget computation reads, counted on the loaded ledger: the budget directives and the postings of the
/// accounts the `budget` metadata links.
fn input_counts(ledger: &Ledger) -> Value {
    let mut counts = std::collections::BTreeMap::<&str, u64>::new();
    let linked = ledger.budget_accounts().into_values().flatten().collect::<std::collections::BTreeSet<_>>();
    for directive in &ledger.directives {
        let kind = match &directive.data {
            Directive::Budget(_) => "budget",
            Directive::BudgetAdd(_) => "budget_add",
            Directive::BudgetTransfer(_) => "budget_transfer",
            Directive::BudgetClose(_) => "budget_close",
            Directive::Price(_) => "price",
            Directive::Transaction(txn) => {
                *counts.entry("postings").or_default() += txn.postings.len() as u64;
                let linked = txn.postings.iter().filter(|it| linked.contains(it.account.name())).count();
                *counts.entry("postings_on_linked_accounts").or_default() += linked as u64;
                "transaction"
            }
            _ => "other",
        };
        *counts.entry(kind).or_default() += 1;
    }
    counts.insert("linked_accounts", linked.len() as u64);
    json!(counts)
}

fn baseline(spec: Spec) {
    if cfg!(debug_assertions) {
        eprintln!("skipped: the budget baseline needs --release");
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let alone = !RAN.swap(true, Ordering::SeqCst);
    let name = spec.name();
    let load_average_before = load_average();

    // the ledger, checked to be the one the baseline page measured
    let started = Instant::now();
    let generated = spec.generate();
    let generate = started.elapsed();
    let (digest, size, files) = (generated.sha256(), generated.bytes(), generated.files().len());
    if let Some(dir) = std::env::var_os("ZHANG_BUDGET_BASELINE_KEEP") {
        generated.write_to(&PathBuf::from(dir).join(&name));
    }
    let scratch = generated.scratch();
    drop(generated);
    let expected = DIGESTS.iter().find(|(it, _)| *it == name).map(|(_, digest)| *digest);
    assert_eq!(Some(digest.as_str()), expected, "{name}: not the ledger the baseline page measured");

    let peak_reset = alone && reset_peak_rss();
    let rss_before = status_mib("VmRSS");
    let load = |dir: &Path| {
        let started = Instant::now();
        let ledger =
            Ledger::load_with_clock(dir.to_path_buf(), MAIN.to_owned(), data_source_for(MAIN), spec.clock()).unwrap_or_else(|error| panic!("{name}: {error}"));
        (ledger, started.elapsed())
    };
    let (ledger, first_load) = load(scratch.dir());
    let peak_load = status_mib("VmHWM");
    let rss_loaded = status_mib("VmRSS");
    // the two reopened accounts close with a balance; nothing else is wrong, budgets included
    let errors = ledger.errors.iter().map(|it| it.error_type.clone()).collect::<Vec<_>>();
    assert!(errors.iter().all(|kind| *kind == ErrorKind::CloseNonZeroAccount), "{name}: {errors:?}");
    let input = input_counts(&ledger);

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    let report = runtime.block_on(async {
        let shared = Arc::new(RwLock::new(ledger));
        let config = serve_config(scratch.dir(), MAIN, data_source_for(MAIN), &Settings::default());
        let router = router_of(app(config, shared.clone())).await;

        // reloads, as the server reloads: the new ledger replaces the old one once loaded
        let mut reloads = vec![];
        for _ in 0..RELOADS {
            let (next, elapsed) = load(scratch.dir());
            reloads.push(elapsed);
            *shared.write().await = next;
        }
        let peak_reloads = status_mib("VmHWM");
        // the first request of each kind after a reload, the kinds in turn
        let mut cold = SCENARIOS.map(|_| vec![]);
        if spec.with_budgets {
            for index in 0..COLD * SCENARIOS.len() {
                *shared.write().await = load(scratch.dir()).0;
                let scenario = index % SCENARIOS.len();
                cold[scenario].push(send(&router, &spec, SCENARIOS[scenario]).await.elapsed);
            }
        }

        let mut report = json!({
            "ledger": name,
            "spec": {"seed": spec.seed, "transactions": spec.transactions, "budgets": if spec.with_budgets { spec.budgets } else { 0 }, "months": spec.months},
            "files": files,
            "bytes": size,
            "sha256": digest,
            "generate_ms": ms(generate),
            "load_ms": ms(first_load),
            "reload": summary(&reloads),
            "errors": errors.len(),
            "input": input,
        });
        if spec.with_budgets {
            report["cold"] = Value::Object(
                SCENARIOS
                    .iter()
                    .zip(&cold)
                    .map(|(it, samples)| (it.name().to_owned(), summary(samples)))
                    .collect(),
            );
            let (requests, sizes) = requests(&router, &spec).await;
            report["requests"] = requests;
            report["response_bytes"] = sizes;
        }
        let peak_requests = status_mib("VmHWM");
        if spec.with_budgets {
            report["reads"] = reads(&router).await;
        }
        let peak_reads = status_mib("VmHWM");
        if spec.with_budgets {
            // the ledger of the reads goes; the writes go to a copy of its files
            drop((router, shared));
            report["writes"] = writes(&spec, &scratch, &load).await;
        }
        report["memory_mib"] = match alone {
            true => json!({
                "peak_reset": peak_reset,
                "rss_before_load": rss_before,
                "peak_load": peak_load,
                "rss_after_load": rss_loaded,
                "peak_after_reloads": peak_reloads,
                "peak_after_requests": peak_requests,
                "peak_after_reads": peak_reads,
                "peak_final": status_mib("VmHWM"),
                "rss_final": status_mib("VmRSS"),
            }),
            false => json!("not measured: another test ran in this process before"),
        };
        report["machine"] = machine();
        report["load_average"] = json!({"before": load_average_before, "after": load_average()});
        report
    });
    print(&report);
    if let Some(dir) = std::env::var_os("ZHANG_BUDGET_BASELINE_OUT") {
        let path = PathBuf::from(dir).join(format!("{name}.json"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the output directory");
        std::fs::write(&path, serde_json::to_string_pretty(&report).expect("JSON") + "\n").unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

/// The timed requests: warmed up, then [`ROUNDS`] rounds of one request of each kind, the order rotating by one
/// from round to round; and the bytes of each kind's response.
async fn requests(router: &Router, spec: &Spec) -> (Value, Value) {
    let mut sizes = serde_json::Map::new();
    for scenario in SCENARIOS {
        for _ in 0..WARM_UP {
            let answer = send(router, spec, scenario).await;
            sizes.insert(scenario.name().to_owned(), json!(answer.bytes.len()));
        }
    }
    let mut samples = SCENARIOS.map(|_| vec![]);
    for round in 0..ROUNDS {
        for offset in 0..SCENARIOS.len() {
            let index = (round + offset) % SCENARIOS.len();
            samples[index].push(send(router, spec, SCENARIOS[index]).await.elapsed);
        }
    }
    let timings = SCENARIOS
        .iter()
        .zip(&samples)
        .map(|(it, samples)| (it.name().to_owned(), summary(samples)))
        .collect();
    (Value::Object(timings), Value::Object(sizes))
}

/// The transaction the write `index` posts: groceries on the last but one day of the ledger, paid by card.
fn written_transaction(spec: &Spec, index: usize) -> Value {
    let date = spec.last_day().pred_opt().expect("a date");
    json!({
        "datetime": format!("{date}T12:00:00"),
        "payee": "Whole Foods",
        "narration": "groceries",
        "postings": [
            {"account": "Expenses:Food:Groceries", "unit": format!("{}.{:02} USD", 40 + index / 100, index % 100)},
            {"account": "Liabilities:CreditCard", "unit": null},
        ],
        "metas": [],
        "tags": [],
        "links": [],
    })
}

fn transactions(ledger: &Ledger) -> usize {
    ledger.directives.iter().filter(|it| matches!(it.data, Directive::Transaction(_))).count()
}

/// Transactions written through `POST /api/transactions`, [`WRITE_WARM_UP`] and then [`WRITES`] timed ones, to a copy
/// of the ledger's files served by a router of their own. A write appends the transaction to
/// `data/<year>/<month>.zhang` and reloads the ledger in place before it answers (no reload listener takes part), so
/// the ledger grows by one transaction a write.
async fn writes(spec: &Spec, scratch: &Scratch, load: &impl Fn(&Path) -> (Ledger, Duration)) -> Value {
    let copy = Scratch::copy_of_dir(scratch.dir(), MAIN);
    let (ledger, _) = load(copy.dir());
    let before = transactions(&ledger);
    let shared = Arc::new(RwLock::new(ledger));
    let config = serve_config(copy.dir(), MAIN, data_source_for(MAIN), &Settings::default());
    let router = router_of(app(config, shared.clone())).await;
    let (mut samples, mut bytes) = (vec![], 0);
    for index in 0..WRITE_WARM_UP + WRITES {
        let body = written_transaction(spec, index);
        let answer = timed(&router, json_request(Method::POST, "/api/transactions", &[], Some(&body))).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.bytes));
        // an id: the reload before the answer found the transaction written
        assert!(answer.json()["data"]["id"].is_string(), "{}", String::from_utf8_lossy(&answer.bytes));
        bytes = answer.bytes.len();
        if index >= WRITE_WARM_UP {
            samples.push(answer.elapsed);
        }
    }
    let after = transactions(&*shared.read().await);
    assert_eq!(after, before + WRITE_WARM_UP + WRITES, "one transaction a write");
    let mut report = summary(&samples);
    report["response_bytes"] = json!(bytes);
    report["transactions_before"] = json!(before);
    report["transactions_after"] = json!(after);
    report
}

/// What a plugin computing the budgets would read across its boundary, as the query engine answers it: how many
/// postings count toward a budget, how many `#budgets` rows every month of every budget is, and the bytes of two
/// ways to read the postings of the linked accounts, row by row or added up per account, month and currency.
async fn reads(router: &Router) -> Value {
    let linked = "length(account_budgets(account, date)) > 0";
    let counted = query(router, "SELECT count(*) WHERE length(budgets) > 0").await;
    let pairs = query(router, "SELECT sum(length(budgets)) WHERE length(budgets) > 0").await;
    let months = query(router, "SELECT count(*) FROM #budgets").await;
    let rows = query(
        router,
        &format!("SELECT date, time, account, units(position) AS units, cost_number, cost_currency WHERE {linked}"),
    )
    .await;
    let monthly = query(
        router,
        &format!(
            "SELECT account, yearmonth(date) AS month, currency, cost_currency, sum(number) AS number WHERE {linked} \
             GROUP BY account, month, currency, cost_currency"
        ),
    )
    .await;
    let rows_of = |answer: &Timed| answer.json()["data"]["rows"].as_array().map(Vec::len).unwrap_or_default();
    json!({
        "postings_counting_toward_a_budget": first_cell(&counted),
        "posting_budget_pairs": first_cell(&pairs),
        "budget_months": first_cell(&months),
        "linked_postings_rows": rows_of(&rows),
        "linked_postings_bytes": rows.bytes.len(),
        "linked_postings_ms": ms(rows.elapsed),
        "monthly_sums_rows": rows_of(&monthly),
        "monthly_sums_bytes": monthly.bytes.len(),
        "monthly_sums_ms": ms(monthly.elapsed),
    })
}

/// the report, as a table and as one JSON line
fn print(report: &Value) {
    println!("== budget baseline: {} ==", report["ledger"].as_str().unwrap_or_default());
    println!(
        "ledger  {} files, {} bytes, sha256 {}",
        report["files"],
        report["bytes"],
        report["sha256"].as_str().unwrap_or_default()
    );
    let reload = &report["reload"];
    println!(
        "load    first {} ms; reload min {} p50 {} p95 {} max {} ms",
        report["load_ms"], reload["min_ms"], reload["p50_ms"], reload["p95_ms"], reload["max_ms"]
    );
    if let Some(requests) = report["requests"].as_object() {
        println!(
            "{:<12} {:>4} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
            "request", "n", "min ms", "p50 ms", "p95 ms", "max ms", "bytes", "cold p50 ms"
        );
        for (name, stats) in requests {
            println!(
                "{:<12} {:>4} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
                name,
                stats["n"],
                stats["min_ms"],
                stats["p50_ms"],
                stats["p95_ms"],
                stats["max_ms"],
                report["response_bytes"][name],
                report["cold"][name]["p50_ms"]
            );
        }
        let writes = &report["writes"];
        println!(
            "{:<12} {:>4} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "write", writes["n"], writes["min_ms"], writes["p50_ms"], writes["p95_ms"], writes["max_ms"], writes["response_bytes"]
        );
        println!("reads   {}", report["reads"]);
    }
    println!("input   {}", report["input"]);
    println!("memory  {}", report["memory_mib"]);
    println!("machine {}; load average {}", report["machine"], report["load_average"]);
    println!("BASELINE {report}");
}
