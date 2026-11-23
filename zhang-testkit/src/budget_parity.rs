//! The budgets of a ledger computed twice and compared (RFC #698's `both` mode): with zhang's native budget code, and
//! with the bundled script `zhang:budget` (`plugins/budget/budget.lua`) as the default plugin, each served as the
//! server serves a ledger. What a client sees is compared: the ledger's errors id-exact, the budget tables, the
//! `budgets` of `#postings`, and the budget pages' named queries.
//!
//! A difference is classified by the rules of the testing guide ("The budget contract", allowed differences): the scale
//! of a decimal (`500.00` against `500`, the same value) is allowed, and so is the order of the load's errors across
//! places; anything else is not.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use axum::http::Method;
use axum::Router;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::clock::Clock;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::PluginPolicy;
use zhang_server::analysis::{load_options_for, BudgetImpl};

use crate::fixtures::data_source_for;
use crate::http::{app, call, router_of, serve_config, Settings};

/// A difference between what the native budgets and the plugin's answer.
#[derive(Debug, Clone)]
pub struct Difference {
    /// where: the request, then the JSON path in its answer
    pub path: String,
    pub native: Value,
    pub plugin: Value,
    /// why the testing guide allows it, or `None` for a regression
    pub allowed: Option<&'static str>,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.allowed {
            Some(why) => format!("allowed ({why})"),
            None => "NOT ALLOWED".to_owned(),
        };
        write!(f, "{}: native {} / plugin {}: {verdict}", self.path, short(&self.native), short(&self.plugin))
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

/// Every difference between `native` and `plugin`, by JSON path under `path`: objects by key, whatever the order of
/// their fields, arrays by position.
pub fn differences(path: &str, native: &Value, plugin: &Value) -> Vec<Difference> {
    let mut found = vec![];
    diff(path, native, plugin, &mut found);
    found
}

fn diff(path: &str, native: &Value, plugin: &Value, found: &mut Vec<Difference>) {
    match (native, plugin) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                diff(&format!("{path}.{key}"), value, b.get(key).unwrap_or(&Value::Null), found);
            }
            for (key, value) in b.iter().filter(|(key, _)| !a.contains_key(*key)) {
                diff(&format!("{path}.{key}"), &Value::Null, value, found);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}[{index}]"), a, b, found);
            }
        }
        (a, b) if a == b => {}
        (a, b) => {
            let allowed = match (decimal(a), decimal(b)) {
                (Some(a), Some(b)) if a == b => Some("the scale of a decimal"),
                _ => None,
            };
            found.push(Difference {
                path: path.to_owned(),
                native: a.clone(),
                plugin: b.clone(),
                allowed,
            });
        }
    }
}

/// A ledger served as the server serves it, its budgets computed by `budgets`, and the ledger. With the plugin, the
/// script must have published its snapshot ([`published`]): a plugin run that fell back to the native checks compares
/// nothing.
pub async fn serve(dir: &Path, entry: &str, clock: Clock, budgets: BudgetImpl) -> Result<(Router, Arc<RwLock<Ledger>>), String> {
    let source = data_source_for(entry);
    let options = load_options_for(Arc::new(PluginPolicy::default()), budgets);
    let (root, main, data_source) = (dir.to_path_buf(), entry.to_owned(), source.clone());
    let ledger = tokio::task::spawn_blocking(move || Ledger::load_with_options(root, main, data_source, clock, options))
        .await
        .expect("the load runs")
        .map_err(|error| error.to_string())?;
    if budgets != BudgetImpl::Native {
        published(&ledger)?;
    }
    let ledger = Arc::new(RwLock::new(ledger));
    let router = router_of(app(serve_config(dir, entry, source, &Settings::default()), ledger.clone())).await;
    Ok((router, ledger))
}

/// Whether `ledger`'s budgets come from the script `zhang:budget`: it ran, did not fail, and published its snapshot (or
/// had nothing to read); what went wrong otherwise.
pub fn published(ledger: &Ledger) -> Result<(), String> {
    let failed = ledger
        .errors
        .iter()
        .filter(|error| error.error_type == zhang_ast::error::ErrorKind::PluginAnalysisFailed)
        .map(|error| format!("{:?}", error.metas))
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        return Err(format!("zhang:budget failed: {failed:?}"));
    }
    if !ledger
        .plugins
        .analyzers()
        .any(|plugin| plugin.is_budget_provider() && plugin.script().is_some())
    {
        return Err("the budget script is not the default plugin zhang:budget".to_owned());
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
    /// how many errors the native load reports, and of which kinds
    pub errors: usize,
    pub kinds: std::collections::BTreeSet<String>,
    pub differences: Vec<Difference>,
}

impl Report {
    pub fn regressions(&self) -> impl Iterator<Item = &Difference> {
        self.differences.iter().filter(|it| it.allowed.is_none())
    }
}

/// The ledger `dir/entry` loaded with `clock` natively and with the plugin, compared: its errors, then the answers to
/// `requests`.
pub async fn compare(dir: &Path, entry: &str, clock: Clock, requests: &[Value]) -> Report {
    compare_until(dir, entry, clock, requests, false).await.expect("the ledger loads")
}

/// [`compare`], stopping at the first difference the testing guide does not allow when `first` is set; an error when the
/// ledger does not load (both ways).
pub async fn compare_until(dir: &Path, entry: &str, clock: Clock, requests: &[Value], first: bool) -> Result<Report, String> {
    let mut report = Report::default();
    let loads = (
        serve(dir, entry, clock, BudgetImpl::Native).await,
        serve(dir, entry, clock, BudgetImpl::Plugin).await,
    );
    let ((native, native_ledger), (plugin, plugin_ledger)) = match loads {
        (Ok(native), Ok(plugin)) => (native, plugin),
        (Err(native), Err(plugin)) if native == plugin => return Err(native),
        (native, plugin) => {
            let outcome = |load: Result<_, String>| load.map_or_else(Value::from, |_| json!("loaded"));
            report.differences.push(Difference {
                path: "the load".to_owned(),
                native: outcome(native),
                plugin: outcome(plugin),
                allowed: None,
            });
            return Ok(report);
        }
    };

    // the errors id-exact, in the load's order; an order that differs only across places is allowed
    let (a, b) = (error_list(&*native_ledger.read().await, dir), error_list(&*plugin_ledger.read().await, dir));
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
            native: Value::from(a.len()),
            plugin: Value::from(b.len()),
            allowed: Some("the order in which the load reports errors at different places"),
        });
    }
    report.differences.extend(differences);
    report.kinds = a.iter().filter_map(|error| error["kind"].as_str().map(str::to_owned)).collect();

    for request in requests {
        if first && report.regressions().next().is_some() {
            break;
        }
        let (a, b) = (answer(&native, request).await, answer(&plugin, request).await);
        let path = request.to_string();
        report.differences.extend(differences_of_answers(&path, &a, &b));
        report.requests += 1;
    }
    Ok(report)
}

fn differences_of_answers(path: &str, native: &(u16, Value), plugin: &(u16, Value)) -> Vec<Difference> {
    let mut found = differences(&format!("{path} status"), &json!(native.0), &json!(plugin.0));
    found.extend(differences(&format!("{path} body"), &native.1, &plugin.1));
    found
}
