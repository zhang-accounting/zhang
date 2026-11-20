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
//!   with `budgets.budget_month`) and its activity of the month (`budgets.events` with `budgets.postings`), each
//!   batch as the app sends it;
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
//!   their kind, file, line and metas; the same errors id-exact, in the order the load reported them (the core's list,
//!   `Ledger::errors`, which the plugin's diagnostics join): id, kind, span (file, start, end, line) and every meta,
//!   `txn_id` included, with the id spelled out as the `(place, kind, n)` it derives from (the `n`-th error of its kind
//!   at its place, `FromSpan::of_error`); every request with the HTTP status and the JSON body of its answer; and the
//!   round trips: every
//!   directive of the ledger as its format reads it (but the `include`s and the comments), written in one format
//!   (`zhang`, `beancount`) as one main file, loaded with the same clock and asked the same requests. A round trip is
//!   `"same"` when every answer equals the original apart from what depends on where a directive is written (the ids
//!   of transactions, the file and line of errors, the main file a message names), otherwise it lists the errors and
//!   the answers that differ, or why the text does not load and the line it names;
//! - `export.<entry>.txt`: the budget directives and the `open`s linking accounts to budgets, as written, file after
//!   file, as the zhang and the beancount formats write them.
//!
//! `schema.json` holds the columns of the budget tables and the parameters and columns of the built-in queries the
//! budget pages and the app run, as `GET /api/query/schema` and `GET /api/query/builtins` list them.

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
use zhang_core::ledger::Ledger;
use zhang_core::utils::id::FromSpan;
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

/// A server on the ledger `dir/entry`, loaded in the format of `entry` with its clock at `now`, and the ledger it
/// serves; the load's error when it does not load.
async fn server(dir: &Path, entry: &str, now: Clock) -> Result<(Router, Arc<RwLock<Ledger>>), String> {
    let source = data_source_for(entry);
    let ledger = Ledger::load_with_clock(dir.to_path_buf(), entry.to_owned(), source.clone(), now).map_err(|error| error.to_string())?;
    let ledger = Arc::new(RwLock::new(ledger));
    let router = router_of(app(serve_config(dir, entry, source, &Settings::default()), ledger.clone())).await;
    Ok((router, ledger))
}

/// The errors of `ledger`, whose files are in `dir`, id-exact and in the order the load reported them: the id and
/// the `(place, kind, n)` it derives from, the kind, the span and every meta.
fn error_list(ledger: &Ledger, dir: &Path) -> Vec<Value> {
    let mut seen: HashMap<(Option<PathBuf>, usize, String), usize> = HashMap::new();
    ledger
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
        .collect()
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

/// The directives of a budget's contract: the budget directives and the `open`s with a `budget` metadata entry.
fn is_budget_directive(directive: &Directive) -> bool {
    match directive {
        Directive::Budget(_) | Directive::BudgetAdd(_) | Directive::BudgetTransfer(_) | Directive::BudgetClose(_) => true,
        Directive::Open(open) => !open.meta.get_all("budget").is_empty(),
        _ => false,
    }
}

/// `directives` written as zhang text, and as beancount text.
fn written(directives: &[&Spanned<Directive>]) -> (String, String) {
    let join = |texts: Vec<String>| texts.into_iter().map(|text| text.trim_end().to_owned() + "\n").collect::<String>();
    (
        join(directives.iter().map(|it| ZhangDataType {}.export((*it).clone())).collect()),
        join(directives.iter().map(|it| Beancount::default().export((*it).clone())).collect()),
    )
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
async fn round_trip(entry: &str, main: &str, text: &str, now: &Clock, requests: &[Value], original: &Answers) -> Value {
    let scratch = Scratch::with_main(main, text);
    let router = match server(scratch.dir(), main, *now).await {
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

/// Run the case in `budget_contract/<name>` and compare what the server answers with its golden files.
async fn run(name: &str) {
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
    let ledger_dir = copy.dir().to_path_buf();
    let instant = chrono::DateTime::parse_from_rfc3339(&case.now).unwrap_or_else(|error| panic!("{name}: now {}: {error}", case.now));
    let now = Clock::Fixed(instant.to_utc());
    let requests = case.all_requests();

    for entry in &entries {
        let (router, ledger) = server(&ledger_dir, entry, now)
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
        let budget_directives = directives.iter().filter(|it| is_budget_directive(&it.data)).collect::<Vec<_>>();
        let (as_zhang, as_beancount) = written(&budget_directives);
        let export = format!(
            "; the budget directives of {entry} and the opens linking accounts to budgets, as written, file after file\n; ---- written as zhang\n{as_zhang}; ---- written as beancount\n{as_beancount}"
        );
        golden::assert_text(&dir.join(format!("export.{entry}.txt")), &export);

        // every directive, written in each format and read again
        let (all_zhang, all_beancount) = written(&directives.iter().collect::<Vec<_>>());
        let mut round_trips = Map::new();
        round_trips.insert(
            "zhang".to_owned(),
            round_trip(entry, "main.zhang", &all_zhang, &now, &requests, &original).await,
        );
        round_trips.insert(
            "beancount".to_owned(),
            round_trip(entry, "main.bean", &all_beancount, &now, &requests, &original).await,
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
        golden::assert_text(&dir.join(format!("expected.{entry}.json")), &render(&expected));
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
    budget_contract_errors_order => "errors-order",
    budget_contract_currencies => "currencies",
    budget_contract_months => "months",
    budget_contract_months_of_a_past_clock => "months-of-a-past-clock",
    budget_contract_months_and_a_date_typo => "months-and-a-date-typo",
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
    let scratch = Scratch::zhang("1970-01-01 commodity CNY\n");
    let (router, _) = server(scratch.dir(), "main.zhang", Clock::Fixed(chrono::DateTime::UNIX_EPOCH))
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
    golden::assert_text(
        &cases_dir().join("schema.json"),
        &render(&json!({"tables": tables, "builtin_queries": builtins})),
    );
}
