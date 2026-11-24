//! The budget contract (#698 P0): what a client of the budget system sees, frozen as data, so that moving budgets out
//! of the core and the query engine into the official plugin (RFC #698) can be diffed against it. Nothing here reads a
//! Rust type of the budget system: a case is a ledger and the requests a client sends, and its golden files hold what
//! the server answers, so the cases keep their meaning whatever implements budgets.
//!
//! Each directory of `budget_contract/` is a case. Its `case.json` holds
//!
//! - `description`: what the case pins;
//! - `ledger`: the directory of the ledger, relative to the repository root, when the case wraps a ledger kept
//!   elsewhere (an `integration-tests/` case); the case's own directory otherwise;
//! - `entries`: the main files to load, `main.zhang` and `main.bean` (those present) by default;
//! - `timezone`: the ledger's timezone, for a wrapped ledger that sets none: an `option "timezone"` appended to a copy
//!   of its main file, where it moves no line. Every case's ledger has a timezone: without one a ledger takes the
//!   system's, and the answers would depend on the machine;
//! - `now`: the ledger's clock (RFC 3339), which sets "today" and so the current month of the budget pages;
//! - `pages`: `{"months": [..], "budgets": [..]}`, the reads of the budget pages and the app for those months
//!   (`YYYY-MM-01`): the budget list of each month (`budgets.month`), and for each budget its page (`budgets.budget`
//!   with `budgets.budget_month`) and its activity of the month, as the app reads it (`budgets.events` with
//!   `budgets.postings`) and as the web UI does (`budgets.budget`, `budgets.events` and `budgets.postings`), each batch
//!   as the client sends it;
//! - `tables`: whether to read the whole of `#budget_definitions`, `#budget_events`, `#budgets` and the `budgets` of
//!   every posting of `#postings` (true by default);
//! - `requests`: what else a client sends, through the router the server runs: `{"builtin": name, "params": {..}}` is
//!   `POST /api/query/builtins/{name}`, `{"batch": [..]}` is `POST /api/query/builtins`, `{"bql": text}` is
//!   `POST /api/query`, and `{"method", "uri", "body"}` any other request. A request may carry a `note`, copied into
//!   the golden file with it; a note starting with `KNOWN-ISSUE:` marks a behaviour that looks wrong and is pinned as
//!   it is, to be decided on its own rather than changed by the migration;
//! - `known_issues`: notes on the export or a round trip, by name (`export`, `round_trip.beancount`, ...).
//!
//! The golden files of a case, per entry, rewritten with `UPDATE_GOLDEN=1` (`zhang_testkit::golden`):
//!
//! - `expected.<entry>.json`: the ledger's errors as `journals.errors` lists them (by file, then position), reduced to
//!   their kind, file, line and metas; the same errors id-exact (the core's list, `Ledger::errors`, which the plugin's
//!   diagnostics join): id, kind, span (file, start, end, line) and every meta, `txn_id` included, with the id spelled
//!   out as the `(place, kind, n)` it derives from (the `n`-th error of its kind at its place, `FromSpan::of_error`),
//!   in the API's order, by file and position, the errors of one place in the order the load reports them (the order
//!   of the places in the load's own list is not pinned: no client sees it); every request with the HTTP status and
//!   the JSON body of its answer; and the round trips: every
//!   directive of the ledger as its format reads it (but the `include`s and the comments), written in one format
//!   (`zhang`, `beancount`) as one main file, loaded with the same clock and asked the same requests. A round trip is
//!   `"same"` when every answer equals the original apart from what depends on where a directive is written (the ids
//!   of transactions, the file and line of errors, the main file a message names), otherwise it lists the errors and
//!   the answers that differ, or why the text does not load and the line it names;
//! - `export.<entry>.txt`: the budget directives and the `open`s linking accounts to budgets, as written, file after
//!   file, as the zhang and the beancount formats write them.
//!
//! What the migration may change in these files, and what it may not, is listed in the testing guide
//! (`docs/src/content/docs/developers/testing.md`, "The budget contract").
//!
//! `schema.json` holds the columns of the budget tables and the parameters and columns of the built-in queries the
//! budget pages and the app run, as `GET /api/query/schema` and `GET /api/query/builtins` list them.
//!
//! **The implementations of budgets.** Each case runs as the server loads a ledger (`load_options`): with the bundled
//! script `zhang:budget`, the only implementation of budgets (RFC #698 P3.3), or, in a build with the `budget-switch`
//! feature, with the implementation `ZHANG_BUDGET_IMPL` chooses. That run compares with the golden files byte for
//! byte, and rewrites them when asked to. Every case also runs, compared with the same golden files and never
//! rewriting them,
//!
//! - natively, with RFC #698's test switch (`budget_contract_natively`), and with the script whatever the build
//!   (`budget_contract_as_the_budget_script`): each difference is classified by the testing guide's allowed
//!   differences, and any other fails the test;
//! - with the budget analyzer of `zhang-plugin-sdk/examples/budget` (built for wasm32) as the default plugin
//!   `zhang:budget` in place of the script (RFC #698 P1.4), byte for byte: the analyzer's diagnostics land at the
//!   places and with the ids the native checks give, and its tables answer alike.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::Method;
use axum::Router;
use beancount::Beancount;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;
use uuid::Uuid;
use zhang_ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::PluginPolicy;
use zhang_core::utils::id::FromSpan;
use zhang_server::analysis::{load_options_for, BudgetImpl};
use zhang_testkit::budget_parity::{budget_analyzer, with_budget_plugin};
use zhang_testkit::fixtures::{data_source_for, repo_root};
use zhang_testkit::golden;
use zhang_testkit::http::{app, call, router_of, serve_config, Settings};
use zhang_testkit::ledger::Scratch;

/// The directory of the cases.
fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/budget_contract")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    description: String,
    ledger: Option<String>,
    entries: Option<Vec<String>>,
    timezone: Option<String>,
    now: String,
    pages: Option<Pages>,
    tables: Option<bool>,
    #[serde(default)]
    requests: Vec<Value>,
    #[serde(default)]
    known_issues: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pages {
    months: Vec<String>,
    budgets: Vec<String>,
}

/// The whole of the budget tables, and the budgets of every posting.
const TABLES: &[&str] = &[
    "SELECT name, date, currency, alias, category, accounts, close, close_time FROM #budget_definitions",
    "SELECT name, date, time, timestamp, type, amount FROM #budget_events",
    "SELECT name, date, year, month, currency, alias, category, assigned, added, activity, available, closed, accounts FROM #budgets",
    "SELECT date, time, account, narration, units(position) AS units, budgets FROM #postings",
];

impl CaseFile {
    /// Every request of the case: the clock, the pages, the tables, then the requests written out.
    fn all_requests(&self) -> Vec<Value> {
        let mut requests = vec![json!({"builtin": "ledger.now"})];
        if let Some(pages) = &self.pages {
            for month in &pages.months {
                requests.push(json!({"builtin": "budgets.month", "params": {"month": month}}));
            }
            for name in &pages.budgets {
                for month in &pages.months {
                    requests.push(json!({"batch": [
                        {"name": "budgets.budget", "params": {"name": name}},
                        {"name": "budgets.budget_month", "params": {"name": name, "month": month}},
                    ]}));
                    requests.push(json!({"batch": [
                        {"name": "budgets.events", "params": {"name": name, "month": month}},
                        {"name": "budgets.postings", "params": {"name": name, "month": month}},
                    ]}));
                    requests.push(json!({"batch": [
                        {"name": "budgets.budget", "params": {"name": name}},
                        {"name": "budgets.events", "params": {"name": name, "month": month}},
                        {"name": "budgets.postings", "params": {"name": name, "month": month}},
                    ]}));
                }
            }
        }
        if self.tables.unwrap_or(true) {
            requests.extend(TABLES.iter().map(|query| json!({ "bql": query })));
        }
        requests.extend(self.requests.iter().cloned());
        requests
    }
}

/// The method, the path and the body of a request of `case.json`.
fn http_request(request: &Value) -> (Method, String, Option<Value>) {
    let object = request.as_object().unwrap_or_else(|| panic!("a request is an object: {request}"));
    if let Some(name) = object.get("builtin").and_then(Value::as_str) {
        let mut body = json!({"params": object.get("params").cloned().unwrap_or_else(|| json!({}))});
        if let Some(count_total) = object.get("count_total") {
            body["count_total"] = count_total.clone();
        }
        return (Method::POST, format!("/api/query/builtins/{name}"), Some(body));
    }
    if let Some(batch) = object.get("batch") {
        return (Method::POST, "/api/query/builtins".to_owned(), Some(batch.clone()));
    }
    if let Some(query) = object.get("bql") {
        return (Method::POST, "/api/query".to_owned(), Some(json!({ "query": query })));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a request names a builtin, a batch, a bql query or a method: {request}"));
    let uri = object
        .get("uri")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a request has a uri: {request}"));
    (method.parse().expect("an HTTP method"), uri.to_owned(), object.get("body").cloned())
}

/// What computes the budgets of a run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Budgets {
    /// what the server loads a ledger with (`zhang_server::analysis::load_options`): the bundled script `zhang:budget`,
    /// or what `ZHANG_BUDGET_IMPL` chooses in a build with the `budget-switch` feature. Its answers are the golden files
    Default,
    /// zhang's native budget code, through RFC #698's test switch: compared with the golden files by value, as
    /// [`Budgets::Script`] is
    Native,
    /// the budget analyzer of the SDK's examples, as the default plugin `zhang:budget`
    Analyzer,
    /// the bundled script `zhang:budget` (`plugins/budget/budget.lua`), as the default plugin: RFC #698's `plugin`
    /// mode, whatever the build. Its answers are compared with the golden files by value, and every difference is
    /// classified by the testing guide's allowed differences ([`zhang_testkit::budget_parity`])
    Script,
}

/// The differences found with the golden files, by case and file: collected by [`compare`], and reported by the test
/// that runs the script ([`SCRIPT_DIFFERENCES`]) or the native code ([`NATIVE_DIFFERENCES`]).
type Differences = std::sync::Mutex<Vec<(String, zhang_testkit::budget_parity::Difference)>>;

/// The differences the script's answers have with the golden files.
static SCRIPT_DIFFERENCES: Differences = std::sync::Mutex::new(Vec::new());

/// The differences the native budgets' answers have with the golden files.
static NATIVE_DIFFERENCES: Differences = std::sync::Mutex::new(Vec::new());

/// A server on the ledger `dir/entry`, loaded in the format of `entry` with its clock at `now`, its budgets computed by
/// `budgets`, and the ledger it serves; the load's error when it does not load.
async fn server(dir: &Path, entry: &str, now: Clock, budgets: Budgets) -> Result<(Router, Arc<RwLock<Ledger>>), String> {
    let source = data_source_for(entry);
    let options = match budgets {
        Budgets::Default => zhang_server::analysis::load_options(Arc::new(PluginPolicy::default())),
        Budgets::Native => load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Native),
        Budgets::Script => load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Plugin),
        Budgets::Analyzer => with_budget_plugin("zhang_plugin_example_budget.wasm", budget_analyzer().expect("the budget analyzer is built")),
    };
    let script = options.default_plugins.iter().any(|plugin| plugin.file == "plugins/budget/budget.lua");
    let ledger = Ledger::load_with_options(dir.to_path_buf(), entry.to_owned(), source.clone(), now, options).map_err(|error| error.to_string())?;
    if script {
        // the budgets are the script's: it ran, and published
        zhang_testkit::budget_parity::assert_published(&ledger);
    }
    if budgets == Budgets::Analyzer {
        // the analyzer ran, and served the budget tables
        let failed = ledger
            .errors
            .iter()
            .filter(|error| error.error_type == zhang_ast::error::ErrorKind::PluginAnalysisFailed)
            .map(|error| format!("{:?}", error.metas))
            .collect::<Vec<_>>();
        assert!(failed.is_empty(), "the budget analyzer failed: {failed:?}");
        assert!(
            ledger.plugins.analyzers().any(|plugin| plugin.is_budget_provider()),
            "the budget analyzer is the default plugin zhang:budget"
        );
        let slot = ledger
            .extensions
            .analyzer("zhang:budget")
            .expect("the budget analyzer has tables in the ledger's catalog");
        let status = &slot.snapshot.get().expect("the budget analyzer ran").status;
        assert!(
            matches!(status, SnapshotStatus::Ok | SnapshotStatus::Skipped),
            "the budget analyzer published its snapshot: {status:?}"
        );
        assert_ne!(
            zhang_query::Catalog::of_ledger(&ledger).digest(),
            zhang_query::Catalog::standard().digest(),
            "the budget tables are served from the analyzer's snapshot"
        );
    }
    let ledger = Arc::new(RwLock::new(ledger));
    let router = router_of(app(serve_config(dir, entry, source, &Settings::default()), ledger.clone())).await;
    Ok((router, ledger))
}

/// The errors of `ledger`, whose files are in `dir`, id-exact: the id and the `(place, kind, n)` it derives from, the
/// kind, the span and every meta. They are ordered as `#errors` and `journals.errors` order them, by file, then by
/// position, the errors of one place in the order the load reported them: which place the load checks first is not
/// for a client to see, so it is no part of the contract.
fn error_list(ledger: &Ledger, dir: &Path) -> Vec<Value> {
    let mut seen: HashMap<(Option<PathBuf>, usize, String), usize> = HashMap::new();
    let mut errors = ledger
        .errors
        .iter()
        .map(|error| {
            let kind = error.error_type.to_string();
            let metas = error.metas.iter().collect::<BTreeMap<_, _>>();
            let Some(span) = &error.span else {
                return json!({"id": error.id, "kind": kind, "span": null, "metas": metas});
            };
            let file = span
                .filename
                .as_deref()
                .map(|file| file.strip_prefix(dir).unwrap_or(file).display().to_string());
            // the id is that of the `n`-th error of its kind at its place
            let n = seen.entry((span.filename.clone(), span.start, kind.clone())).or_default();
            let derived =
                (Uuid::of_error(span, &kind, *n).to_string() == error.id).then(|| format!("{}@{} {kind} #{n}", file.as_deref().unwrap_or("-"), span.start));
            *n += 1;
            json!({
                "id": error.id,
                "id_of": derived,
                "kind": kind,
                "span": {"file": file, "start": span.start, "end": span.end, "line": span.line},
                "metas": metas,
            })
        })
        .collect::<Vec<_>>();
    // a stable sort: the errors of one place keep the load's order
    errors.sort_by_key(|error| (error["span"]["file"].as_str().map(str::to_owned), error["span"]["start"].as_u64()));
    errors
}

/// The status and the body of a request's answer.
async fn answer(router: &Router, request: &Value) -> (u16, Value) {
    let (method, uri, body) = http_request(request);
    let reply = call(router, method, &uri, &[], body.as_ref()).await;
    (reply.status.as_u16(), reply.body)
}

/// The rows of a query result (`{"columns": [..], "rows": [..]}`), as objects by column name.
fn rows_by_column(result: &Value) -> Vec<Map<String, Value>> {
    let columns = result["columns"]
        .as_array()
        .unwrap_or_else(|| panic!("a query result: {result}"))
        .iter()
        .map(|column| column["name"].as_str().expect("a column name").to_owned())
        .collect::<Vec<_>>();
    result["rows"]
        .as_array()
        .expect("the rows of a query result")
        .iter()
        .map(|row| columns.iter().cloned().zip(row.as_array().expect("a row").iter().cloned()).collect())
        .collect()
}

/// The ledger's errors, as `journals.errors` lists them: their kind, file, line and metas.
async fn errors(router: &Router) -> Vec<Value> {
    let request = json!({"builtin": "journals.errors", "params": {"size": 10000, "offset": 0}});
    let (status, body) = answer(router, &request).await;
    assert_eq!(status, 200, "journals.errors: {body}");
    rows_by_column(&body["data"])
        .into_iter()
        .map(|row| json!({"kind": row["kind"], "file": row["file"], "line": row["line"], "metas": row["metas"]}))
        .collect()
}

/// The errors without where they are written: their kind and metas, but the id of a transaction (`txn_id`), which
/// derives from where the transaction is written, in an order of their own (the errors are listed by file).
fn unplaced(errors: &[Value]) -> Vec<Value> {
    let metas = |metas: &Value| match metas {
        Value::Array(metas) => Value::Array(metas.iter().filter(|meta| meta["key"] != "txn_id").cloned().collect()),
        other => other.clone(),
    };
    let mut errors = errors
        .iter()
        .map(|error| json!({"kind": error["kind"], "metas": metas(&error["metas"])}))
        .collect::<Vec<_>>();
    errors.sort_by_key(|error| error.to_string());
    errors
}

/// A query result, or the results of a batch, with what derives from where a directive is written masked: the ids of
/// transactions, and the main file `entry` a message names.
fn unplaced_body(body: &Value, entry: &str) -> Value {
    fn mask(result: &mut Value) {
        let Some(columns) = result.get("columns").and_then(Value::as_array) else {
            return;
        };
        let ids = columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column["name"] == "id")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if let Some(rows) = result.get_mut("rows").and_then(Value::as_array_mut) {
            for row in rows {
                for index in &ids {
                    if let Some(cell) = row.get_mut(*index) {
                        *cell = json!("<id>");
                    }
                }
            }
        }
    }
    let named = body.to_string().replace(&format!("({entry})"), "(<main file>)");
    let mut body: Value = serde_json::from_str(&named).expect("JSON reads back");
    match body.get_mut("data") {
        Some(Value::Array(results)) => results.iter_mut().for_each(mask),
        Some(result) => mask(result),
        None => {}
    }
    body
}

/// Whether `text`, a directive as zhang writes it, is one of the budget's contract: a budget directive (`budget`,
/// `budget-add`, `budget-transfer`, `budget-close` after the date and time) or an `open` with a `budget` metadata
/// entry. The text decides, not the parsed form, which the migration may change.
fn is_budget_text(text: &str) -> bool {
    let mut lines = text.lines();
    let keyword = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .find(|word| !word.starts_with(|c: char| c.is_ascii_digit()));
    match keyword {
        Some("budget" | "budget-add" | "budget-transfer" | "budget-close") => true,
        Some("open") => lines.any(|line| line.trim_start().starts_with("budget:")),
        _ => false,
    }
}

/// `directives` written as zhang text, and as beancount text, those whose zhang text `keep` keeps.
fn written(directives: &[Spanned<Directive>], keep: impl Fn(&str) -> bool) -> (String, String) {
    let (mut zhang, mut beancount) = (String::new(), String::new());
    for directive in directives {
        let text = ZhangDataType {}.export(directive.clone());
        if keep(&text) {
            zhang.push_str(text.trim_end());
            zhang.push('\n');
            beancount.push_str(Beancount::default().export(directive.clone()).trim_end());
            beancount.push('\n');
        }
    }
    (zhang, beancount)
}

/// The directives of the ledger `dir/entry` as its format reads them, file after file in the order the load reads
/// them, without the `include`s and the comments: what a round trip writes as one main file.
fn parsed_directives(dir: &Path, entry: &str) -> Vec<Spanned<Directive>> {
    let loaded = data_source_for(entry)
        .load(dir.to_string_lossy().into_owned(), entry.to_owned())
        .unwrap_or_else(|error| panic!("{}/{entry}: {error}", dir.display()));
    loaded
        .directives
        .into_iter()
        .filter(|it| !matches!(it.data, Directive::Include(_) | Directive::Comment(_)))
        .collect()
}

/// The answers of a server: its errors and the status and body of each request.
struct Answers {
    errors: Vec<Value>,
    responses: Vec<(u16, Value)>,
}

async fn answers(router: &Router, requests: &[Value]) -> Answers {
    let mut responses = vec![];
    for request in requests {
        responses.push(answer(router, request).await);
    }
    Answers {
        errors: errors(router).await,
        responses,
    }
}

/// What a round trip through `text`, written as the main file `main`, answers, against the `original` answers of the
/// ledger whose main file is `entry`.
async fn round_trip(entry: &str, main: &str, text: &str, now: &Clock, requests: &[Value], original: &Answers, budgets: Budgets) -> Value {
    let scratch = Scratch::with_main(main, text);
    let router = match server(scratch.dir(), main, *now, budgets).await {
        Ok((router, _)) => router,
        Err(error) => {
            // the line the error names, as written
            let line = error
                .split_once("at line ")
                .and_then(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next()?.parse::<usize>().ok())
                .and_then(|line| text.lines().nth(line.checked_sub(1)?));
            return json!({ "load_error": error, "line": line });
        }
    };
    let again = answers(&router, requests).await;
    let mut differences = Map::new();
    if unplaced(&again.errors) != unplaced(&original.errors) {
        differences.insert("errors".to_owned(), Value::Array(unplaced(&again.errors)));
    }
    let responses = requests
        .iter()
        .zip(original.responses.iter().zip(&again.responses))
        .enumerate()
        .filter(|(_, (_, ((status, body), (again_status, again_body))))| {
            status != again_status || unplaced_body(body, entry) != unplaced_body(again_body, main)
        })
        .map(|(index, (request, (_, (status, body))))| json!({"index": index, "request": request, "status": status, "body": body}))
        .collect::<Vec<_>>();
    if !responses.is_empty() {
        differences.insert("responses".to_owned(), Value::Array(responses));
    }
    if differences.is_empty() {
        json!("same")
    } else {
        Value::Object(differences)
    }
}

/// Compare `actual` with the golden file at `path`: with the server's budgets as `golden::assert_text` does (rewriting it
/// when asked to), with the others never rewriting it.
fn compare(budgets: Budgets, path: &Path, actual: &str) {
    match budgets {
        Budgets::Default => golden::assert_text(path, actual),
        Budgets::Native | Budgets::Script => {
            let expected = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let file = path.strip_prefix(cases_dir()).unwrap_or(path).display().to_string();
            let found = match path.extension().and_then(|it| it.to_str()) {
                Some("json") => {
                    let expected: Value = serde_json::from_str(&expected).unwrap_or_else(|error| panic!("{file}: {error}"));
                    let actual: Value = serde_json::from_str(actual).expect("the rendered answers are JSON");
                    zhang_testkit::budget_parity::differences("", &expected, &actual)
                }
                _ => expected
                    .lines()
                    .zip(actual.lines())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .map(|(line, (a, b))| zhang_testkit::budget_parity::Difference {
                        path: format!("line {}", line + 1),
                        expected: json!(a),
                        actual: json!(b),
                        allowed: None,
                    })
                    .collect(),
            };
            let differences = match budgets {
                Budgets::Native => &NATIVE_DIFFERENCES,
                _ => &SCRIPT_DIFFERENCES,
            };
            differences.lock().unwrap().extend(found.into_iter().map(|it| (file.clone(), it)));
        }
        Budgets::Analyzer => {
            let expected = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            if expected != actual {
                let line = expected.lines().zip(actual.lines()).position(|(a, b)| a != b).unwrap_or(0);
                panic!(
                    "{} differs as an analyzer computes the budgets, first at line {}:\n  golden:   {}\n  analyzer: {}",
                    path.display(),
                    line + 1,
                    expected.lines().nth(line).unwrap_or_default(),
                    actual.lines().nth(line).unwrap_or_default()
                );
            }
        }
    }
}

/// Run the case in `budget_contract/<name>` and compare what the server answers with its golden files.
async fn run(name: &str) {
    run_with(name, Budgets::Default).await
}

/// A case read from `budget_contract/<name>`: what `case.json` says, a copy of its ledger with the case's timezone,
/// the main files it loads, and its clock.
struct Case {
    case: CaseFile,
    copy: Scratch,
    entries: Vec<String>,
    now: Clock,
}

/// The case in `budget_contract/<name>`.
fn read_case(name: &str) -> Case {
    let dir = cases_dir().join(name);
    let case_path = dir.join("case.json");
    let case: CaseFile = serde_json::from_value(golden::read_json(&case_path)).unwrap_or_else(|error| panic!("{}: {error}", case_path.display()));
    let ledger_dir = match &case.ledger {
        Some(relative) => repo_root().join(relative),
        None => dir.clone(),
    };
    let ledger_dir = ledger_dir
        .canonicalize()
        .unwrap_or_else(|error| panic!("{name}: {}: {error}", ledger_dir.display()));
    let entries = case.entries.clone().unwrap_or_else(|| {
        ["main.zhang", "main.bean"]
            .into_iter()
            .filter(|entry| ledger_dir.join(entry).is_file())
            .map(str::to_owned)
            .collect()
    });
    assert!(!entries.is_empty(), "{name}: no main file in {}", ledger_dir.display());
    // a copy of the ledger, with the case's timezone appended to its main files
    let copy = Scratch::copy_of_dir(&ledger_dir, &entries[0]);
    if let Some(timezone) = &case.timezone {
        for entry in &entries {
            let text = copy.read(entry);
            copy.write(entry, &format!("{text}\noption \"timezone\" \"{timezone}\"\n"));
        }
    }
    let instant = chrono::DateTime::parse_from_rfc3339(&case.now).unwrap_or_else(|error| panic!("{name}: now {}: {error}", case.now));
    let now = Clock::Fixed(instant.to_utc());
    Case { case, copy, entries, now }
}

/// [`run`], the budgets computed by `budgets`.
async fn run_with(name: &str, budgets: Budgets) {
    let dir = cases_dir().join(name);
    let Case { case, copy, entries, now } = read_case(name);
    let ledger_dir = copy.dir().to_path_buf();
    let requests = case.all_requests();

    for entry in &entries {
        let (router, ledger) = server(&ledger_dir, entry, now, budgets)
            .await
            .unwrap_or_else(|error| panic!("{name}/{entry}: the ledger does not load: {error}"));
        let original = answers(&router, &requests).await;
        let error_list = error_list(&*ledger.read().await, &ledger_dir);
        let directives = parsed_directives(&ledger_dir, entry);
        let has_timezone = directives
            .iter()
            .any(|it| matches!(&it.data, Directive::Option(option) if option.key.as_str() == "timezone"));
        assert!(
            has_timezone,
            "{name}/{entry}: the ledger sets no timezone, so it would take the system's; set one in the ledger or in case.json"
        );

        // the budget directives, as each format writes them
        let (as_zhang, as_beancount) = written(&directives, is_budget_text);
        let export = format!(
            "; the budget directives of {entry} and the opens linking accounts to budgets, as written, file after file\n; ---- written as zhang\n{as_zhang}; ---- written as beancount\n{as_beancount}"
        );
        compare(budgets, &dir.join(format!("export.{entry}.txt")), &export);

        // every directive, written in each format and read again
        let (all_zhang, all_beancount) = written(&directives, |_| true);
        let mut round_trips = Map::new();
        round_trips.insert(
            "zhang".to_owned(),
            round_trip(entry, "main.zhang", &all_zhang, &now, &requests, &original, budgets).await,
        );
        round_trips.insert(
            "beancount".to_owned(),
            round_trip(entry, "main.bean", &all_beancount, &now, &requests, &original, budgets).await,
        );

        let responses = requests
            .iter()
            .zip(&original.responses)
            .map(|(request, (status, body))| json!({"request": request, "status": status, "body": body}))
            .collect::<Vec<_>>();
        let mut expected = json!({
            "case": name,
            "description": case.description,
            "ledger": format!("{}/{entry}", case.ledger.clone().unwrap_or_else(|| format!("zhang-server/tests/golden/budget_contract/{name}"))),
            "now": case.now,
            "errors": original.errors,
            "error_list": error_list,
            "responses": responses,
            "round_trip": round_trips,
        });
        if !case.known_issues.is_empty() {
            expected["known_issues"] = Value::Object(case.known_issues.clone());
        }
        compare(budgets, &dir.join(format!("expected.{entry}.json")), &render(&expected));
    }

    // a golden file of an entry the case no longer runs would go stale unseen
    for file in std::fs::read_dir(&dir).expect("the case directory") {
        let file = file.expect("a directory entry").file_name().to_string_lossy().into_owned();
        let of_entry = file
            .strip_prefix("expected.")
            .and_then(|rest| rest.strip_suffix(".json"))
            .or_else(|| file.strip_prefix("export.").and_then(|rest| rest.strip_suffix(".txt")));
        if let Some(entry) = of_entry {
            assert!(
                entries.iter().any(|it| it == entry),
                "{name}: {file} is the golden file of {entry}, which the case does not run; delete it"
            );
        }
    }
}

// ---- the golden files' layout ----------------------------------------------------------------------------------

/// The widest line [`render`] writes a value on whole.
const WIDTH: usize = 120;

/// The widest line [`render`] writes a row on whole: an array of plain values and flat objects, such as a row of a
/// query result, its columns, or the metas of an error.
const ROW_WIDTH: usize = 400;

/// The fields [`render`] writes first, in this order, so that a request reads before its answer and an amount as
/// `number currency`; the others follow by name.
const FIELD_ORDER: &[&str] = &[
    "case",
    "description",
    "ledger",
    "now",
    "known_issues",
    "errors",
    "error_list",
    "responses",
    "round_trip",
    "load_error",
    "index",
    "id",
    "id_of",
    "span",
    "request",
    "builtin",
    "batch",
    "bql",
    "method",
    "uri",
    "note",
    "status",
    "body",
    "data",
    "message",
    "kind",
    "file",
    "start",
    "end",
    "line",
    "metas",
    "name",
    "type",
    "params",
    "columns",
    "rows",
    "total",
    "key",
    "value",
    "number",
    "currency",
];

/// `value` as JSON, a value on one line when it fits in [`WIDTH`] columns (a row in [`ROW_WIDTH`]), otherwise one item
/// or field per line, the fields of an object in [`FIELD_ORDER`]. Only the layout is chosen: the text reads back as
/// the same JSON value.
fn render(value: &Value) -> String {
    let mut out = String::new();
    render_at(value, 0, &mut out);
    out.push('\n');
    out
}

/// The fields of an object, in [`FIELD_ORDER`].
fn ordered(fields: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut fields = fields.iter().collect::<Vec<_>>();
    let rank = |key: &str| FIELD_ORDER.iter().position(|it| *it == key).unwrap_or(FIELD_ORDER.len());
    fields.sort_by(|(a, _), (b, _)| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    fields
}

/// Whether `value` is a row: an array of plain values and of objects of plain values.
fn is_row(value: &Value) -> bool {
    let plain = |value: &Value| !value.is_array() && !value.is_object();
    let flat = |value: &Value| plain(value) || value.as_object().is_some_and(|fields| fields.values().all(plain));
    value.as_array().is_some_and(|items| items.iter().all(flat))
}

fn render_at(value: &Value, indent: usize, out: &mut String) {
    let inline = inline(value);
    let nested = match value {
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        _ => false,
    };
    let width = indent + inline.chars().count();
    if !nested || width <= WIDTH || (is_row(value) && width <= ROW_WIDTH) {
        out.push_str(&inline);
        return;
    }
    let pad = " ".repeat(indent + 2);
    match value {
        Value::Array(items) => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&pad);
                render_at(item, indent + 2, out);
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            let _ = write!(out, "{}]", " ".repeat(indent));
        }
        Value::Object(fields) => {
            out.push_str("{\n");
            let fields = ordered(fields);
            for (index, (key, field)) in fields.iter().enumerate() {
                let _ = write!(out, "{pad}{}: ", Value::String((*key).clone()));
                render_at(field, indent + 2, out);
                out.push_str(if index + 1 < fields.len() { ",\n" } else { "\n" });
            }
            let _ = write!(out, "{}}}", " ".repeat(indent));
        }
        _ => unreachable!("only arrays and objects nest"),
    }
}

/// `value` as JSON on one line, with a space after each comma and colon, the fields of an object in [`FIELD_ORDER`].
fn inline(value: &Value) -> String {
    match value {
        Value::Array(items) => format!("[{}]", items.iter().map(inline).collect::<Vec<_>>().join(", ")),
        Value::Object(fields) => format!(
            "{{{}}}",
            ordered(fields)
                .into_iter()
                .map(|(key, field)| format!("{}: {}", Value::String(key.clone()), inline(field)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        scalar => scalar.to_string(),
    }
}

// ---- the cases -------------------------------------------------------------------------------------------------

macro_rules! cases {
    ($($test:ident => $dir:literal,)*) => {
        /// the directory of every case
        const CASES: &[&str] = &[$($dir),*];
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $test() {
                run($dir).await
            }
        )*
    };
}

cases! {
    budget_contract_zhang_syntax => "zhang-syntax",
    budget_contract_beancount_syntax => "beancount-syntax",
    budget_contract_beancount_custom_syntax_500 => "beancount-custom-syntax-500",
    budget_contract_close_and_currency_499 => "close-and-currency-499",
    budget_contract_categories_in_an_included_file => "categories-in-an-included-file",
    budget_contract_timezone_and_inverse_prices => "timezone-and-inverse-prices",
    budget_contract_duplicate_and_undefined => "duplicate-and-undefined",
    budget_contract_activity_before_the_definition => "activity-before-the-definition",
    budget_contract_linked_accounts => "linked-accounts",
    budget_contract_carry_over_and_negative => "carry-over-and-negative",
    budget_contract_close => "close",
    budget_contract_same_day_order => "same-day-order",
    budget_contract_same_day_order_beancount => "same-day-order-beancount",
    budget_contract_booked_postings => "booked-postings",
    budget_contract_errors_at_one_place => "errors-at-one-place",
    budget_contract_same_day_order_across_files => "same-day-order-across-files",
    budget_contract_months_of_date_typos => "months-of-date-typos",
    budget_contract_months_over_decades => "months-over-decades",
    budget_contract_beancount_included_budgets => "beancount-included-budgets",
    budget_contract_currencies => "currencies",
    budget_contract_months => "months",
    budget_contract_months_of_a_past_clock => "months-of-a-past-clock",
    budget_contract_months_and_a_date_typo => "months-and-a-date-typo",
    budget_contract_lots_of_two_cost_currencies => "lots-of-two-cost-currencies",
    budget_contract_many_significant_digits => "many-significant-digits",
}

/// Every case, with the budget analyzer of the SDK's examples computing the budgets instead of the native code: the
/// same golden files, ids and places of the errors included.
#[tokio::test(flavor = "multi_thread")]
async fn budget_contract_as_an_analyzer() {
    if budget_analyzer().is_none() {
        return;
    }
    for case in CASES {
        run_with(case, Budgets::Analyzer).await;
    }
    budget_contract_schema_with(Budgets::Analyzer).await;
}

/// A budget closed at a time the ledger's timezone skips (02:45 on 2024-03-10 in New York, whose clocks go from 02:00
/// to 03:00) closes at that time as written: its `close_time` is `02:45:00`, the posting at 03:30 is after the close
/// (`BudgetClosed`, no activity), its event's time is the instant zhang reads, 03:45. The analyzers, the SDK's and the
/// bundled script, answer as the native budgets do. #835 has no such case; RFC #698's differential fuzzer found it.
#[tokio::test(flavor = "multi_thread")]
async fn a_close_at_a_time_the_timezone_skips_closes_as_written() {
    const LEDGER: &str = "option \"operating_currency\" \"CNY\"\noption \"timezone\" \"America/New_York\"\n\
                          1970-01-01 commodity CNY\n2024-01-01 open Assets:Bank\n2024-01-01 open Expenses:Food\n  budget: Food\n\
                          2024-01-01 budget Food CNY\n2024-03-01 budget-add Food 100 CNY\n\
                          2024-03-10 01:30:00 * \"A\" \"before the close\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n\
                          2024-03-10 02:45:00 budget-close Food\n\
                          2024-03-10 03:30:00 * \"B\" \"after the close\"\n  Expenses:Food 2 CNY\n  Assets:Bank\n";
    let requests = [
        json!({"bql": "SELECT name, close, close_time FROM #budget_definitions"}),
        json!({"bql": "SELECT name, date, time, timestamp, type FROM #budget_events"}),
        json!({"bql": "SELECT name, date, assigned, activity, available, closed FROM #budgets WHERE date <= 2024-03-01"}),
        json!({"bql": "SELECT narration, budgets FROM #postings WHERE account = 'Expenses:Food'"}),
    ];
    let scratch = Scratch::zhang(LEDGER);
    let now = Clock::Fixed(chrono::DateTime::parse_from_rfc3339("2024-03-20T12:00:00Z").unwrap().to_utc());
    let mut runs = vec![Budgets::Native, Budgets::Script, Budgets::Default];
    if budget_analyzer().is_some() {
        runs.push(Budgets::Analyzer);
    }
    let mut seen = vec![];
    for budgets in runs {
        let (router, ledger) = server(scratch.dir(), "main.zhang", now, budgets).await.expect("the ledger loads");
        let errors = error_list(&*ledger.read().await, scratch.dir());
        seen.push((answers(&router, &requests).await.responses, errors));
    }
    let (responses, errors) = &seen[0];
    assert_eq!(
        rows_by_column(&responses[0].1["data"])[0]["close_time"],
        json!("02:45:00"),
        "{:?}",
        responses[0]
    );
    assert_eq!(rows_by_column(&responses[1].1["data"])[1]["time"], json!("03:45:00"), "{:?}", responses[1]);
    assert_eq!(
        rows_by_column(&responses[2].1["data"])[2]["activity"]["number"],
        json!("1"),
        "{:?}",
        responses[2]
    );
    let kinds = errors.iter().map(|it| it["kind"].as_str().unwrap_or_default()).collect::<Vec<_>>();
    assert_eq!(kinds, ["BudgetClosed"], "{errors:?}");
    for other in &seen[1..] {
        assert_eq!(other, &seen[0]);
    }
}

/// Every case, with the bundled script `zhang:budget` computing the budgets whatever the build (RFC #698's `plugin`
/// mode): the same golden files, ids and places of the errors included, but for the differences the testing guide
/// allows, which are listed. The list is also written to `budget-contract-script.txt` in the test's temporary
/// directory.
#[tokio::test(flavor = "multi_thread")]
async fn budget_contract_as_the_budget_script() {
    budget_contract_compared(Budgets::Script, &SCRIPT_DIFFERENCES, "script").await
}

/// Every case, with zhang's native budget code computing the budgets through RFC #698's test switch (its `native`
/// mode), until P5 deletes it: as [`budget_contract_as_the_budget_script`], the list written to
/// `budget-contract-native.txt`.
// TODO(P5): goes with the native budget code
#[tokio::test(flavor = "multi_thread")]
async fn budget_contract_natively() {
    budget_contract_compared(Budgets::Native, &NATIVE_DIFFERENCES, "native").await
}

/// Every case and the schema with the budgets computed by `budgets`, compared with the golden files by value: each
/// difference found, collected in `differences`, must be one the testing guide allows. The list is written to
/// `budget-contract-<name>.txt` in the test's temporary directory.
async fn budget_contract_compared(budgets: Budgets, differences: &Differences, name: &str) {
    for case in CASES {
        run_with(case, budgets).await;
    }
    budget_contract_schema_with(budgets).await;
    let found = std::mem::take(&mut *differences.lock().unwrap());
    let report = found.iter().map(|(file, difference)| format!("{file} {difference}\n")).collect::<String>();
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("budget-contract-{name}.txt"));
    std::fs::write(&path, &report).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let allowed = found.iter().filter(|(_, it)| it.allowed.is_some()).count();
    eprintln!("{} differences with the golden files, {allowed} of them allowed:\n{report}", found.len());
    assert_eq!(
        allowed,
        found.len(),
        "the {name} budgets differ from the golden files where the testing guide allows no difference:\n{report}"
    );
}

/// The bundled script `zhang:budget` declares what the native provider declares, descriptions and BQL included: the
/// manifest the SDK's budget analyzer reads, which a test of zhang-query checks against the native provider.
#[test]
fn the_budget_script_declares_the_native_manifest() {
    let scratch = Scratch::zhang("1970-01-01 open Assets:Cash\n  budget: \"Food\"\n");
    let options = load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Plugin);
    let ledger = Ledger::load_with_options(
        scratch.dir().to_path_buf(),
        "main.zhang".to_owned(),
        data_source_for("main.zhang"),
        Clock::System,
        options,
    )
    .expect("the ledger loads");
    let script = ledger
        .plugins
        .analyzers()
        .find(|plugin| plugin.is_budget_provider())
        .expect("zhang:budget is registered");
    assert!(script.script().is_some(), "zhang:budget is the bundled script");
    let declared = serde_json::to_value(script.analyzer_manifest().expect("its manifest holds")).unwrap();
    let native: zhang_shared::plugin_abi::analysis::Manifest =
        serde_json::from_str(include_str!("../../../zhang-plugin-sdk/examples/budget/manifest.json")).expect("the native manifest");
    assert_eq!(declared, serde_json::to_value(native).unwrap());
}

/// A `zhang:budget` that declares the budget errors and whose analysis traps: the WAT text of a module whose `manifest`
/// answers so and whose `analyze` export traps.
fn failing_budget_analyzer() -> String {
    let manifest = json!({
        "analysis_abi": 1,
        "consumes": ["budget", "budget-add", "budget-transfer", "budget-close"],
        "open_meta": ["budget"],
        "diagnostics": [{"kind": "BudgetDoesNotExist"}, {"kind": "DefineDuplicatedBudget"}, {"kind": "BudgetClosed"},
                        {"kind": "BudgetCommodityMismatch", "phase": "after_fold"}]
    })
    .to_string();
    let exports = [
        ("name", "\"zhang:budget\"".to_owned()),
        ("version", "\"0.1.0\"".to_owned()),
        ("supported_type", "[\"Analyzer\"]".to_owned()),
        ("manifest", manifest),
    ];
    let mut wat = String::from(
        "(module\n\
         (import \"extism:host/env\" \"alloc\" (func $alloc (param i64) (result i64)))\n\
         (import \"extism:host/env\" \"store_u8\" (func $store_u8 (param i64 i32)))\n\
         (import \"extism:host/env\" \"output_set\" (func $output_set (param i64 i64)))\n\
         (memory 1)\n\
         (func $output_bytes (param $ptr i32) (param $len i32) (local $block i64) (local $i i32)\n\
           (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))\n\
           (block $done (loop $copy\n\
             (br_if $done (i32.ge_u (local.get $i) (local.get $len)))\n\
             (call $store_u8 (i64.add (local.get $block) (i64.extend_i32_u (local.get $i))) (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))\n\
             (local.set $i (i32.add (local.get $i) (i32.const 1)))\n\
             (br $copy)))\n\
           (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))\n\
         (func (export \"analyze\") (result i32) unreachable)\n",
    );
    let mut offset = 0;
    for (export, text) in exports {
        let escaped = text.bytes().map(|byte| format!("\\{byte:02x}")).collect::<String>();
        wat.push_str(&format!(
            "(data (i32.const {offset}) \"{escaped}\")\n(func (export \"{export}\") (result i32) (call $output_bytes (i32.const {offset}) (i32.const {})) (i32.const 0))\n",
            text.len()
        ));
        offset += text.len() + 16;
    }
    assert!(offset < 65536, "the module's data fits its one page");
    wat.push_str(")\n");
    wat
}

/// Every case, with a `zhang:budget` that reports the budget errors and whose analysis fails, in place of the bundled
/// script: the script is the only implementation of budgets, so the load has its one `PluginAnalysisFailed`, on the
/// first use of budgets, and no budget error. Every other error is the native load's, ids, places and order included.
#[tokio::test(flavor = "multi_thread")]
async fn budget_contract_has_one_error_and_no_budget_error_when_zhang_budget_fails() {
    let module: Arc<[u8]> = Arc::from(failing_budget_analyzer().into_bytes());
    let budget_kinds = ["BudgetDoesNotExist", "DefineDuplicatedBudget", "BudgetCommodityMismatch", "BudgetClosed"];
    let without = |errors: Vec<Value>, kinds: &[&str]| {
        errors
            .into_iter()
            .filter(|error| !kinds.contains(&error["kind"].as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
    };
    let mut failed_loads = 0;
    for name in CASES {
        let Case { copy, entries, now, .. } = read_case(name);
        let dir = copy.dir().to_path_buf();
        for entry in &entries {
            let natively = load_options_for(Arc::new(PluginPolicy::default()), BudgetImpl::Native);
            let native = Ledger::load_with_options(dir.clone(), entry.clone(), data_source_for(entry), now, natively).expect("the ledger loads");
            let options = with_budget_plugin("budget.wat", module.clone());
            let failed = Ledger::load_with_options(dir.clone(), entry.clone(), data_source_for(entry), now, options).expect("the ledger loads");
            assert_eq!(
                failed.plugins.analyzers().filter(|plugin| plugin.is_budget_provider()).count(),
                1,
                "{name}/{entry}: one zhang:budget, the module"
            );
            assert!(failed.plugins.reports_budget_errors(), "{name}/{entry}: the module stands for zhang:budget");
            let failures = failed
                .errors
                .iter()
                .filter(|error| error.error_type == zhang_ast::error::ErrorKind::PluginAnalysisFailed)
                .collect::<Vec<_>>();
            let status = &failed
                .extensions
                .analyzer("zhang:budget")
                .and_then(|slot| slot.snapshot.get())
                .expect("it ran or was skipped")
                .status;
            match status {
                SnapshotStatus::Failed { .. } => {
                    assert_eq!(failures.len(), 1, "{name}/{entry}");
                    failed_loads += 1;
                    // the module declares no table: the budget tables of a failed analysis of the script, which
                    // declares them, are checked by budget_parity's out-of-memory test
                }
                SnapshotStatus::Skipped => assert_eq!(failures.len(), 0, "{name}/{entry}"),
                SnapshotStatus::Ok => panic!("{name}/{entry}: the analysis traps"),
            }
            let errors = error_list(&failed, &dir);
            assert_eq!(
                without(errors.clone(), &["PluginAnalysisFailed"]),
                without(error_list(&native, &dir), &budget_kinds),
                "{name}/{entry}: no budget error, and every other error of the native load"
            );
            assert!(
                errors.iter().all(|error| !budget_kinds.contains(&error["kind"].as_str().unwrap_or_default())),
                "{name}/{entry}: no budget error"
            );
        }
    }
    assert!(
        failed_loads >= CASES.len(),
        "the analysis failed in {failed_loads} loads: every case has budgets"
    );
}

/// Every directory of `budget_contract/` is a case one of the tests above runs.
#[test]
fn budget_contract_cases_are_every_directory() {
    let mut dirs = std::fs::read_dir(cases_dir())
        .expect("the cases directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    dirs.sort();
    let mut cases = CASES.iter().map(|it| (*it).to_owned()).collect::<Vec<_>>();
    cases.sort();
    assert_eq!(dirs, cases, "add a test to `cases!` for each case directory");
}

/// The budget tables' columns and the built-in queries of the budget pages and the app, with their parameters and
/// columns: the names and types clients rely on, without the descriptions and the BQL behind them.
#[tokio::test(flavor = "multi_thread")]
async fn budget_contract_schema() {
    budget_contract_schema_with(Budgets::Default).await
}

async fn budget_contract_schema_with(budgets: Budgets) {
    let scratch = Scratch::zhang("1970-01-01 commodity CNY\n");
    let (router, _) = server(scratch.dir(), "main.zhang", Clock::Fixed(chrono::DateTime::UNIX_EPOCH), budgets)
        .await
        .expect("an empty ledger loads");
    fn without(value: &Value, keys: &[&str]) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .filter(|(key, _)| !keys.contains(&key.as_str()))
                    .map(|(key, field)| (key.clone(), without(field, keys)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|item| without(item, keys)).collect()),
            scalar => scalar.clone(),
        }
    }
    let (status, schema) = answer(&router, &json!({"method": "GET", "uri": "/api/query/schema"})).await;
    assert_eq!(status, 200, "{schema}");
    // the budget tables, and the `budgets` column of `#postings`, which the budget page's posting list reads
    let tables = schema["data"]["tables"]
        .as_array()
        .unwrap_or_else(|| panic!("the tables of the schema: {schema}"))
        .iter()
        .filter_map(|table| match table["name"].as_str().unwrap_or_default() {
            "budgets" | "budget_definitions" | "budget_events" => Some(without(table, &["description"])),
            "postings" => {
                let mut table = without(table, &["description"]);
                if let Some(columns) = table.get_mut("columns").and_then(Value::as_array_mut) {
                    columns.retain(|column| column["name"] == "budgets");
                }
                Some(table)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 4, "{schema}");
    let (status, builtins) = answer(&router, &json!({"method": "GET", "uri": "/api/query/builtins"})).await;
    assert_eq!(status, 200, "{builtins}");
    let builtins = builtins["data"]
        .as_array()
        .unwrap_or_else(|| panic!("the built-in queries: {builtins}"))
        .iter()
        .filter(|query| {
            let name = query["name"].as_str().unwrap_or_default();
            name.starts_with("budgets.") || name == "ledger.now" || name == "journals.errors"
        })
        .map(|query| without(query, &["description", "bql"]))
        .collect::<Vec<_>>();
    compare(
        budgets,
        &cases_dir().join("schema.json"),
        &render(&json!({"tables": tables, "builtin_queries": builtins})),
    );
}
