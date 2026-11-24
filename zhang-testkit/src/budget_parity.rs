//! The budgets of a ledger computed twice and compared: with the bundled script `zhang:budget`
//! (`plugins/budget/budget.lua`), the official implementation of budgets, and with an oracle, each served as the server
//! serves a ledger. What a client sees is compared: the ledger's errors id-exact, the budget tables, the `budgets` of
//! `#postings`, and the budget pages' named queries.
//!
//! The oracle ([`Budgets::Oracle`]) is the budget analyzer of `zhang-plugin-sdk/examples/budget`, built for wasm32: an
//! implementation of the budgets in Rust, written to zhang's native budget code (RFC #698 P1.4), which answered as the
//! native code did on #835's contract and on the fuzzer's seeds 1 to 10,000, and which stands for that code since RFC
//! #698 P5 deleted it. [`BASELINE_ENV`] names another module in its place, such as an earlier `budget.lua`, to compare
//! a change of the script with the script it changes.
//!
//! A difference is classified by the rules of the testing guide ("The budget contract", allowed differences): the scale
//! of a decimal (`500.00` against `500`, the same value) is allowed where the answer types the value as a decimal (a
//! `decimal` cell, or the number of an `amount`, a `position` or an `inventory` cell), and so is the order of the load's
//! errors across places; anything else is not, a text that reads as a number included.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use axum::http::Method;
use axum::Router;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::analysis::{DefaultPlugin, LoadOptions};
use zhang_core::clock::Clock;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::PluginPolicy;

use crate::fixtures::{data_source_for, repo_root};
use crate::http::{app, call, router_of, serve_config, Settings};

/// A difference between two answers: what was expected (the oracle's, or a golden file's) and what the script answered.
#[derive(Debug, Clone)]
pub struct Difference {
    /// where: the request, then the JSON path in its answer
    pub path: String,
    pub expected: Value,
    pub actual: Value,
    /// why the testing guide allows it, or `None` for a regression
    pub allowed: Option<&'static str>,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.allowed {
            Some(why) => format!("allowed ({why})"),
            None => "NOT ALLOWED".to_owned(),
        };
        write!(
            f,
            "{}: expected {} / actual {}: {verdict}",
            self.path,
            short(&self.expected),
            short(&self.actual)
        )
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(200) {
        None => text,
        Some((end, _)) => format!("{}…", &text[..end]),
    }
}

/// The decimal a JSON string holds, if it is one: `"500.00"`.
fn decimal(value: &Value) -> Option<BigDecimal> {
    let text = value.as_str()?;
    let digits = text.strip_prefix('-').unwrap_or(text);
    let plain = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.') && digits.chars().filter(|c| *c == '.').count() <= 1;
    plain.then(|| BigDecimal::from_str(text).ok()).flatten()
}

/// What an answer says a value is, as far as the scale of a decimal goes: the cells of a table
/// (`{"columns": [{"name", "type"}], "rows": [[..]]}`) are typed by their column, and the scale may differ only in a
/// decimal. A text cell, an error's meta, or anything outside a table is compared as it is, even if it reads as a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typed {
    Other,
    /// a `decimal` cell, or the number of an amount
    Decimal,
    /// an `amount` cell, or a position's units or cost: `{"number", "currency", ..}`
    Amount,
    /// a `position` cell: `{"units", "cost"}`
    Position,
    /// an `inventory` cell, `{"positions": [..]}`, and its positions
    Inventory,
    Positions,
}

impl Typed {
    /// the cells of a column of the type `name`
    fn of_column(name: Option<&str>) -> Typed {
        match name {
            Some("decimal") => Typed::Decimal,
            Some("amount") => Typed::Amount,
            Some("position") => Typed::Position,
            Some("inventory") => Typed::Inventory,
            _ => Typed::Other,
        }
    }

    /// the field `key` of an object of this type
    fn field(self, key: &str) -> Typed {
        match (self, key) {
            (Typed::Amount, "number") => Typed::Decimal,
            (Typed::Position, "units" | "cost") => Typed::Amount,
            (Typed::Inventory, "positions") => Typed::Positions,
            _ => Typed::Other,
        }
    }

    /// an element of an array of this type
    fn element(self) -> Typed {
        match self {
            Typed::Positions => Typed::Position,
            _ => Typed::Other,
        }
    }
}

/// The types of the columns of `object`, when it is a table: `{"columns": [{"name", "type"}], "rows": [..]}`.
fn column_types(object: &serde_json::Map<String, Value>) -> Option<Vec<Typed>> {
    object.get("rows")?.as_array()?;
    let columns = object.get("columns")?.as_array()?;
    Some(
        columns
            .iter()
            .map(|column| Typed::of_column(column.get("type").and_then(Value::as_str)))
            .collect(),
    )
}

/// Every difference between `expected` and `actual`, by JSON path under `path`: objects by key, whatever the order of
/// their fields, arrays by position, the cells of a table typed by the expected answer's columns.
pub fn differences(path: &str, expected: &Value, actual: &Value) -> Vec<Difference> {
    let mut found = vec![];
    diff(path, expected, actual, Typed::Other, &mut found);
    found
}

fn diff(path: &str, expected: &Value, actual: &Value, typed: Typed, found: &mut Vec<Difference>) {
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            let columns = column_types(a);
            for (key, value) in a {
                let other = b.get(key).unwrap_or(&Value::Null);
                match &columns {
                    Some(types) if key == "rows" => diff_rows(&format!("{path}.rows"), value, other, types, found),
                    _ => diff(&format!("{path}.{key}"), value, other, typed.field(key), found),
                }
            }
            for (key, value) in b.iter().filter(|(key, _)| !a.contains_key(*key)) {
                diff(&format!("{path}.{key}"), &Value::Null, value, typed.field(key), found);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}[{index}]"), a, b, typed.element(), found);
            }
        }
        (a, b) if a == b => {}
        (a, b) => {
            let allowed = match (typed, decimal(a), decimal(b)) {
                (Typed::Decimal, Some(a), Some(b)) if a == b => Some("the scale of a decimal"),
                _ => None,
            };
            found.push(Difference {
                path: path.to_owned(),
                expected: a.clone(),
                actual: b.clone(),
                allowed,
            });
        }
    }
}

/// The rows of a table, each cell typed by its column in `types`.
fn diff_rows(path: &str, expected: &Value, actual: &Value, types: &[Typed], found: &mut Vec<Difference>) {
    let (Value::Array(a), Value::Array(b)) = (expected, actual) else {
        return diff(path, expected, actual, Typed::Other, found);
    };
    if a.len() != b.len() {
        return diff(path, expected, actual, Typed::Other, found);
    }
    for (index, (a, b)) in a.iter().zip(b).enumerate() {
        match (a, b) {
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
                for (column, (x, y)) in x.iter().zip(y).enumerate() {
                    let typed = types.get(column).copied().unwrap_or(Typed::Other);
                    diff(&format!("{path}[{index}][{column}]"), x, y, typed, found);
                }
            }
            _ => diff(&format!("{path}[{index}]"), a, b, Typed::Other, found),
        }
    }
}

/// The environment variable naming a module to compare the bundled script with in place of the SDK's budget analyzer:
/// a Lua script (such as the `budget.lua` of `main`, to compare a change of the script with it) or a wasm module
/// (`.wasm`), loaded as the default plugin `zhang:budget`.
pub const BASELINE_ENV: &str = "ZHANG_BUDGET_PARITY_BASELINE";

/// What computes the budgets of a ledger [`serve`] loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budgets {
    /// the bundled script `zhang:budget`, as the server loads a ledger
    Script,
    /// the oracle the script is compared with ([`oracle`]): the module [`BASELINE_ENV`] names, or the budget analyzer of
    /// `zhang-plugin-sdk/examples/budget`
    Oracle,
}

/// The budget analyzer of `zhang-plugin-sdk/examples/budget` (`zhang_plugin_example_budget.wasm`), built for wasm32 in
/// release once per process, with cargo, into `<target>/tmp/sdk-plugins`; `None` when the wasm32 target is not
/// installed, which fails on CI (`CI` set) and skips locally.
pub fn budget_analyzer() -> Option<Arc<[u8]>> {
    static MODULE: std::sync::OnceLock<Option<Arc<[u8]>>> = std::sync::OnceLock::new();
    MODULE
        .get_or_init(|| {
            const TARGET: &str = "wasm32-unknown-unknown";
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let sysroot = std::process::Command::new(rustc).args(["--print", "sysroot"]).output().ok()?;
            let installed = Path::new(String::from_utf8_lossy(&sysroot.stdout).trim())
                .join("lib/rustlib")
                .join(TARGET)
                .join("lib")
                .is_dir();
            if !installed {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "the {TARGET} target is not installed: CI must run the SDK's budget analyzer"
                );
                eprintln!("the {TARGET} target is not installed: no SDK budget analyzer to compare the budget script with");
                return None;
            }
            let target_dir = target_tmpdir().join("sdk-plugins");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let output = std::process::Command::new(cargo)
                .current_dir(repo_root())
                .args(["build", "--release", "--target", TARGET, "-p", "zhang-plugin-example-budget", "--target-dir"])
                .arg(&target_dir)
                .output()
                .expect("cargo runs");
            assert!(
                output.status.success(),
                "building the SDK's budget analyzer failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let module = target_dir.join(TARGET).join("release/zhang_plugin_example_budget.wasm");
            Some(Arc::from(
                std::fs::read(&module).unwrap_or_else(|error| panic!("{}: {error}", module.display())),
            ))
        })
        .clone()
}

/// The temporary directory of the target directory the running test binary was built in (`CARGO_TARGET_TMPDIR`, which
/// only a test target sees): `<target>/tmp`, the binary being `<target>/<profile>/deps/<name>`.
fn target_tmpdir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.ancestors().nth(3).map(|target| target.join("tmp")).unwrap_or_else(std::env::temp_dir)
}

/// A module the default plugin `zhang:budget` loads: its file name (a `.lua` script or a `.wasm` module) and its content.
pub type Module = (String, Arc<[u8]>);

/// The oracle the budget script is compared with, as the default plugin `zhang:budget` loads it: its file name and its
/// content. The module [`BASELINE_ENV`] names, else the SDK's budget analyzer ([`budget_analyzer`]); `Ok(None)` when
/// there is neither, locally without the wasm32 target, where a comparison has nothing to compare with.
pub fn oracle() -> Result<Option<Module>, String> {
    if let Some(path) = std::env::var_os(BASELINE_ENV) {
        let path = PathBuf::from(path);
        let content = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let file = path
            .file_name()
            .map_or_else(|| "baseline.lua".to_owned(), |it| it.to_string_lossy().into_owned());
        return Ok(Some((file, Arc::from(content))));
    }
    Ok(budget_analyzer().map(|module| ("zhang_plugin_example_budget.wasm".to_owned(), module)))
}

/// The options the server loads a ledger with (`zhang_server::analysis::load_options`), the default plugin
/// `zhang:budget` being the module `content`, a file named `file`, in place of the bundled script.
pub fn with_budget_plugin(file: &str, content: Arc<[u8]>) -> LoadOptions {
    let mut options = zhang_server::analysis::load_options(Arc::new(PluginPolicy::default()));
    let budget = options
        .default_plugins
        .iter_mut()
        .find(|plugin| plugin.module == "zhang:budget")
        .expect("the server registers zhang:budget");
    *budget = DefaultPlugin {
        module: "zhang:budget".to_owned(),
        file: file.to_owned(),
        content,
    };
    options
}

/// A ledger served as the server serves it, its budgets computed by `budgets`, and the ledger. Either must have
/// published its snapshot ([`published`]): a load whose analysis failed compares nothing. An error when the ledger does
/// not load, or when there is no oracle ([`oracle`]).
pub async fn serve(dir: &Path, entry: &str, clock: Clock, budgets: Budgets) -> Result<(Router, Arc<RwLock<Ledger>>), String> {
    let source = data_source_for(entry);
    let options = match budgets {
        Budgets::Script => zhang_server::analysis::load_options(Arc::new(PluginPolicy::default())),
        Budgets::Oracle => {
            let (file, content) = oracle()?.ok_or("no oracle to compare the budget script with: the wasm32 target is not installed")?;
            with_budget_plugin(&file, content)
        }
    };
    let (root, main, data_source) = (dir.to_path_buf(), entry.to_owned(), source.clone());
    let ledger = tokio::task::spawn_blocking(move || Ledger::load_with_options(root, main, data_source, clock, options))
        .await
        .expect("the load runs")
        .map_err(|error| error.to_string())?;
    match budgets {
        Budgets::Script => published(&ledger)?,
        Budgets::Oracle => published_by_any(&ledger)?,
    }
    let ledger = Arc::new(RwLock::new(ledger));
    let router = router_of(app(serve_config(dir, entry, source, &Settings::default()), ledger.clone())).await;
    Ok((router, ledger))
}

/// Whether `ledger`'s budgets come from the script `zhang:budget`: it ran, did not fail, and published its snapshot (or
/// had nothing to read); what went wrong otherwise.
pub fn published(ledger: &Ledger) -> Result<(), String> {
    if !ledger
        .plugins
        .analyzers()
        .any(|plugin| plugin.is_budget_provider() && plugin.script().is_some())
    {
        return Err("the budget script is not the default plugin zhang:budget".to_owned());
    }
    published_by_any(ledger)
}

/// Whether `ledger`'s budgets come from the default plugin `zhang:budget`, whatever module it is: it ran, did not fail,
/// and published its snapshot (or had nothing to read); what went wrong otherwise.
pub fn published_by_any(ledger: &Ledger) -> Result<(), String> {
    let failed = ledger
        .errors
        .iter()
        .filter(|error| error.error_type == zhang_ast::error::ErrorKind::PluginAnalysisFailed)
        .map(|error| format!("{:?}", error.metas))
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        return Err(format!("zhang:budget failed: {failed:?}"));
    }
    if !ledger.plugins.analyzers().any(|plugin| plugin.is_budget_provider()) {
        return Err("zhang:budget is not a default plugin of the ledger".to_owned());
    }
    let slot = ledger
        .extensions
        .analyzer("zhang:budget")
        .ok_or("zhang:budget has no tables in the ledger's catalog")?;
    match slot.snapshot.get().map(|snapshot| &snapshot.status) {
        Some(SnapshotStatus::Ok | SnapshotStatus::Skipped) => Ok(()),
        status => Err(format!("zhang:budget did not publish its snapshot: {status:?}")),
    }
}

/// Assert [`published`].
pub fn assert_published(ledger: &Ledger) {
    if let Err(why) = published(ledger) {
        panic!("{why}");
    }
}

/// The errors of `ledger`, id-exact, in the order the load reports them: id, kind, span (file within `dir`, start, end,
/// line) and every meta.
pub fn error_list(ledger: &Ledger, dir: &Path) -> Vec<Value> {
    ledger
        .errors
        .iter()
        .map(|error| {
            let span = error.span.as_ref().map(|span| {
                let file = span
                    .filename
                    .as_deref()
                    .map(|file| file.strip_prefix(dir).unwrap_or(file).display().to_string());
                json!({"file": file, "start": span.start, "end": span.end, "line": span.line})
            });
            json!({"id": error.id, "kind": error.error_type.to_string(), "span": span, "metas": error.metas.iter().collect::<BTreeMap<_, _>>()})
        })
        .collect()
}

/// The status and the body of the answer to `request`: `{"builtin": name, "params": {..}}`, `{"batch": [..]}` or
/// `{"bql": text}`.
pub async fn answer(router: &Router, request: &Value) -> (u16, Value) {
    let (uri, body) = if let Some(name) = request.get("builtin").and_then(Value::as_str) {
        (
            format!("/api/query/builtins/{name}"),
            json!({"params": request.get("params").cloned().unwrap_or_else(|| json!({}))}),
        )
    } else if let Some(batch) = request.get("batch") {
        ("/api/query/builtins".to_owned(), batch.clone())
    } else if let Some(query) = request.get("bql") {
        ("/api/query".to_owned(), json!({ "query": query }))
    } else {
        panic!("a request names a builtin, a batch or a bql query: {request}")
    };
    let reply = call(router, Method::POST, &uri, &[], Some(&body)).await;
    (reply.status.as_u16(), reply.body)
}

/// What a client reads of a ledger's budgets: the whole budget tables and the `budgets` of every posting, the errors
/// page, every budget as of every month of `months` (`YYYY-MM-01`), and the pages of the budgets `budgets` for the
/// months `pages`.
pub fn requests(months: &[String], budgets: &[String], pages: &[String]) -> Vec<Value> {
    let mut requests = vec![
        json!({"bql": "SELECT name, date, currency, alias, category, accounts, close, close_time FROM #budget_definitions"}),
        json!({"bql": "SELECT name, date, time, timestamp, type, amount FROM #budget_events"}),
        json!({"bql": "SELECT name, date, year, month, currency, alias, category, assigned, added, activity, available, closed, accounts FROM #budgets"}),
        json!({"bql": "SELECT seq, posting_index, date, time, account, units(position) AS units, budgets FROM #postings WHERE budgets IS NOT NULL"}),
        json!({"builtin": "journals.errors", "params": {"size": 1000, "offset": 0}}),
    ];
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

/// What [`compare`] found.
#[derive(Debug, Default)]
pub struct Report {
    pub requests: usize,
    /// how many errors the oracle's load reports, and of which kinds
    pub errors: usize,
    pub kinds: std::collections::BTreeSet<String>,
    pub differences: Vec<Difference>,
}

impl Report {
    pub fn regressions(&self) -> impl Iterator<Item = &Difference> {
        self.differences.iter().filter(|it| it.allowed.is_none())
    }
}

/// The ledger `dir/entry` loaded with `clock` by the oracle and by the script, compared: its errors, then the answers to
/// `requests`. Panics when there is no oracle ([`oracle`]): a caller that may run without one asks first.
pub async fn compare(dir: &Path, entry: &str, clock: Clock, requests: &[Value]) -> Report {
    compare_until(dir, entry, clock, requests, false).await.expect("the ledger loads")
}

/// [`compare`], stopping at the first difference the testing guide does not allow when `first` is set; an error when the
/// ledger does not load (both ways), or when there is no oracle.
pub async fn compare_until(dir: &Path, entry: &str, clock: Clock, requests: &[Value], first: bool) -> Result<Report, String> {
    let mut report = Report::default();
    oracle()?.ok_or("no oracle to compare the budget script with: the wasm32 target is not installed")?;
    let loads = (serve(dir, entry, clock, Budgets::Oracle).await, serve(dir, entry, clock, Budgets::Script).await);
    let ((oracle, oracle_ledger), (script, script_ledger)) = match loads {
        (Ok(oracle), Ok(script)) => (oracle, script),
        (Err(oracle), Err(script)) if oracle == script => return Err(oracle),
        (oracle, script) => {
            let outcome = |load: Result<_, String>| load.map_or_else(Value::from, |_| json!("loaded"));
            report.differences.push(Difference {
                path: "the load".to_owned(),
                expected: outcome(oracle),
                actual: outcome(script),
                allowed: None,
            });
            return Ok(report);
        }
    };

    // the errors id-exact, in the load's order; an order that differs only across places is allowed
    let (a, b) = (error_list(&*oracle_ledger.read().await, dir), error_list(&*script_ledger.read().await, dir));
    report.errors = a.len();
    let by_place = |list: &[Value]| {
        let mut list = list.to_vec();
        list.sort_by_key(|error| (error["span"]["file"].as_str().map(str::to_owned), error["span"]["start"].as_u64()));
        list
    };
    let differences = differences("errors", &Value::from(by_place(&a)), &Value::from(by_place(&b)));
    if differences.is_empty() && a != b {
        report.differences.push(Difference {
            path: "errors (load order)".to_owned(),
            expected: Value::from(a.len()),
            actual: Value::from(b.len()),
            allowed: Some("the order in which the load reports errors at different places"),
        });
    }
    report.differences.extend(differences);
    report.kinds = a.iter().filter_map(|error| error["kind"].as_str().map(str::to_owned)).collect();

    for request in requests {
        if first && report.regressions().next().is_some() {
            break;
        }
        let (a, b) = (answer(&oracle, request).await, answer(&script, request).await);
        let path = request.to_string();
        report.differences.extend(differences_of_answers(&path, &a, &b));
        report.requests += 1;
    }
    Ok(report)
}

fn differences_of_answers(path: &str, expected: &(u16, Value), actual: &(u16, Value)) -> Vec<Difference> {
    let mut found = differences(&format!("{path} status"), &json!(expected.0), &json!(actual.0));
    found.extend(differences(&format!("{path} body"), &expected.1, &actual.1));
    found
}
