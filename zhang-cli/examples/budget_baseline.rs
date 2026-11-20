//! The performance baseline of the native budget implementation, which #698 P4 compares the budget plugin against: on
//! generated ledgers of 10,000 and 100,000 transactions ([`zhang_testkit::budget_ledger`]), with budgets and without,
//! the load and reload times, the budget pages' requests (cold after a reload, and warm), the writes of transactions
//! and the reload the file watcher adds after each, readers stalled by it, readers concurrent with writes, the peak
//! RSS, and the bytes of the responses and of what the budget computation reads. The method, the numbers and how to
//! read them are on the "Budget Baseline" page of the developer docs
//! (`docs/src/content/docs/developers/budget-baseline.md`).
//!
//! ```sh
//! cargo run --release -p zhang --example budget_baseline                       # the four ledgers
//! cargo run --release -p zhang --example budget_baseline -- 100000tx-30b-60m   # one of them
//! ```
//!
//! The ledger is served as `zhang serve` serves a local ledger: read through `OpendalDataSource` with the `Fs`
//! service, the data source of the `zhang` binary, with the server's reload listener and file watcher running. The
//! data source is part of the binary, not of a library, so this is an example of the `zhang` package, which compiles
//! the binary's `src/opendal.rs` as a module of its own; a test target would compile the unit tests that file holds
//! too. The only difference from `zhang serve` is the clock: the ledger is loaded with a fixed one, so that "today",
//! and the months of `#budgets` with it, do not depend on the day of the run. The requests go through the server's
//! router in the process, without a socket.
//!
//! Without an argument the example runs each ledger in a process of its own, itself again, so that the peak RSS of a
//! ledger is its own. A run prints its figures on stderr and a `BASELINE {...}` JSON line on stdout; with
//! `ZHANG_BUDGET_BASELINE_OUT=<dir>` the JSON is also written to `<dir>/<ledger>.json`, and with
//! `ZHANG_BUDGET_BASELINE_KEEP=<dir>` the ledger, as generated, to `<dir>/<ledger>/`. A debug build refuses to run.

use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use chrono::Datelike;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_ast::error::ErrorKind;
use zhang_ast::Directive;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::broadcast::Broadcaster;
use zhang_server::{create_server_app, start_fs_event_listener, start_reload_listener, ReloadSender};
use zhang_testkit::budget_ledger::{Spec, MAIN};
use zhang_testkit::http::{json_request, router_of, serve_config, Settings};

#[path = "../src/opendal.rs"]
mod opendal;

use opendal::OpendalDataSource;

/// What `src/opendal.rs` reads of the options of `zhang serve`: the ledger's root and its main file.
pub struct ServerOpts {
    pub path: PathBuf,
    pub endpoint: String,
}

/// The storages `zhang serve` reads a ledger from; the bench reads a local one.
#[allow(dead_code)]
pub enum FileSystem {
    Fs,
    S3,
    WebDav,
    Github,
}

/// the budgets of the baseline's ledgers
const BUDGETS: usize = 30;
/// the timed reloads, after one that is not timed
const RELOADS: usize = 20;
/// the cold requests of each kind: the first request after a reload, which also fills the ledger's query cache
const COLD: usize = 5;
/// the requests of each kind before the timed rounds
const WARM_UP: usize = 3;
/// the timed rounds: each times one request of every kind, in an order that rotates from round to round
const ROUNDS: usize = 30;
/// the transactions written before the timed ones; the first also adds the file it goes to and its `include`
const WRITE_WARM_UP: usize = 2;
/// the transactions written and timed, each with the reload of the file watcher after it
const WRITES: usize = 20;
/// how long the bench waits after the watcher's reload of a write, for a further reload to show
const QUIET: Duration = Duration::from_secs(1);
/// the concurrent readers
const READERS: usize = 8;
/// the rounds of every kind of request each concurrent reader sends while nothing is written
const READER_ROUNDS: usize = 10;
/// the transactions written while the concurrent readers read
const CONCURRENT_WRITES: usize = 10;
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

/// the ledgers, in the order a run measures them
fn specs() -> [Spec; 4] {
    let (small, large) = (Spec::new(10_000, BUDGETS), Spec::new(100_000, BUDGETS));
    [small, small.without_budgets(), large, large.without_budgets()]
}

fn main() -> ExitCode {
    if cfg!(debug_assertions) {
        eprintln!("the budget baseline needs a release build: cargo run --release -p zhang --example budget_baseline");
        return ExitCode::FAILURE;
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => run_all(),
        [name] => match specs().iter().copied().find(|spec| spec.name() == *name) {
            Some(spec) => {
                let report = baseline(spec);
                println!("BASELINE {report}");
                ExitCode::SUCCESS
            }
            None => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    let names = specs().map(|spec| spec.name()).join(", ");
    eprintln!("usage: budget_baseline [ledger], where the ledger is one of {names}; without one, all of them");
    ExitCode::FAILURE
}

/// Every ledger in a process of its own, one after the other.
fn run_all() -> ExitCode {
    let exe = std::env::current_exe().expect("the example's path");
    for spec in specs() {
        let name = spec.name();
        let output = Command::new(&exe)
            .arg(&name)
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .unwrap_or_else(|error| panic!("{}: {}", name, error));
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(line) = stdout.lines().find(|line| line.starts_with("BASELINE ")) else {
            eprintln!("{name}: no report ({})", output.status);
            return ExitCode::FAILURE;
        };
        println!("{line}");
        if let Some(dir) = std::env::var_os("ZHANG_BUDGET_BASELINE_OUT") {
            let report: Value = serde_json::from_str(&line["BASELINE ".len()..]).expect("the report is JSON");
            let path = PathBuf::from(dir).join(format!("{name}.json"));
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("the output directory");
            std::fs::write(&path, serde_json::to_string_pretty(&report).expect("JSON") + "\n").unwrap_or_else(|error| panic!("{}: {}", path.display(), error));
        }
    }
    ExitCode::SUCCESS
}

// ---- the server's reloads ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    Started,
    Done,
}

/// The logger of the bench. It keeps when the server's reloads start and end, as `ReloadSender::reload_in_place` logs
/// them ("start reloading...", "ledger is reloaded successfully in ..."; every reload of the served ledger runs it, the
/// reload listener's and a write's alike), and, at the trace level, the kinds of the file events each batch of the
/// file watcher holds ("receive all file changes: ..."). It prints the other warnings and errors, but for the count
/// of errors of every load: the ledgers have two.
struct Reloads {
    events: Mutex<Vec<(Instant, Reload)>>,
    /// the kinds of the events of each batch of the file watcher, as notify writes them (`Modify(Data(Any))`)
    batches: Mutex<Vec<Vec<String>>>,
}

static RELOAD_LOG: Reloads = Reloads {
    events: Mutex::new(Vec::new()),
    batches: Mutex::new(Vec::new()),
};

impl log::Log for Reloads {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if record.target() != "zhang_server" {
            if record.level() <= log::Level::Warn && !(record.target() == "zhang_core::ledger" && record.args().to_string().starts_with("Ledger loaded with")) {
                eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
            }
            return;
        }
        let at = Instant::now();
        let message = record.args().to_string();
        if record.level() <= log::Level::Warn {
            eprintln!("[{} {}] {}", record.level(), record.target(), message);
        }
        let event = match message.as_str() {
            "start reloading..." => Reload::Started,
            it if it.starts_with("ledger is reloaded successfully") => Reload::Done,
            it if it.starts_with("receive all file changes") => {
                let kinds = it
                    .split("kind: ")
                    .skip(1)
                    .map(|kind| kind.split(", paths").next().unwrap_or_default().to_owned());
                self.batches.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(kinds.collect());
                return;
            }
            _ => return,
        };
        self.events.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((at, event));
    }

    fn flush(&self) {}
}

/// the batches of the file watcher logged so far
fn watcher_batches() -> Vec<Vec<String>> {
    RELOAD_LOG.batches.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// the reload events since `since`, in order
fn reloads_since(since: Instant) -> Vec<(Instant, Reload)> {
    let events = RELOAD_LOG.events.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    events.iter().copied().filter(|(at, _)| *at >= since).collect()
}

fn count(events: &[(Instant, Reload)], kind: Reload) -> usize {
    events.iter().filter(|(_, it)| *it == kind).count()
}

/// Wait until every reload started since `since` is done, `done` of them at least; panics after `limit`.
async fn reloads_done(since: Instant, done: usize, limit: Duration) -> Vec<(Instant, Reload)> {
    let deadline = Instant::now() + limit;
    loop {
        let events = reloads_since(since);
        let finished = count(&events, Reload::Done);
        if finished >= done && finished == count(&events, Reload::Started) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {} reloads done after {:?}: does the file watcher run?",
            finished,
            done,
            limit
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
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
    bytes: Bytes,
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
        .unwrap_or_else(|| panic!("a number, got {}", cell))
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

/// a transaction written through `POST /api/transactions`, which answers once the ledger is reloaded with it
async fn write(router: &Router, spec: &Spec, index: usize) -> Timed {
    let body = written_transaction(spec, index);
    let answer = timed(router, json_request(Method::POST, "/api/transactions", &[], Some(&body))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.bytes));
    // an id: the reload before the answer found the transaction written
    assert!(answer.json()["data"]["id"].is_string(), "{}", String::from_utf8_lossy(&answer.bytes));
    answer
}

fn transactions(ledger: &Ledger) -> usize {
    ledger.directives.iter().filter(|it| matches!(it.data, Directive::Transaction(_))).count()
}

// ---- statistics, memory and the machine -------------------------------------------------------------------------

/// the nearest-rank percentile `p` of sorted samples
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank - 1]
}

fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// the number of samples, the minimum, the nearest-rank p50 and p95 and the maximum, in milliseconds, and the samples
/// in the order they were taken when there are at most 200
fn summary(samples: &[Duration]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let mut summary = json!({
        "n": sorted.len(),
        "min_ms": ms(sorted[0]),
        "p50_ms": ms(percentile(&sorted, 50.0)),
        "p95_ms": ms(percentile(&sorted, 95.0)),
        "max_ms": ms(sorted[sorted.len() - 1]),
    });
    if samples.len() <= 200 {
        summary["samples_ms"] = json!(samples.iter().copied().map(ms).collect::<Vec<_>>());
    }
    summary
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

/// the machine, as far as the bench can read it without a shell
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

// ---- the bench --------------------------------------------------------------------------------------------------

fn baseline(spec: Spec) -> Value {
    log::set_logger(&RELOAD_LOG).expect("the bench's logger");
    log::set_max_level(log::LevelFilter::Info);
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
    let dir = scratch.dir().to_path_buf();

    let peak_reset = reset_peak_rss();
    let rss_before = status_mib("VmRSS");
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    let mut report = runtime.block_on(async {
        // the data source of `zhang serve` for a local ledger; `ZHANG_BUDGET_BASELINE_SOURCE=local` reads the files
        // with zhang-core's `LocalFileSystemDataSource` instead, to compare
        let mut opts = ServerOpts {
            path: dir.clone(),
            endpoint: MAIN.to_owned(),
        };
        let local = std::env::var("ZHANG_BUDGET_BASELINE_SOURCE").is_ok_and(|it| it == "local");
        let source: Arc<dyn DataSource> = match local {
            true => Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            false => Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.expect("the data source")),
        };

        // the first load, as `zhang serve` loads (`load_served_ledger`), on a blocking thread, with the fixed clock
        let started = Instant::now();
        let (root, data_source, clock) = (dir.clone(), source.clone(), spec.clock());
        let ledger = tokio::task::spawn_blocking(move || Ledger::load_with_clock(root, MAIN.to_owned(), data_source, clock))
            .await
            .expect("the load runs")
            .unwrap_or_else(|error| panic!("{}: {}", name, error));
        let first_load = started.elapsed();
        let (peak_load, rss_loaded) = (status_mib("VmHWM"), status_mib("VmRSS"));
        // the two reopened accounts close with a balance; nothing else is wrong, budgets included
        let errors = ledger.errors.iter().map(|it| it.error_type.clone()).collect::<Vec<_>>();
        assert!(errors.iter().all(|kind| *kind == ErrorKind::CloseNonZeroAccount), "{}: {:?}", name, errors);
        let input = input_counts(&ledger);

        // what `zhang serve` starts: the reload listener, the file watcher and the app
        let shared = Arc::new(RwLock::new(ledger));
        let broadcaster = Broadcaster::create();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        start_reload_listener(shared.clone(), broadcaster.clone(), reload_sender.clone(), receiver);
        start_fs_event_listener(shared.clone(), reload_sender.clone());
        let config = serve_config(&dir, MAIN, source.clone(), &Settings::default());
        let router = router_of(create_server_app(config, shared.clone(), broadcaster, reload_sender.clone())).await;

        // reloads through the reload listener, as the watcher asks for them: each loads the files again under the
        // ledger's write lock, replaces the ledger and drops the old one. The first, after which the listener runs,
        // is not timed
        let reload = || async {
            reload_sender
                .reload_and_wait()
                .await
                .unwrap_or_else(|failure| panic!("{}: {}", name, failure.message));
        };
        reload().await;
        let (mut reloads, since) = (vec![], Instant::now());
        for _ in 0..RELOADS {
            let started = Instant::now();
            reload().await;
            reloads.push(started.elapsed());
        }
        let logged = reload_durations(&reloads_since(since));
        let peak_reloads = status_mib("VmHWM");

        let mut report = json!({
            "ledger": name,
            "spec": {"seed": spec.seed, "transactions": spec.transactions, "budgets": if spec.with_budgets { spec.budgets } else { 0 }, "months": spec.months},
            "files": files,
            "bytes": size,
            "sha256": digest,
            "data_source": if local { "LocalFileSystemDataSource" } else { "OpendalDataSource (Fs)" },
            "generate_ms": ms(generate),
            "load_ms": ms(first_load),
            "reload": summary(&reloads),
            "reload_as_logged": summary(&logged),
            "errors": errors.len(),
            "input": input,
        });
        let mut memory = json!({
            "peak_reset": peak_reset,
            "rss_before_load": rss_before,
            "peak_load": peak_load,
            "rss_after_load": rss_loaded,
            "peak_after_reloads": peak_reloads,
        });
        if spec.with_budgets {
            // the first request of each kind after a reload, the kinds in turn
            let mut cold = SCENARIOS.map(|_| vec![]);
            for index in 0..COLD * SCENARIOS.len() {
                reload().await;
                let scenario = index % SCENARIOS.len();
                cold[scenario].push(send(&router, &spec, SCENARIOS[scenario]).await.elapsed);
            }
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
            memory["peak_after_requests"] = json!(status_mib("VmHWM"));
            report["reads"] = reads(&router).await;
            memory["peak_after_reads"] = json!(status_mib("VmHWM"));
            let before = transactions(&*shared.read().await);
            report["writes"] = writes(&router, &spec).await;
            memory["peak_after_writes"] = json!(status_mib("VmHWM"));
            report["concurrent"] = concurrent(&router, &spec).await;
            let after = transactions(&*shared.read().await);
            assert_eq!(after, before + WRITE_WARM_UP + WRITES + CONCURRENT_WRITES, "one transaction a write");
            report["transactions_written"] = json!(after - before);
        }
        memory["peak_final"] = json!(status_mib("VmHWM"));
        memory["rss_final"] = json!(status_mib("VmRSS"));
        report["memory_mib"] = memory;
        report
    });
    // the runtime stays: dropped, it would drop the file watcher's channel, whose sender, on notify's thread, panics
    std::mem::forget(runtime);
    report["machine"] = machine();
    report["load_average"] = json!({"before": load_average_before, "after": load_average()});
    print(&report);
    report
}

/// the durations of the reloads in `events`, from their start to their end as the server logs them
fn reload_durations(events: &[(Instant, Reload)]) -> Vec<Duration> {
    let starts = events.iter().filter(|(_, it)| *it == Reload::Started).map(|(at, _)| *at);
    let ends = events.iter().filter(|(_, it)| *it == Reload::Done).map(|(at, _)| *at);
    starts.zip(ends).map(|(start, end)| end - start).collect()
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

/// Transactions written one at a time, [`WRITE_WARM_UP`] and then [`WRITES`] timed ones. A write appends to
/// `data/<year>/<month>.zhang` and reloads the ledger in place before it answers; the file watcher then sees the file
/// written and asks the reload listener for a second reload. After each answer a reader sends `budgets.month` request
/// after request until that second reload is done: the longest of them is what the watcher's reload stalls a reader.
/// The bench then waits [`QUIET`] for a further reload before the next write.
async fn writes(router: &Router, spec: &Spec) -> Value {
    // the file watcher logs its batches at the trace level
    log::set_max_level(log::LevelFilter::Trace);
    let (mut latencies, mut stalls, mut watcher_delays, mut watcher_reloads, mut per_write) = (vec![], vec![], vec![], vec![], vec![]);
    let mut bytes = 0;
    let mut batches = 0;
    for index in 0..WRITE_WARM_UP + WRITES {
        if index == WRITE_WARM_UP {
            batches = watcher_batches().len();
        }
        let sent = Instant::now();
        let answer = write(router, spec, index).await;
        let answered = Instant::now();
        bytes = answer.bytes.len();
        let mut longest = Duration::ZERO;
        loop {
            longest = longest.max(send(router, spec, Scenario::Month).await.elapsed);
            if count(&reloads_since(sent), Reload::Done) >= 2 {
                break;
            }
            assert!(answered.elapsed() < Duration::from_secs(60), "no reload of the watcher a minute after a write");
        }
        tokio::time::sleep(QUIET).await;
        let events = reloads_done(sent, 2, Duration::from_secs(60)).await;
        if index < WRITE_WARM_UP {
            continue;
        }
        let starts = events.iter().filter(|(_, it)| *it == Reload::Started).map(|(at, _)| *at).collect::<Vec<_>>();
        let durations = reload_durations(&events);
        latencies.push(answer.elapsed);
        stalls.push(longest);
        // the watcher's reload starts once the write released the ledger: right after the answer when the write's own
        // reload took longer than the watcher waits
        watcher_delays.push(starts[1].saturating_duration_since(answered));
        watcher_reloads.push(durations[1]);
        per_write.push(starts.len());
    }
    // the batches of the timed writes: how many events the watcher waited for, and of which kinds
    let batches = watcher_batches().split_off(batches);
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for kind in batches.iter().flatten() {
        *kinds.entry(kind.clone()).or_default() += 1;
    }
    let sizes = batches.iter().map(Vec::len).collect::<Vec<_>>();
    log::set_max_level(log::LevelFilter::Info);
    json!({
        "write": summary(&latencies),
        "watcher_batches": {"n": batches.len(), "events_min": sizes.iter().min(), "events_max": sizes.iter().max(), "kinds": kinds},
        "response_bytes": bytes,
        "reloads_per_write": {"min": per_write.iter().min(), "max": per_write.iter().max()},
        "watcher_reload_start_after_answer": summary(&watcher_delays),
        "watcher_reload": summary(&watcher_reloads),
        "reader_stall": summary(&stalls),
    })
}

/// [`READERS`] readers sending the budget pages' requests at once, the kinds in turn: first while nothing is written,
/// [`READER_ROUNDS`] rounds of every kind each; then while [`CONCURRENT_WRITES`] transactions are written one after
/// the other, each once the watcher's reload of the one before is done.
async fn concurrent(router: &Router, spec: &Spec) -> Value {
    let readers = |rounds: Option<usize>, stop: Arc<AtomicBool>| {
        (0..READERS)
            .map(|reader| {
                let (router, spec, stop) = (router.clone(), *spec, stop.clone());
                tokio::spawn(async move {
                    let mut samples = vec![];
                    let mut index = reader;
                    while rounds.is_none_or(|rounds| samples.len() < rounds * SCENARIOS.len()) && !stop.load(Ordering::SeqCst) {
                        samples.push(send(&router, &spec, SCENARIOS[index % SCENARIOS.len()]).await.elapsed);
                        index += 1;
                    }
                    samples
                })
            })
            .collect::<Vec<_>>()
    };
    let gather = |tasks: Vec<tokio::task::JoinHandle<Vec<Duration>>>| async move {
        let mut all = vec![];
        for task in tasks {
            all.extend(task.await.expect("a reader"));
        }
        all
    };

    let quiet = gather(readers(Some(READER_ROUNDS), Arc::new(AtomicBool::new(false)))).await;

    let stop = Arc::new(AtomicBool::new(false));
    let tasks = readers(None, stop.clone());
    let (mut written, started) = (vec![], Instant::now());
    for index in 0..CONCURRENT_WRITES {
        let sent = Instant::now();
        written.push(write(router, spec, WRITE_WARM_UP + WRITES + index).await.elapsed);
        reloads_done(sent, 2, Duration::from_secs(60)).await;
    }
    let period = started.elapsed();
    stop.store(true, Ordering::SeqCst);
    let during_writes = gather(tasks).await;
    json!({
        "readers": READERS,
        "reads_without_writes": summary(&quiet),
        "reads_during_writes": summary(&during_writes),
        "writes": summary(&written),
        "period_ms": ms(period),
    })
}

/// the report, as a table on stderr
fn print(report: &Value) {
    let line = |name: &str, stats: &Value, extra: String| {
        eprintln!(
            "{:<22} {:>5} {:>10} {:>10} {:>10} {:>10} {}",
            name, stats["n"], stats["min_ms"], stats["p50_ms"], stats["p95_ms"], stats["max_ms"], extra
        );
    };
    eprintln!("== budget baseline: {} ==", report["ledger"].as_str().unwrap_or_default());
    eprintln!(
        "ledger  {} files, {} bytes, sha256 {}, read with {}",
        report["files"],
        report["bytes"],
        report["sha256"].as_str().unwrap_or_default(),
        report["data_source"]
    );
    eprintln!("{:<22} {:>5} {:>10} {:>10} {:>10} {:>10}", "ms", "n", "min", "p50", "p95", "max");
    eprintln!("{:<22} {:>5} {:>10}", "load", 1, report["load_ms"]);
    line("reload", &report["reload"], String::new());
    line("reload, as logged", &report["reload_as_logged"], String::new());
    if let Some(requests) = report["requests"].as_object() {
        for (name, stats) in requests {
            let cold = &report["cold"][name];
            line(
                name,
                stats,
                format!("bytes {}, cold {} – {}", report["response_bytes"][name], cold["min_ms"], cold["max_ms"]),
            );
        }
        let writes = &report["writes"];
        line(
            "write",
            &writes["write"],
            format!("bytes {}, reloads a write {}", writes["response_bytes"], writes["reloads_per_write"]),
        );
        line("watcher reload", &writes["watcher_reload"], format!("batches {}", writes["watcher_batches"]));
        line("  start after answer", &writes["watcher_reload_start_after_answer"], String::new());
        line("reader stall", &writes["reader_stall"], String::new());
        let concurrent = &report["concurrent"];
        line("8 readers", &concurrent["reads_without_writes"], String::new());
        line(
            "8 readers, writes",
            &concurrent["reads_during_writes"],
            format!("over {} ms", concurrent["period_ms"]),
        );
        line("writes, 8 readers", &concurrent["writes"], String::new());
        eprintln!("reads   {}", report["reads"]);
    }
    eprintln!("input   {}", report["input"]);
    eprintln!("memory  {}", report["memory_mib"]);
    eprintln!("machine {}; load average {}", report["machine"], report["load_average"]);
}
