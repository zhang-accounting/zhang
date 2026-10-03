//! Golden comparison of the budget and commodity endpoints: each handler that runs built-in
//! queries against the handler it replaces ([`budget::legacy`], [`commodity::legacy`]), on every
//! ledger of `integration-tests/` and `examples/`, in both formats where there are both (#479).
//!
//! A difference is accepted only when it is one of the documented ones (see [`Reason`]); anything
//! else fails. `golden_report` (ignored) runs the same comparison on more ledgers, given as
//! `ZHANG_GOLDEN_LEDGERS=dir[:dir...]`, and prints every difference with its reason and the
//! timings of both handlers.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{Datelike, Months, NaiveDate};
use serde::Serialize;
use serde_json::Value as Json;
use tokio::sync::RwLock;
use zhang_ast::Directive;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
use crate::response::ResponseWrapper;
use crate::routes::{budget, commodity};
use crate::state::SharedLedger;
use crate::ServerResult;

/// What an endpoint answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Json(Json),
    Status(u16),
    Panic,
}

impl Outcome {
    fn json(&self) -> Option<&Json> {
        match self {
            Outcome::Json(json) => Some(json),
            _ => None,
        }
    }
}

/// Run a handler, catching a panic as [`Outcome::Panic`], and time it.
async fn call<T, F>(handler: F) -> (Outcome, Duration)
where
    T: Serialize + gotcha::Schematic + Send + 'static,
    F: Future<Output = ServerResult<ResponseWrapper<T>>> + Send + 'static,
{
    let start = Instant::now();
    let outcome = match tokio::spawn(handler).await {
        Err(_) => Outcome::Panic,
        Ok(Ok(wrapper)) => Outcome::Json(serde_json::to_value(&wrapper.data).expect("serializable")),
        Ok(Err(err)) => Outcome::Status(err.into_response().status().as_u16()),
    };
    (outcome, start.elapsed())
}

/// The endpoints compared, with their arguments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Probe {
    BudgetList { month: Option<(u32, u32)> },
    BudgetInfo { name: String, month: Option<(u32, u32)> },
    BudgetInterval { name: String, year: u32, month: u32 },
    Commodities,
    Commodity { name: String },
}

impl Probe {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Probe::BudgetList { .. } => "GET /api/budgets",
            Probe::BudgetInfo { .. } => "GET /api/budgets/{name}",
            Probe::BudgetInterval { .. } => "GET /api/budgets/{name}/interval/{y}/{m}",
            Probe::Commodities => "GET /api/commodities",
            Probe::Commodity { .. } => "GET /api/commodities/{name}",
        }
    }

    fn month_of(month: &Option<(u32, u32)>) -> Option<NaiveDate> {
        month.and_then(|(year, month)| NaiveDate::from_ymd_opt(year as i32, month, 1))
    }

    /// the month the probe asks about, if it names one
    pub(crate) fn month(&self) -> Option<NaiveDate> {
        match self {
            Probe::BudgetList { month } | Probe::BudgetInfo { month, .. } => Probe::month_of(month),
            Probe::BudgetInterval { year, month, .. } => NaiveDate::from_ymd_opt(*year as i32, *month, 1),
            _ => None,
        }
    }

    fn request(month: &Option<(u32, u32)>) -> Query<BudgetListRequest> {
        Query(BudgetListRequest {
            year: month.map(|it| it.0),
            month: month.map(|it| it.1),
        })
    }

    /// the old and the new handler's answers, and how long each took
    pub(crate) async fn run(&self, ledger: &SharedLedger) -> ((Outcome, Duration), (Outcome, Duration)) {
        let state = || State(SharedLedger(ledger.0.clone()));
        match self.clone() {
            Probe::BudgetList { month } => (
                call(budget::legacy::get_budget_list(state(), Probe::request(&month))).await,
                call(budget::get_budget_list(state(), Probe::request(&month))).await,
            ),
            Probe::BudgetInfo { name, month } => (
                call(budget::legacy::get_budget_info(state(), Path((name.clone(),)), Probe::request(&month))).await,
                call(budget::get_budget_info(state(), Path((name,)), Probe::request(&month))).await,
            ),
            Probe::BudgetInterval { name, year, month } => {
                let path = || {
                    Path(BudgetIntervalDetailRequest {
                        budget_name: name.clone(),
                        year,
                        month,
                    })
                };
                (
                    call(budget::legacy::get_budget_interval_detail(state(), path())).await,
                    call(budget::get_budget_interval_detail(state(), path())).await,
                )
            }
            Probe::Commodities => (
                call(commodity::legacy::get_all_commodities(state())).await,
                call(commodity::get_all_commodities(state())).await,
            ),
            Probe::Commodity { name } => (
                call(commodity::legacy::get_single_commodity(state(), Path((name.clone(),)))).await,
                call(commodity::get_single_commodity(state(), Path((name,)))).await,
            ),
        }
    }
}

/// A ledger to compare on: its directory and main file.
#[derive(Debug, Clone)]
pub(crate) struct Fixture {
    pub(crate) dir: PathBuf,
    pub(crate) entry: String,
}

impl Fixture {
    pub(crate) fn name(&self) -> String {
        let dir = self.dir.file_name().map(|it| it.to_string_lossy().to_string()).unwrap_or_default();
        format!("{}/{}", dir, self.entry)
    }

    /// the ledger, or `None` if it cannot be loaded. The local data source does not expand wildcard
    /// includes and fails on a missing include, so a ledger that has either is loaded from a copy
    /// with its wildcards expanded and its missing includes empty ([`materialize`])
    pub(crate) async fn try_load(&self) -> Option<SharedLedger> {
        let load = |dir: PathBuf| async move {
            let source: Arc<dyn zhang_core::data_source::DataSource> = if self.entry.ends_with(".bean") {
                Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
            } else {
                Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
            };
            Ledger::async_load(dir, self.entry.clone(), source).await.ok()
        };
        let ledger = match load(self.dir.clone()).await {
            Some(ledger) => ledger,
            None => load(materialize(&self.dir)).await?,
        };
        Some(SharedLedger(Arc::new(RwLock::new(ledger))))
    }
}

/// Whether `name` matches `pattern`, whose `*` match any run of characters.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((prefix, rest)) => {
            name.starts_with(prefix)
                && (0..=name.len() - prefix.len()).any(|skip| name.is_char_boundary(prefix.len() + skip) && wildcard_match(rest, &name[prefix.len() + skip..]))
        }
    }
}

/// The files under `dir` that the include pattern matches, as paths relative to `dir`, sorted.
fn expand(dir: &FsPath, pattern: &str) -> Vec<String> {
    let mut matches = vec![String::new()];
    for segment in pattern.split('/') {
        matches = matches
            .into_iter()
            .flat_map(|base: String| {
                let path = dir.join(&base);
                let mut names = std::fs::read_dir(&path)
                    .map(|entries| {
                        entries
                            .filter_map(|it| it.ok())
                            .map(|it| it.file_name().to_string_lossy().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                names.sort();
                names
                    .into_iter()
                    .filter(|name| wildcard_match(segment, name))
                    .map(|name| if base.is_empty() { name } else { format!("{}/{}", base, name) })
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    matches.into_iter().filter(|it| dir.join(it).is_file()).collect()
}

/// A copy of the ledger directory `dir` in which every wildcard include lists the files it
/// matches, and every missing included file exists, empty.
fn materialize(dir: &FsPath) -> PathBuf {
    fn copy(from: &FsPath, to: &FsPath, files: &mut Vec<PathBuf>) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().filter_map(|it| it.ok()) {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &target, files);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
                files.push(target);
            }
        }
    }
    let target = tempfile::tempdir().unwrap().into_path();
    let mut files = vec![];
    copy(dir, &target, &mut files);
    for file in files.into_iter().filter(|it| it.extension().is_some_and(|ext| ext == "zhang" || ext == "bean")) {
        let base = file.parent().unwrap().to_path_buf();
        let text = std::fs::read_to_string(&file).unwrap();
        let lines = text
            .lines()
            .flat_map(|line| {
                let pattern = line.trim().strip_prefix("include \"").and_then(|it| it.strip_suffix('"'));
                match pattern {
                    Some(pattern) if pattern.contains('*') => expand(&base, pattern).into_iter().map(|it| format!("include \"{}\"", it)).collect(),
                    Some(pattern) => {
                        let included = base.join(pattern);
                        if !included.exists() {
                            std::fs::create_dir_all(included.parent().unwrap()).unwrap();
                            std::fs::write(&included, "").unwrap();
                        }
                        vec![line.to_owned()]
                    }
                    None => vec![line.to_owned()],
                }
            })
            .collect::<Vec<_>>();
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    }
    target
}

/// The main files of the ledgers in `dir`'s sub-directories (or of `dir` itself).
pub(crate) fn fixtures_in(dir: &FsPath) -> Vec<Fixture> {
    let mut dirs = vec![dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut subs = entries
            .filter_map(|it| it.ok())
            .map(|it| it.path())
            .filter(|it| it.is_dir())
            .collect::<Vec<_>>();
        subs.sort();
        dirs.extend(subs);
    }
    dirs.into_iter()
        .flat_map(|dir| {
            ["main.zhang", "main.bean"]
                .into_iter()
                .filter(|entry| dir.join(entry).is_file())
                .map(|entry| Fixture {
                    dir: dir.clone(),
                    entry: entry.to_owned(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Every ledger of `integration-tests/` and `examples/`.
pub(crate) fn repository_fixtures() -> Vec<Fixture> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut fixtures = fixtures_in(&root.join("integration-tests"));
    fixtures.extend(fixtures_in(&root.join("examples")));
    fixtures
}

fn month_pair(date: NaiveDate) -> (u32, u32) {
    (date.year() as u32, date.month())
}

/// The probes of a ledger: every endpoint, for every budget and commodity, an unknown one, and
/// the months around the budgets' series: before the first, every month of it (sampled when it is
/// long), and after the last.
pub(crate) async fn probes(ledger: &SharedLedger) -> Vec<Probe> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    let budgets = store.budgets.keys().cloned().collect::<BTreeSet<_>>();
    let commodities = store.commodities.keys().cloned().collect::<Vec<_>>();
    let mut months = BTreeSet::new();
    let first = store.budgets.values().flat_map(|it| it.detail.keys()).min().copied();
    let last_detail = store.budgets.values().flat_map(|it| it.detail.keys()).max().copied();
    let last_posting = store.postings.iter().map(|it| it.trx_datetime.date_naive()).max();
    if let (Some(first), Some(last)) = (first, last_detail) {
        let first = NaiveDate::from_ymd_opt((first / 100) as i32, first % 100, 1).unwrap();
        let last = NaiveDate::from_ymd_opt((last / 100) as i32, last % 100, 1).unwrap();
        let last = last_posting.map(|it| it.with_day(1).unwrap()).map_or(last, |it| it.max(last));
        let start = first - Months::new(1);
        let end = last + Months::new(2);
        let all = std::iter::successors(Some(start), |it| Some(*it + Months::new(1)))
            .take_while(|it| *it <= end)
            .collect::<Vec<_>>();
        let count = all.len();
        for (idx, month) in all.into_iter().enumerate() {
            if count <= 40 || idx < 3 || idx + 4 >= count || idx % 6 == 0 {
                months.insert(month_pair(month));
            }
        }
    }
    let mut probes = vec![Probe::BudgetList { month: None }];
    probes.extend(months.iter().map(|month| Probe::BudgetList { month: Some(*month) }));
    for name in budgets.iter().cloned().chain(std::iter::once("no-such-budget".to_owned())) {
        probes.push(Probe::BudgetInfo {
            name: name.clone(),
            month: None,
        });
        for month in &months {
            probes.push(Probe::BudgetInfo {
                name: name.clone(),
                month: Some(*month),
            });
            probes.push(Probe::BudgetInterval {
                name: name.clone(),
                year: month.0,
                month: month.1,
            });
        }
    }
    probes.push(Probe::Commodities);
    probes.extend(
        commodities
            .into_iter()
            .chain(std::iter::once("NO-SUCH-COMMODITY".to_owned()))
            .map(|name| Probe::Commodity { name }),
    );
    probes
}

/// The result of one probe.
pub(crate) struct Compared {
    pub(crate) probe: Probe,
    pub(crate) old: Outcome,
    pub(crate) new: Outcome,
    pub(crate) old_time: Duration,
    pub(crate) new_time: Duration,
}

pub(crate) async fn compare(ledger: &SharedLedger) -> Vec<Compared> {
    let mut compared = vec![];
    for probe in probes(ledger).await {
        let ((old, old_time), (new, new_time)) = probe.run(ledger).await;
        compared.push(Compared {
            probe,
            old,
            new,
            old_time,
            new_time,
        });
    }
    compared
}

/// Why an answer of a new handler differs from the old one's: a bug of #479's list the move
/// fixes, or one of its decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reason {
    /// the old handler panicked on an unknown commodity (`.expect("cannot find commodity")`)
    UnknownCommodityIsNotFound,
    /// decision 9: commodity lots list Assets and Liabilities holdings only
    LotsOfAssetsAndLiabilitiesOnly,
    /// the lots came in HashMap order; now by account, acquisition date and cost
    LotsInDeterministicOrder,
    /// the budgets came in HashMap order; now by name
    BudgetsInNameOrder,
    /// decision 8: `closed` is per month, not the final state
    ClosedPerMonth,
    /// decision 8: activity (and what budget directives add) is converted to the budget's
    /// commodity at its date, instead of adding the numbers of other commodities
    ActivityConverted,
    /// a budget's accounts are the set of `#budgets`, in name order; they were in the order of
    /// their `open` directives
    RelatedAccountsInNameOrder,
    /// the month's events are ordered by their Unix time; the old handler compared the UTC time of
    /// a budget event with the local time of a posting
    IntervalOrderedByTimestamp,
}

impl Reason {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Reason::UnknownCommodityIsNotFound => "an unknown commodity is a 404 instead of a panic",
            Reason::LotsOfAssetsAndLiabilitiesOnly => "lots list Assets and Liabilities holdings only (decision 9)",
            Reason::LotsInDeterministicOrder => "lots in deterministic order (were in HashMap order)",
            Reason::BudgetsInNameOrder => "budgets in name order (were in HashMap order)",
            Reason::ClosedPerMonth => "closed is per month (decision 8)",
            Reason::ActivityConverted => "activity converted to the budget's commodity at its date (decision 8)",
            Reason::RelatedAccountsInNameOrder => "related accounts in name order (were in open-directive order)",
            Reason::IntervalOrderedByTimestamp => "events and postings ordered by Unix time (old mixed UTC and local times)",
        }
    }
}

/// What the classification needs to know of a ledger, read from its store and directives.
pub(crate) struct Context {
    /// the first day of the month of each budget's `budget-close`
    closed_from: BTreeMap<String, NaiveDate>,
    /// the date of the first posting of a budget's accounts, or amount of a directive of the
    /// budget, in another commodity than the budget's, by budget
    foreign_from: BTreeMap<String, NaiveDate>,
    /// the current month in the ledger's timezone
    current_month: NaiveDate,
}

impl Context {
    pub(crate) async fn of(ledger: &SharedLedger) -> Context {
        let ledger = ledger.read().await;
        let store = ledger.store.read().unwrap();
        let first_of_month = |date: NaiveDate| date.with_day(1).unwrap();
        let mut closed_from = BTreeMap::new();
        let mut foreign_from: BTreeMap<String, NaiveDate> = BTreeMap::new();
        let mut foreign = |name: &str, date: NaiveDate| {
            let entry = foreign_from.entry(name.to_owned()).or_insert(date);
            *entry = (*entry).min(date);
        };
        let commodity = |name: &str| store.budgets.get(name).map(|it| it.commodity.clone());
        for directive in &ledger.directives {
            match &directive.data {
                Directive::BudgetClose(close) => {
                    closed_from.entry(close.name.clone()).or_insert(first_of_month(close.date.naive_date()));
                }
                Directive::BudgetAdd(add) if commodity(&add.name).is_some_and(|it| it != add.amount.commodity) => {
                    foreign(&add.name, add.date.naive_date());
                }
                Directive::BudgetTransfer(transfer) => {
                    for name in [&transfer.from, &transfer.to] {
                        if commodity(name).is_some_and(|it| it != transfer.amount.commodity) {
                            foreign(name, transfer.date.naive_date());
                        }
                    }
                }
                _ => {}
            }
        }
        for meta in store.metas.iter().filter(|meta| meta.meta_type == "AccountMeta" && meta.key == "budget") {
            let Some(budget_commodity) = commodity(&meta.value) else { continue };
            for posting in store.postings.iter().filter(|it| it.account.name() == meta.type_identifier) {
                if posting.inferred_amount.commodity != budget_commodity {
                    foreign(&meta.value, posting.trx_datetime.date_naive());
                }
            }
        }
        let current_month = first_of_month(chrono::Utc::now().with_timezone(&ledger.options.timezone).date_naive());
        Context {
            closed_from,
            foreign_from,
            current_month,
        }
    }
}

fn sorted_by(items: &mut [Json], key: impl Fn(&Json) -> String) -> bool {
    let before = items.to_vec();
    items.sort_by_key(|it| key(it));
    before != items
}

/// Erase the documented differences of one budget's figures (`old` and `new` are the objects of
/// one budget), noting their reasons.
fn budget_figures(old: &mut Json, new: &Json, name: &str, month: NaiveDate, context: &Context, reasons: &mut BTreeSet<Reason>) {
    if old["closed"] != new["closed"] {
        // the old handler said whether the budget is closed now; the new one whether it was closed by the month
        let closed_by_month = context.closed_from.get(name).is_some_and(|from| *from <= month);
        if old["closed"] == Json::Bool(context.closed_from.contains_key(name)) && new["closed"] == Json::Bool(closed_by_month) {
            old["closed"] = new["closed"].clone();
            reasons.insert(Reason::ClosedPerMonth);
        }
    }
    let amounts = ["assigned_amount", "activity_amount", "available_amount"];
    if amounts.iter().any(|key| old[key] != new[key]) {
        // only a budget with amounts in other commodities by the end of the month can differ
        let end_of_month = month + Months::new(1);
        if context.foreign_from.get(name).is_some_and(|from| *from < end_of_month) {
            for key in amounts {
                old[key] = new[key].clone();
            }
            reasons.insert(Reason::ActivityConverted);
        }
    }
}

/// The reasons why `new` differs from `old`, or the remaining difference if a documented reason
/// does not explain it.
pub(crate) fn classify(probe: &Probe, old: &Outcome, new: &Outcome, context: &Context) -> Result<BTreeSet<Reason>, String> {
    let mut reasons = BTreeSet::new();
    if old == new {
        return Ok(reasons);
    }
    let (mut old_json, new_json) = match (probe, old, new) {
        (Probe::Commodity { .. }, Outcome::Panic, Outcome::Status(404)) => {
            reasons.insert(Reason::UnknownCommodityIsNotFound);
            return Ok(reasons);
        }
        (_, Outcome::Json(old), Outcome::Json(new)) => (old.clone(), new.clone()),
        _ => return Err(format!("old {:?}, new {:?}", old, new)),
    };
    let mut new_json = new_json;
    let month = probe.month().unwrap_or(context.current_month);
    match probe {
        Probe::Commodity { .. } => {
            let lot_key = |lot: &Json| format!("{} {} {}", lot["account"], lot["acquisition_date"], lot["cost"]);
            if let Some(lots) = old_json["lots"].as_array_mut() {
                let before = lots.len();
                lots.retain(|lot| {
                    lot["account"]
                        .as_str()
                        .is_some_and(|it| it.starts_with("Assets:") || it.starts_with("Liabilities:"))
                });
                if lots.len() != before {
                    reasons.insert(Reason::LotsOfAssetsAndLiabilitiesOnly);
                }
                if sorted_by(lots, lot_key) {
                    reasons.insert(Reason::LotsInDeterministicOrder);
                }
            }
            if let Some(lots) = new_json["lots"].as_array_mut() {
                sorted_by(lots, lot_key);
            }
        }
        Probe::BudgetList { .. } => {
            let name = |it: &Json| it["name"].as_str().unwrap_or_default().to_owned();
            if let Some(budgets) = old_json.as_array_mut() {
                if sorted_by(budgets, name) {
                    reasons.insert(Reason::BudgetsInNameOrder);
                }
            }
            if let (Some(old_budgets), Some(new_budgets)) = (old_json.as_array_mut(), new_json.as_array()) {
                for (old_budget, new_budget) in old_budgets.iter_mut().zip(new_budgets) {
                    let budget = name(new_budget);
                    if name(old_budget) == budget {
                        budget_figures(old_budget, new_budget, &budget, month, context, &mut reasons);
                    }
                }
            }
        }
        Probe::BudgetInfo { name, .. } => {
            if let Some(accounts) = old_json["related_accounts"].as_array_mut() {
                if sorted_by(accounts, |it| it.as_str().unwrap_or_default().to_owned()) {
                    reasons.insert(Reason::RelatedAccountsInNameOrder);
                }
            }
            budget_figures(&mut old_json, &new_json, name, month, context, &mut reasons);
        }
        Probe::BudgetInterval { .. } => {
            // newest first; of the same time, budget events first, then in the old order
            let key = |it: &Json| (std::cmp::Reverse(it["timestamp"].as_i64().unwrap_or_default()), it["type"] != "BudgetEvent");
            if let Some(events) = old_json.as_array_mut() {
                let before = events.clone();
                events.sort_by_key(|it| key(it));
                if before != *events {
                    reasons.insert(Reason::IntervalOrderedByTimestamp);
                }
            }
        }
        Probe::Commodities => {}
    }
    if old_json == new_json {
        Ok(reasons)
    } else {
        Err(json_diff(&old_json, &new_json))
    }
}

/// The paths where two JSON values differ, with both values; at most a few.
pub(crate) fn json_diff(old: &Json, new: &Json) -> String {
    fn walk(path: String, old: &Json, new: &Json, out: &mut Vec<String>) {
        if out.len() >= 4 || old == new {
            return;
        }
        match (old, new) {
            (Json::Object(a), Json::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    walk(
                        format!("{}.{}", path, key),
                        a.get(key).unwrap_or(&Json::Null),
                        b.get(key).unwrap_or(&Json::Null),
                        out,
                    );
                }
            }
            (Json::Array(a), Json::Array(b)) if a.len() == b.len() => {
                for (idx, (a, b)) in a.iter().zip(b).enumerate() {
                    walk(format!("{}[{}]", path, idx), a, b, out);
                }
            }
            _ => out.push(format!("{}: {} -> {}", if path.is_empty() { "$" } else { &path }, old, new)),
        }
    }
    let mut out = vec![];
    walk(String::new(), old, new, &mut out);
    out.join("; ")
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every answer of the new handlers on the repository's ledgers is the old handler's, but for
    /// the documented differences.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_new_handlers_answer_as_the_old_ones_but_for_documented_differences() {
        let mut unexplained = vec![];
        for fixture in repository_fixtures() {
            let Some(ledger) = fixture.try_load().await else { continue };
            let context = Context::of(&ledger).await;
            for compared in compare(&ledger).await {
                if let Err(diff) = classify(&compared.probe, &compared.old, &compared.new, &context) {
                    unexplained.push(format!("{} {:?}: {}", fixture.name(), compared.probe, diff));
                }
            }
        }
        assert!(unexplained.is_empty(), "unexplained differences:\n{}", unexplained.join("\n"));
    }

    /// The probes of one endpoint on one ledger: how many, their differences by reasons with an
    /// example, and the timings of both handlers.
    #[derive(Default)]
    struct Stats {
        probes: u32,
        differences: BTreeMap<String, (usize, String)>,
        old_total: Duration,
        new_total: Duration,
        old_max: Duration,
        new_max: Duration,
    }

    /// The comparison on the repository's ledgers and those of `ZHANG_GOLDEN_LEDGERS`, with every
    /// difference and the timings of both handlers.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn golden_report() {
        let mut fixtures = repository_fixtures();
        if let Ok(extra) = std::env::var("ZHANG_GOLDEN_LEDGERS") {
            for dir in extra.split(':').filter(|it| !it.is_empty()) {
                fixtures.extend(fixtures_in(FsPath::new(dir)));
            }
        }
        let mut unexplained = 0;
        println!("| endpoint | ledger | probes | differ | reasons | example (old -> new) |");
        println!("|---|---|---|---|---|---|");
        let mut timings = vec![];
        for fixture in fixtures {
            let Some(ledger) = fixture.try_load().await else {
                println!("| - | {} | - | - | not loadable with the local data source | - |", fixture.name());
                continue;
            };
            let context = Context::of(&ledger).await;
            let compared = compare(&ledger).await;
            // per endpoint: probes, differences by reasons, an example of each, timings
            let mut kinds: BTreeMap<&str, Stats> = BTreeMap::new();
            for it in &compared {
                let stats = kinds.entry(it.probe.kind()).or_default();
                stats.probes += 1;
                stats.old_total += it.old_time;
                stats.new_total += it.new_time;
                stats.old_max = stats.old_max.max(it.old_time);
                stats.new_max = stats.new_max.max(it.new_time);
                if it.old == it.new {
                    continue;
                }
                let (reasons, example) = match classify(&it.probe, &it.old, &it.new, &context) {
                    Ok(reasons) => (
                        reasons.iter().map(|it| it.describe()).collect::<Vec<_>>().join("; "),
                        match (it.old.json(), it.new.json()) {
                            (Some(old), Some(new)) => json_diff(old, new),
                            _ => format!("{:?} -> {:?}", it.old, it.new),
                        },
                    ),
                    Err(diff) => {
                        unexplained += 1;
                        ("**UNEXPLAINED**".to_owned(), diff)
                    }
                };
                let example = format!("{:?}: {}", it.probe, example).chars().take(400).collect::<String>();
                stats.differences.entry(reasons).or_insert((0, example)).0 += 1;
            }
            for (kind, stats) in kinds {
                for (reasons, (differ, example)) in &stats.differences {
                    println!(
                        "| {} | {} | {} | {} | {} | {} |",
                        kind,
                        fixture.name(),
                        stats.probes,
                        differ,
                        reasons,
                        example.replace('|', "\\|")
                    );
                }
                timings.push((fixture.name(), kind, stats));
            }
        }
        println!("\n| ledger | endpoint | probes | old mean | new mean | old max | new max |");
        println!("|---|---|---|---|---|---|---|");
        for (ledger, kind, stats) in timings {
            println!(
                "| {} | {} | {} | {:.2?} | {:.2?} | {:.2?} | {:.2?} |",
                ledger,
                kind,
                stats.probes,
                stats.old_total / stats.probes,
                stats.new_total / stats.probes,
                stats.old_max,
                stats.new_max
            );
        }
        assert_eq!(unexplained, 0, "unexplained differences");
    }
}

/// The documented differences, on small ledgers whose figures are worked out by hand: what the old
/// handlers answered, and what the new ones answer.
#[cfg(test)]
mod documented_differences {
    use serde_json::json;

    use super::*;

    async fn ledger_of(text: &str) -> SharedLedger {
        let dir = tempfile::tempdir().unwrap().into_path();
        std::fs::write(dir.join("main.zhang"), text).unwrap();
        Fixture {
            dir,
            entry: "main.zhang".to_owned(),
        }
        .try_load()
        .await
        .expect("the ledger loads")
    }

    async fn answers(ledger: &SharedLedger, probe: Probe) -> (Outcome, Outcome) {
        let ((old, _), (new, _)) = probe.run(ledger).await;
        (old, new)
    }

    async fn json_answers(ledger: &SharedLedger, probe: Probe) -> (Json, Json) {
        match answers(ledger, probe.clone()).await {
            (Outcome::Json(old), Outcome::Json(new)) => (old, new),
            other => panic!("{:?}: {:?}", probe, other),
        }
    }

    /// Asia/Shanghai is UTC+8. `trip` and `food` start in March 2025 with 1000 and 500 CNY.
    /// - The flight of 2025-04-02 costs 300 USD: 2100.0 CNY at the price of 2025-03-31 (7.0),
    ///   the latest one on that date. With the 40 CNY taxi, April's activity is 2140.0 CNY, and
    ///   with the 200 CNY added on 2025-04-10 April starts with 1200 CNY, so 1200 - 2140.0 =
    ///   -940.0 CNY is left. The old handler added 300 + 40 = 340 "CNY", leaving 860.
    /// - `trip` is closed on 2025-05-02: closed from May, not before; the old handler said closed
    ///   for every month.
    /// - The budget-add of 2025-04-10 (00:00 +08:00, Unix 1744214400) is after the taxi of
    ///   2025-04-09 20:00 +08:00 (1744200000); the old handler compared the add's UTC time
    ///   (04-09 16:00) with the taxi's local time (20:00) and put the taxi first.
    const BUDGETS: &str = r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD

1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Expenses:Travel
  budget: trip
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Dining
  budget: food

2025-03-01 budget trip CNY
2025-03-01 budget food CNY
  alias: "Groceries"
2025-03-01 budget-add trip 1000 CNY
2025-03-01 budget-add food 500 CNY

2025-03-31 price USD 7.0 CNY
2025-04-30 price USD 7.3 CNY

2025-03-15 "Bistro" "dinner"
  Expenses:Dining 60 CNY
  Assets:Bank

2025-04-02 "Airline" "flight, in dollars"
  Expenses:Travel 300 USD
  Assets:USBank

2025-04-09 20:00:00 "Taxi" "late taxi"
  Expenses:Travel 40 CNY
  Assets:Bank

2025-04-10 budget-add trip 200 CNY

2025-05-02 budget-close trip
"#;

    fn figures(json: &Json) -> (String, String, String, bool) {
        (
            json["assigned_amount"]["number"].as_str().unwrap().to_owned(),
            json["activity_amount"]["number"].as_str().unwrap().to_owned(),
            json["available_amount"]["number"].as_str().unwrap().to_owned(),
            json["closed"].as_bool().unwrap(),
        )
    }

    fn of(assigned: &str, activity: &str, available: &str, closed: bool) -> (String, String, String, bool) {
        (assigned.to_owned(), activity.to_owned(), available.to_owned(), closed)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn budget_activity_is_converted_and_closed_is_per_month() {
        let ledger = ledger_of(BUDGETS).await;
        let info = |month: (u32, u32)| Probe::BudgetInfo {
            name: "trip".to_owned(),
            month: Some(month),
        };
        // before the budget starts: nothing, and not closed yet
        let (old, new) = json_answers(&ledger, info((2025, 2))).await;
        assert_eq!(figures(&old), of("0", "0", "0", true));
        assert_eq!(figures(&new), of("0", "0", "0", false));
        let (old, new) = json_answers(&ledger, info((2025, 3))).await;
        assert_eq!(figures(&old), of("1000", "0", "1000", true));
        assert_eq!(figures(&new), of("1000", "0", "1000", false));
        let (old, new) = json_answers(&ledger, info((2025, 4))).await;
        assert_eq!(figures(&old), of("1200", "340", "860", true));
        assert_eq!(figures(&new), of("1200", "2140.0", "-940.0", false));
        let (old, new) = json_answers(&ledger, info((2025, 5))).await;
        assert_eq!(figures(&old), of("860", "0", "860", true));
        assert_eq!(figures(&new), of("-940.0", "0", "-940.0", true));
        // after the last month of the ledger, the budget carries over
        let (old, new) = json_answers(&ledger, info((2026, 1))).await;
        assert_eq!(figures(&old), of("860", "0", "860", true));
        assert_eq!(figures(&new), of("-940.0", "0", "-940.0", true));
        assert_eq!(new["related_accounts"], json!(["Expenses:Travel"]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn budgets_are_listed_by_name_and_their_accounts_sorted() {
        let ledger = ledger_of(BUDGETS).await;
        let (_, new) = json_answers(&ledger, Probe::BudgetList { month: Some((2025, 4)) }).await;
        let names = new.as_array().unwrap().iter().map(|it| it["name"].clone()).collect::<Vec<_>>();
        assert_eq!(names, vec![json!("food"), json!("trip")]);
        assert_eq!(new[0]["alias"], json!("Groceries"));
        // food spent 60 CNY in March and nothing in April
        assert_eq!(figures(&new[0]), of("440", "0", "440", false));
        assert_eq!(figures(&new[1]), of("1200", "2140.0", "-940.0", false));
        // a month before every budget lists none
        let (old, new) = json_answers(&ledger, Probe::BudgetList { month: Some((2025, 2)) }).await;
        assert_eq!((old, new), (json!([]), json!([])));

        let (old, new) = json_answers(
            &ledger,
            Probe::BudgetInfo {
                name: "food".to_owned(),
                month: Some((2025, 3)),
            },
        )
        .await;
        assert_eq!(old["related_accounts"], json!(["Expenses:Food", "Expenses:Dining"]));
        assert_eq!(new["related_accounts"], json!(["Expenses:Dining", "Expenses:Food"]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_months_events_are_ordered_by_their_time() {
        let ledger = ledger_of(BUDGETS).await;
        let probe = Probe::BudgetInterval {
            name: "trip".to_owned(),
            year: 2025,
            month: 4,
        };
        let (old, new) = json_answers(&ledger, probe).await;
        let summary = |json: &Json| {
            json.as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    let what = match it["type"].as_str().unwrap() {
                        "BudgetEvent" => format!("{} {}", it["event_type"].as_str().unwrap(), it["amount"]["number"].as_str().unwrap()),
                        _ => format!("{} {}", it["narration"].as_str().unwrap(), it["inferred_unit"]["number"].as_str().unwrap()),
                    };
                    (it["timestamp"].as_i64().unwrap(), what)
                })
                .collect::<Vec<_>>()
        };
        let add = (1744214400, "AddAssignedAmount 200".to_owned());
        let taxi = (1744200000, "late taxi 40".to_owned());
        let flight = (1743523200, "flight, in dollars 300".to_owned());
        assert_eq!(summary(&old), vec![taxi.clone(), add.clone(), flight.clone()]);
        assert_eq!(summary(&new), vec![add, taxi, flight]);
        // the posting's local time, and its account's balance in its currency after it
        assert_eq!(new[1]["datetime"], json!("2025-04-09T20:00:00"));
        assert_eq!(new[1]["account_after"], json!({"number": "40", "commodity": "CNY"}));
        assert_eq!(new[2]["account_after"], json!({"number": "300", "commodity": "USD"}));
        // an unknown budget, and a month that does not exist
        let unknown = Probe::BudgetInterval {
            name: "nope".to_owned(),
            year: 2025,
            month: 4,
        };
        assert_eq!(answers(&ledger, unknown).await, (Outcome::Status(404), Outcome::Status(404)));
        let response = budget::get_budget_interval_detail(
            State(SharedLedger(ledger.0.clone())),
            Path(BudgetIntervalDetailRequest {
                budget_name: "trip".to_owned(),
                year: 2025,
                month: 13,
            }),
        )
        .await;
        assert_eq!(response.err().map(|err| err.into_response().status().as_u16()), Some(400));
    }

    /// 15 AAPL bought at 100 and 120 USD, 4 sold from the first lot; a loan in Liabilities. The
    /// latest USD price of AAPL is the 125 USD quoted at 10:00 on 2024-03-01.
    const COMMODITIES: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity CNY
1970-01-01 commodity AAPL
  group: "Stocks"

1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Liabilities:Loan
1970-01-01 open Income:Gains

2024-01-02 "Broker" "buy"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD

2024-02-01 "Broker" "buy more"
  Assets:Broker 5 AAPL {120 USD, 2024-02-01}
  Assets:Cash -600 USD

2024-03-01 "Broker" "sell"
  Assets:Broker -4 AAPL {100 USD}
  Assets:Cash 480 USD
  Income:Gains -80 USD

2024-04-01 "Bank" "loan"
  Assets:Cash 5000 USD
  Liabilities:Loan -5000 USD

2024-01-15 price AAPL 110 USD
2024-03-01 10:00:00 price AAPL 125 USD
2024-03-01 price AAPL 900 CNY
"#;

    #[tokio::test(flavor = "multi_thread")]
    async fn commodity_lots_are_assets_and_liabilities_holdings_in_order() {
        let ledger = ledger_of(COMMODITIES).await;
        let (old, new) = json_answers(&ledger, Probe::Commodity { name: "AAPL".to_owned() }).await;
        assert_eq!(old["info"], new["info"]);
        assert_eq!(
            new["info"],
            json!({
                "name": "AAPL", "precision": 2, "prefix": null, "suffix": null, "rounding": "RoundDown", "group": "Stocks",
                "total_amount": "11",
                "latest_price_date": "2024-03-01T10:00:00", "latest_price_amount": "125", "latest_price_commodity": "USD"
            })
        );
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Broker", "amount": "6", "cost": {"number": "100", "commodity": "USD"}, "price": null, "acquisition_date": "2024-01-02"},
                {"account": "Assets:Broker", "amount": "5", "cost": {"number": "120", "commodity": "USD"}, "price": null, "acquisition_date": "2024-02-01"},
            ])
        );
        assert_eq!(old["prices"], new["prices"]);
        assert_eq!(
            new["prices"],
            json!([
                {"datetime": "2024-01-15T00:00:00", "amount": {"number": "110", "commodity": "USD"}},
                {"datetime": "2024-03-01T00:00:00", "amount": {"number": "900", "commodity": "CNY"}},
                {"datetime": "2024-03-01T10:00:00", "amount": {"number": "125", "commodity": "USD"}},
            ])
        );

        // the old handler also listed Income:Gains as a holding of USD
        let (old, new) = json_answers(&ledger, Probe::Commodity { name: "USD".to_owned() }).await;
        let accounts = |json: &Json| {
            let mut accounts = json["lots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|lot| lot["account"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            accounts.sort();
            accounts
        };
        assert_eq!(accounts(&old), vec!["Assets:Cash", "Income:Gains", "Liabilities:Loan"]);
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Cash", "amount": "3880", "cost": null, "price": null, "acquisition_date": null},
                {"account": "Liabilities:Loan", "amount": "-5000", "cost": null, "price": null, "acquisition_date": null},
            ])
        );
        assert_eq!(
            (old["info"]["total_amount"].clone(), new["info"]["total_amount"].clone()),
            (json!("-1120"), json!("-1120"))
        );

        // an unknown commodity
        assert_eq!(
            answers(&ledger, Probe::Commodity { name: "NOPE".to_owned() }).await,
            (Outcome::Panic, Outcome::Status(404))
        );
    }
}
