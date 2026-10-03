//! The golden comparison of the journal, new-transaction, documents and errors endpoints (#479): the
//! hand-written handlers (`*_legacy`) against the built-in queries that replace them, on every ledger
//! of `integration-tests` and `examples`, in each format it has, the fava demo ledger, and the survey
//! probes in `tests/fixtures/journals`.
//!
//! The JSON of both is compared after normalising map order. Lists are matched item by item by what
//! identifies an item (a transaction by its id, a balance assertion by its date, account and amount,
//! a document by its path), so a change of order is one difference rather than a cascade. Every
//! difference must be one the migration means to make: each [`Reason`] says why, and the test fails
//! on any other. The expected differences are checked separately, with hand-verified values, in
//! `journals_engine.rs`.
//!
//! `ZHANG_GOLDEN_REPORT=1` prints every difference, expected or not, grouped by endpoint, ledger and
//! reason; `ZHANG_GOLDEN_EXTRA=dir[:entry],...` compares more ledgers (such as a large generated
//! one), and `ZHANG_GOLDEN_BENCH=1` times both implementations.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::Value;
use tokio::sync::RwLock;
use zhang_ast::Directive;
use zhang_core::data_source::{DataSource, LoadResult};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;
use zhang_server::request::JournalRequest;
use zhang_server::routes::common::{get_errors, get_errors_legacy};
use zhang_server::routes::document::{get_documents, get_documents_legacy};
use zhang_server::routes::transaction::{get_info_for_new_transactions, get_info_for_new_transactions_legacy, get_journals, get_journals_legacy};
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;

/// A ledger to compare on.
struct Fixture {
    name: String,
    dir: PathBuf,
    entry: String,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Every ledger of `integration-tests` and `examples` in each format it has, the survey probes, and
/// the ledgers of `ZHANG_GOLDEN_EXTRA`.
fn fixtures() -> Vec<Fixture> {
    let mut dirs = vec![];
    for root in [
        workspace().join("integration-tests"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journals"),
    ] {
        let mut entries = std::fs::read_dir(&root)
            .unwrap()
            .map(|it| it.unwrap().path())
            .filter(|it| it.is_dir())
            .collect::<Vec<_>>();
        entries.sort();
        dirs.extend(entries);
    }
    dirs.push(workspace().join("examples"));
    let mut fixtures = vec![];
    for dir in dirs {
        for entry in ["main.zhang", "main.bean"] {
            if dir.join(entry).exists() {
                fixtures.push(Fixture {
                    name: format!("{}/{}", dir.file_name().unwrap().to_string_lossy(), entry),
                    dir: dir.clone(),
                    entry: entry.to_owned(),
                });
            }
        }
    }
    if let Ok(extra) = std::env::var("ZHANG_GOLDEN_EXTRA") {
        for item in extra.split(',').filter(|it| !it.is_empty()) {
            let (dir, entry) = item.split_once(':').unwrap_or((item, "main.zhang"));
            fixtures.push(Fixture {
                name: format!("{}/{}", dir, entry),
                dir: PathBuf::from(dir),
                entry: entry.to_owned(),
            });
        }
    }
    fixtures
}

/// The local files of a ledger, with the wildcard includes (`include "data/*.zhang"`) the server's
/// file system source expands; zhang-core's local source reads includes as plain paths.
struct GlobSource {
    data_type: Box<dyn DataType<Carrier = String> + Send + Sync>,
}

impl GlobSource {
    /// The files `pattern` names relative to `dir`: a `*` in a segment matches any name.
    fn expand(dir: &Path, pattern: &str) -> Vec<PathBuf> {
        let mut paths = vec![if pattern.starts_with('/') { PathBuf::from("/") } else { dir.to_path_buf() }];
        for segment in pattern.split('/').filter(|it| !it.is_empty()) {
            paths = match segment.split_once('*') {
                None => paths.into_iter().map(|path| path.join(segment)).collect(),
                Some((prefix, suffix)) => {
                    let mut matched = vec![];
                    for path in paths {
                        let Ok(entries) = std::fs::read_dir(&path) else { continue };
                        let mut names = entries
                            .filter_map(|it| it.ok())
                            .map(|it| it.file_name().to_string_lossy().into_owned())
                            .collect::<Vec<_>>();
                        names.sort();
                        matched.extend(
                            names
                                .into_iter()
                                .filter(|name| name.len() >= prefix.len() + suffix.len() && name.starts_with(prefix) && name.ends_with(suffix))
                                .map(|name| path.join(name)),
                        );
                    }
                    matched
                }
            };
        }
        paths
    }
}

#[async_trait::async_trait]
impl DataSource for GlobSource {
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        Ok(std::fs::read(path)?)
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let entry = PathBuf::from(entry).canonicalize()?;
        let mut queue = std::collections::VecDeque::from([entry.join(endpoint).canonicalize()?]);
        let mut visited: Vec<PathBuf> = vec![];
        let mut directives = vec![];
        while let Some(path) = queue.pop_front() {
            if visited.contains(&path) {
                continue;
            }
            // `examples` includes a file that the first transaction written through the API creates
            let Ok(content) = std::fs::read(&path) else { continue };
            let content = String::from_utf8_lossy(&content).to_string();
            let parsed = self.data_type.transform(content, Some(path.to_string_lossy().to_string()))?;
            for directive in &parsed {
                if let Directive::Include(include) = &directive.data {
                    queue.extend(GlobSource::expand(path.parent().unwrap(), &include.file.clone().to_plain_string()));
                }
            }
            directives.extend(parsed);
            visited.push(path);
        }
        Ok(LoadResult {
            directives,
            visited_files: visited,
        })
    }
}

async fn load(fixture: &Fixture) -> SharedLedger {
    let data_type: Box<dyn DataType<Carrier = String> + Send + Sync> = if fixture.entry.ends_with(".bean") {
        Box::new(beancount::Beancount {})
    } else {
        Box::new(ZhangDataType {})
    };
    let ledger = Ledger::async_load(fixture.dir.clone(), fixture.entry.clone(), Arc::new(GlobSource { data_type }))
        .await
        .unwrap_or_else(|error| panic!("{}: {:?}", fixture.name, error));
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

async fn json(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if status.is_success() {
        body
    } else {
        serde_json::json!({ "status": status.as_u16(), "body": body })
    }
}

/// The old or the new implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Implementation {
    Old,
    New,
}

/// A search of the journal.
#[derive(Clone, Debug, Default)]
struct Search {
    keyword: Option<String>,
    tags: Option<Vec<String>>,
    links: Option<Vec<String>>,
}

impl Search {
    fn request(&self, page: u32, size: u32) -> JournalRequest {
        let set = |items: &Option<Vec<String>>| items.as_ref().map(|items| items.iter().cloned().collect::<HashSet<_>>());
        JournalRequest {
            page: Some(page),
            size: Some(size),
            keyword: self.keyword.clone(),
            tags: set(&self.tags),
            links: set(&self.links),
        }
    }

    fn is_filtered(&self) -> bool {
        self.keyword.is_some() || self.tags.is_some() || self.links.is_some()
    }
}

impl std::fmt::Display for Search {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "keyword={:?} tags={:?} links={:?}", self.keyword, self.tags, self.links)
    }
}

/// The searches to compare on a ledger: none, and searches by keywords, tags and links the ledger has;
/// on a large ledger (`ZHANG_GOLDEN_EXTRA`), a few of them.
async fn searches(ledger: &SharedLedger, large: bool) -> Vec<Search> {
    let (tags, links, payees, accounts) = {
        let guard = ledger.read().await;
        let store = guard.store.read().unwrap();
        let tags: BTreeSet<String> = store.transactions.values().flat_map(|it| it.tags.clone()).collect();
        let links: BTreeSet<String> = store.transactions.values().flat_map(|it| it.links.clone()).collect();
        let payees: BTreeSet<String> = store.transactions.values().filter_map(|it| it.payee.clone()).collect();
        let accounts: BTreeSet<String> = store.accounts.keys().cloned().collect();
        (tags, links, payees, accounts)
    };
    let mut keywords: Vec<String> = ["", "balance", "check", "pad", "Balance Check", "ASSETS", "food", "e", "zzz-nothing"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    keywords.extend(payees.iter().take(3).cloned());
    keywords.extend(accounts.iter().filter_map(|it| it.rsplit(':').next()).take(4).map(|it| it.to_lowercase()));
    keywords.extend(tags.iter().take(2).cloned());
    keywords.extend(links.iter().take(2).cloned());
    let mut searches = vec![Search::default()];
    searches.extend(keywords.into_iter().map(|keyword| Search {
        keyword: Some(keyword),
        ..Search::default()
    }));
    for tag in tags.iter().take(5) {
        searches.push(Search {
            tags: Some(vec![tag.clone()]),
            ..Search::default()
        });
        searches.push(Search {
            keyword: Some("e".to_owned()),
            tags: Some(vec![tag.clone(), "no-such-tag".to_owned()]),
            ..Search::default()
        });
    }
    for link in links.iter().take(5) {
        searches.push(Search {
            links: Some(vec![link.clone()]),
            ..Search::default()
        });
    }
    if let (Some(tag), Some(link)) = (tags.iter().next(), links.iter().next()) {
        searches.push(Search {
            tags: Some(vec![tag.clone()]),
            links: Some(vec![link.clone()]),
            ..Search::default()
        });
    }
    searches.push(Search {
        tags: Some(vec![]),
        ..Search::default()
    });
    if large {
        searches.retain(|search| {
            search
                .keyword
                .as_deref()
                .is_none_or(|keyword| ["balance", "zzz-nothing"].contains(&keyword) || payees.contains(keyword))
        });
        searches.truncate(8);
    }
    searches
}

async fn journal_page(implementation: Implementation, ledger: &SharedLedger, request: JournalRequest) -> Value {
    match implementation {
        Implementation::Old => json(get_journals_legacy(State(ledger.clone()), UrlQuery(request)).await).await,
        Implementation::New => json(get_journals(State(ledger.clone()), UrlQuery(request)).await).await,
    }
}

/// Every record of a search, page after page of 100, and how long it took.
async fn journal(implementation: Implementation, ledger: &SharedLedger, search: &Search) -> (Vec<Value>, Value, Duration) {
    let start = Instant::now();
    let mut records = vec![];
    let mut first = Value::Null;
    let mut page = 1;
    loop {
        let response = journal_page(implementation, ledger, search.request(page, 100)).await;
        let data = &response["data"];
        records.extend(data["records"].as_array().cloned().unwrap_or_default());
        if page == 1 {
            first = data.clone();
        }
        if u64::from(page) >= data["total_page"].as_u64().unwrap_or(0) {
            break;
        }
        page += 1;
    }
    (records, first, start.elapsed())
}

/// Why a difference is expected: a bug of #479 the migration fixes, a decision, or what the engine
/// defines where the hand-written code had no definition of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reason {
    /// `sequence` is the position in `#entries` (`seq`), not the store's counter
    Sequence,
    /// the id of a balance assertion is that of its `#entries` row
    BalanceCheckId,
    /// a narration that is absent is '' in the engine, as in beancount
    AbsentNarration,
    /// decision 7: a repeated metadata key keeps every value; metadata is sorted by key, as posting metadata is (#471)
    RepeatedMetadata,
    /// the cost is the per-unit cost of the posting's lots: a `{{total}}` cost is divided by the units, and a reduction
    /// written `{}` shows the cost of the lots it reduces when they share one
    PerUnitCost,
    /// the same number, written with other decimals
    Scale,
    /// a `balance ... with pad` names its pad account: `#entries.accounts` has it, so a keyword search finds the
    /// assertion by it
    PadAccountSearch,
    /// decision 4: the payees of padding transactions are not suggested
    SyntheticPayees,
    /// sorted, where the store's hash maps gave any order
    Sorted,
    /// the documents a transaction names come in written order, newest transaction first
    DocumentOrder,
    /// a document named by a posting belongs to the posting's account (`#documents.account`)
    PostingDocumentAccount,
    /// `#errors` lists errors by file and then by position in the file, not in the order they were found
    ErrorOrder,
    /// the file of an error is relative to the ledger's directory (`#errors.file`), not the full path on the server
    ErrorFile,
    /// the text of the directive of an error is trimmed at its end (`#errors.source`)
    ErrorSource,
}

/// A difference between the old and the new response.
#[derive(Debug, Clone)]
struct Finding {
    endpoint: &'static str,
    ledger: String,
    call: String,
    reason: Option<Reason>,
    detail: String,
}

#[derive(Default)]
struct Report {
    findings: Vec<Finding>,
    compared: usize,
}

impl Report {
    fn add(&mut self, endpoint: &'static str, ledger: &str, call: &str, reason: Option<Reason>, detail: String) {
        self.findings.push(Finding {
            endpoint,
            ledger: ledger.to_owned(),
            call: call.to_owned(),
            reason,
            detail,
        });
    }

    fn unexpected(&self) -> Vec<&Finding> {
        self.findings.iter().filter(|it| it.reason.is_none()).collect()
    }

    fn print(&self) {
        println!("compared {} responses", self.compared);
        let mut groups: BTreeMap<(Option<Reason>, &str, &str), Vec<&Finding>> = BTreeMap::new();
        for finding in &self.findings {
            groups
                .entry((finding.reason, finding.endpoint, finding.ledger.as_str()))
                .or_default()
                .push(finding);
        }
        for ((reason, endpoint, ledger), findings) in groups {
            let label = reason.map_or("UNEXPECTED".to_owned(), |it| format!("{:?}", it));
            println!("{} {} {}: {}", label, endpoint, ledger, findings.len());
            for finding in findings.iter().take(if reason.is_none() { 20 } else { 2 }) {
                println!("    [{}] {}", finding.call, finding.detail);
            }
        }
    }
}

fn number(value: &Value) -> Option<bigdecimal::BigDecimal> {
    value.as_str().and_then(|it| it.parse().ok())
}

/// The differences of two JSON values, by path.
fn diff(path: &str, old: &Value, new: &Value, out: &mut Vec<(String, Value, Value)>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let path = if path.is_empty() { key.clone() } else { format!("{}.{}", path, key) };
                diff(&path, a.get(key).unwrap_or(&Value::Null), b.get(key).unwrap_or(&Value::Null), out);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (idx, (a, b)) in a.iter().zip(b).enumerate() {
                diff(&format!("{}[{}]", path, idx), a, b, out);
            }
        }
        _ if old != new => out.push((path.to_owned(), old.clone(), new.clone())),
        _ => {}
    }
}

/// Items keyed by what identifies them, the n-th of identical keys (such as two balance lines alike) numbered
/// apart, so that no item hides another.
fn keyed(items: &[Value], key: impl Fn(&Value) -> String) -> Vec<(String, &Value)> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    items
        .iter()
        .map(|item| {
            let key = key(item);
            let nth = seen.entry(key.clone()).or_default();
            *nth += 1;
            (if *nth == 1 { key } else { format!("{} #{}", key, nth) }, item)
        })
        .collect()
}

/// What identifies a journal item across both implementations: a transaction its id, a balance
/// assertion (whose id changes) its date, account and asserted amount.
fn journal_key(record: &Value) -> String {
    if record["type"] == "BalanceCheck" {
        let asserted = &record["postings"][0]["account_after"];
        format!(
            "check {} {} {} {}",
            record["datetime"],
            record["narration"],
            number(&asserted["number"]).map(|it| it.normalized().to_string()).unwrap_or_default(),
            asserted["commodity"]
        )
    } else {
        record["id"].as_str().unwrap_or_default().to_owned()
    }
}

/// Whether the new metadata is the old with every value of a repeated key, sorted by key.
fn repeated_metadata(old: &Value, new: &Value) -> bool {
    let pairs = |value: &Value| {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|it| {
                        (
                            it["key"].as_str().unwrap_or_default().to_owned(),
                            it["value"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let (old, new) = (pairs(old), pairs(new));
    let keys = |pairs: &[(String, String)]| pairs.iter().map(|it| it.0.clone()).collect::<BTreeSet<_>>();
    new.windows(2).all(|it| it[0].0 <= it[1].0) && old.iter().all(|it| new.contains(it)) && keys(&old) == keys(&new) && new.len() >= old.len()
}

/// Whether the per-unit cost of the `index`-th posting of `new_record` is what the old journal's cost
/// means: a written `{{total}}` divided by the units (within the 28 significant digits of the
/// division), or, where the old journal had none (a reduction written `{}`), the cost of a lot an earlier
/// posting of the account opened, in `everything` (the new journal, newest first).
fn per_unit_cost_holds(index: usize, old_record: &Value, new_record: &Value, everything: &[Value]) -> bool {
    let (old_posting, new_posting) = (&old_record["postings"][index], &new_record["postings"][index]);
    let units = number(&new_posting["inferred_unit"]["number"]);
    let (old_cost, new_cost) = (&old_posting["cost"], &new_posting["cost"]);
    match (old_cost.is_null(), new_cost.is_null()) {
        (false, false) => {
            let (Some(units), Some(total), Some(per_unit)) = (units, number(&old_cost["number"]), number(&new_cost["number"])) else {
                return false;
            };
            let error = (per_unit * units.abs() - &total).abs();
            old_cost["commodity"] == new_cost["commodity"]
                && error * bigdecimal::BigDecimal::from(10i64.pow(15)) * bigdecimal::BigDecimal::from(10i64.pow(11)) <= total.abs()
        }
        (true, false) => {
            let Some(units) = units else { return false };
            let account = &new_posting["account"];
            let commodity = &new_posting["inferred_unit"]["commodity"];
            let position = everything.iter().position(|it| it["id"] == new_record["id"]).unwrap_or(everything.len());
            everything[position..].iter().skip(1).any(|earlier| {
                earlier["postings"].as_array().into_iter().flatten().any(|posting| {
                    posting["account"] == *account
                        && posting["inferred_unit"]["commodity"] == *commodity
                        && posting["cost"] == *new_cost
                        && number(&posting["inferred_unit"]["number"]).is_some_and(|opened| opened.sign() != units.sign())
                })
            })
        }
        _ => false,
    }
}

/// The reason of a difference within a journal item, `path` relative to the item.
fn journal_field_reason(path: &str, old_record: &Value, new_record: &Value, old: &Value, new: &Value, everything: &[Value]) -> Option<Reason> {
    let field = path.rsplit('.').next().unwrap_or_default();
    if path == "sequence" {
        return Some(Reason::Sequence);
    }
    if path == "id" && old_record["type"] == "BalanceCheck" {
        return Some(Reason::BalanceCheckId);
    }
    if path == "narration" && old.is_null() && new == "" {
        return Some(Reason::AbsentNarration);
    }
    if let (Some(a), Some(b)) = (number(old), number(new)) {
        if a == b && field == "number" {
            return Some(Reason::Scale);
        }
    }
    if path.starts_with("postings[") && path.contains(".cost") {
        let index = path["postings[".len()..].split(']').next().unwrap().parse::<usize>().unwrap();
        return per_unit_cost_holds(index, old_record, new_record, everything).then_some(Reason::PerUnitCost);
    }
    None
}

fn compare_journal(report: &mut Report, ledger: &str, search: &Search, old: &[Value], new: &[Value], everything: &[Value]) {
    let call = search.to_string();
    let (old_keyed, new_keyed) = (keyed(old, journal_key), keyed(new, journal_key));
    let old_keys: BTreeMap<String, &Value> = old_keyed.iter().cloned().collect();
    let new_keys: BTreeMap<String, &Value> = new_keyed.iter().cloned().collect();
    for (key, record) in &new_keys {
        if old_keys.contains_key(key) {
            continue;
        }
        // a balance ... with pad found by its pad account, which the padding transaction of its day names
        let by_pad_account = record["type"] == "BalanceCheck"
            && search.keyword.as_ref().is_some_and(|keyword| {
                everything.iter().any(|it| {
                    it["type"] == "BalancePad"
                        && it["datetime"] == record["datetime"]
                        && it["narration"]
                            .as_str()
                            .and_then(|it| it.split(" to ").nth(1))
                            .is_some_and(|pad| pad.to_lowercase().contains(&keyword.to_lowercase()))
                })
            });
        let reason = by_pad_account.then_some(Reason::PadAccountSearch);
        report.add("/api/journals", ledger, &call, reason, format!("only new: {}", key));
    }
    for key in old_keys.keys().filter(|key| !new_keys.contains_key(*key)) {
        report.add("/api/journals", ledger, &call, None, format!("only old: {}", key));
    }
    let common = |records: &[(String, &Value)], other: &BTreeMap<String, &Value>| {
        records
            .iter()
            .filter(|(key, _)| other.contains_key(key))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
    };
    let (old_common, new_common) = (common(&old_keyed, &new_keys), common(&new_keyed, &old_keys));
    let order = |keys: &[String]| keys.to_vec();
    if order(&old_common) != order(&new_common) {
        let reason = None;
        let first = order(&old_common)
            .into_iter()
            .zip(order(&new_common))
            .position(|(a, b)| a != b)
            .unwrap_or_default();
        report.add(
            "/api/journals",
            ledger,
            &call,
            reason,
            format!(
                "order from item {}: {:?} -> {:?}",
                first,
                order(&old_common).get(first),
                order(&new_common).get(first)
            ),
        );
    }
    for (key, old_record) in &old_keys {
        let Some(new_record) = new_keys.get(key) else { continue };
        let mut differences = vec![];
        diff("", old_record, new_record, &mut differences);
        for (path, old_value, new_value) in differences {
            let reason = if path == "metas" || path.starts_with("metas[") {
                repeated_metadata(&old_record["metas"], &new_record["metas"]).then_some(Reason::RepeatedMetadata)
            } else {
                journal_field_reason(&path, old_record, new_record, &old_value, &new_value, everything)
            };
            report.add(
                "/api/journals",
                ledger,
                &call,
                reason,
                format!("{} {}: {} -> {}", key, path, old_value, new_value),
            );
        }
    }
}

/// The pages of the whole journal, and a search's first page, have the same counts.
fn compare_page_counts(report: &mut Report, ledger: &str, call: &str, old: &Value, new: &Value, expected: Option<Reason>) {
    for field in ["total_count", "total_page", "page_size", "current_page"] {
        if old[field] != new[field] {
            report.add("/api/journals", ledger, call, expected, format!("{}: {} -> {}", field, old[field], new[field]));
        }
    }
}

async fn compare_journals(report: &mut Report, timings: &mut Timings, fixture: &Fixture, ledger: &SharedLedger) {
    let name = fixture.name.as_str();
    let (everything, _, _) = journal(Implementation::New, ledger, &Search::default()).await;
    let large = !fixture.dir.starts_with(workspace());
    for search in searches(ledger, large).await {
        let (old, old_first, old_time) = journal(Implementation::Old, ledger, &search).await;
        let (new, new_first, new_time) = journal(Implementation::New, ledger, &search).await;
        report.compared += 2;
        timings.add(
            format!(
                "{} /api/journals {}",
                name,
                if search.is_filtered() { "search (every page)" } else { "every page" }
            ),
            old_time,
            new_time,
        );
        compare_journal(report, name, &search, &old, &new, &everything);
        // a search that finds a balance ... with pad by its pad account counts it
        let pad_search = new.len() > old.len() && search.keyword.is_some();
        compare_page_counts(
            report,
            name,
            &search.to_string(),
            &old_first,
            &new_first,
            pad_search.then_some(Reason::PadAccountSearch),
        );
    }

    // pages of every size are windows of the same journal
    let (old_all, _, _) = journal(Implementation::Old, ledger, &Search::default()).await;
    let total = everything.len() as u32;
    for (page, size) in [(1, 1), (2, 1), (total.max(1), 1), (1, 7), (2, 7), (3, 7), (total + 2, 100), (2, 100)] {
        let call = format!("page={} size={}", page, size);
        let start = Instant::now();
        let old = journal_page(Implementation::Old, ledger, Search::default().request(page, size)).await;
        let old_time = start.elapsed();
        let start = Instant::now();
        let new = journal_page(Implementation::New, ledger, Search::default().request(page, size)).await;
        let new_time = start.elapsed();
        if size == 100 && page == 2 {
            timings.add(format!("{} /api/journals page 2 of 100", name), old_time, new_time);
        }
        report.compared += 1;
        compare_page_counts(report, name, &call, &old["data"], &new["data"], None);
        let window = |all: &[Value]| {
            let from = ((page - 1) * size) as usize;
            all.iter().skip(from).take(size as usize).cloned().collect::<Vec<_>>()
        };
        let records = |response: &Value| response["data"]["records"].as_array().cloned().unwrap_or_default();
        assert_eq!(records(&old), window(&old_all), "{} {}: old page", name, call);
        assert_eq!(records(&new), window(&everything), "{} {}: new page", name, call);
    }
}

async fn compare_new_transaction_info(report: &mut Report, fixture: &Fixture, ledger: &SharedLedger) {
    let name = fixture.name.as_str();
    let old = json(get_info_for_new_transactions_legacy(State(ledger.clone())).await).await;
    let new = json(get_info_for_new_transactions(State(ledger.clone())).await).await;
    report.compared += 1;
    let strings = |value: &Value| {
        value
            .as_array()
            .map(|it| it.iter().filter_map(|it| it.as_str()).map(str::to_owned).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for field in ["payee", "account_name"] {
        let (old, new) = (strings(&old["data"][field]), strings(&new["data"][field]));
        let old_set: BTreeSet<&String> = old.iter().collect();
        let new_set: BTreeSet<&String> = new.iter().collect();
        for removed in old_set.difference(&new_set) {
            let reason = (field == "payee" && *removed == "Balance Pad").then_some(Reason::SyntheticPayees);
            report.add("/api/for-new-transaction", name, field, reason, format!("only old: {}", removed));
        }
        for added in new_set.difference(&old_set) {
            report.add("/api/for-new-transaction", name, field, None, format!("only new: {}", added));
        }
        if !new.windows(2).all(|it| it[0] < it[1]) {
            report.add("/api/for-new-transaction", name, field, None, format!("not sorted: {:?}", new));
        } else if old.iter().filter(|it| new_set.contains(it)).collect::<Vec<_>>() != new.iter().collect::<Vec<_>>() {
            report.add("/api/for-new-transaction", name, field, Some(Reason::Sorted), "order".to_owned());
        }
    }
}

async fn compare_documents(report: &mut Report, fixture: &Fixture, ledger: &SharedLedger) {
    let name = fixture.name.as_str();
    let old = json(get_documents_legacy(State(ledger.clone())).await).await;
    let new = json(get_documents(State(ledger.clone())).await).await;
    report.compared += 1;
    let items = |value: &Value| value["data"].as_array().cloned().unwrap_or_default();
    let (old, new) = (items(&old), items(&new));
    let key = |it: &Value| format!("{} {} {}", it["datetime"], it["path"], it["trx_id"]);
    let old_keys: BTreeMap<String, &Value> = keyed(&old, key).into_iter().collect();
    let new_keys: BTreeMap<String, &Value> = keyed(&new, key).into_iter().collect();
    for key in old_keys.keys().filter(|it| !new_keys.contains_key(*it)) {
        report.add("/api/documents", name, "", None, format!("only old: {}", key));
    }
    for key in new_keys.keys().filter(|it| !old_keys.contains_key(*it)) {
        report.add("/api/documents", name, "", None, format!("only new: {}", key));
    }
    let order = |items: &[Value]| items.iter().map(key).collect::<Vec<_>>();
    if order(&old) != order(&new) {
        // newest first both; within a date and time, in any order
        let by_time = |items: &[Value]| {
            let mut keys = items
                .iter()
                .map(|it| (it["datetime"].as_str().unwrap_or_default().to_owned(), key(it)))
                .collect::<Vec<_>>();
            keys.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            keys
        };
        let reason = (by_time(&old) == by_time(&new)).then_some(Reason::DocumentOrder);
        report.add("/api/documents", name, "", reason, format!("order: {:?} -> {:?}", order(&old), order(&new)));
    }
    for (key, old_item) in &old_keys {
        let Some(new_item) = new_keys.get(key) else { continue };
        let mut differences = vec![];
        diff("", old_item, new_item, &mut differences);
        for (path, old_value, new_value) in differences {
            let reason = (path == "account" && old_value.is_null() && !old_item["trx_id"].is_null()).then_some(Reason::PostingDocumentAccount);
            report.add("/api/documents", name, "", reason, format!("{} {}: {} -> {}", key, path, old_value, new_value));
        }
    }
}

async fn errors_page(implementation: Implementation, ledger: &SharedLedger, page: u32, size: u32) -> Value {
    let request = Search::default().request(page, size);
    match implementation {
        Implementation::Old => json(get_errors_legacy(State(ledger.clone()), axum::extract::Query(request)).await).await,
        Implementation::New => json(get_errors(State(ledger.clone()), axum::extract::Query(request)).await).await,
    }
}

/// Every error, page after page of the largest size.
async fn all_errors(implementation: Implementation, ledger: &SharedLedger) -> Vec<Value> {
    let mut items = vec![];
    for page in 1.. {
        let response = errors_page(implementation, ledger, page, 1000).await;
        items.extend(response["data"]["records"].as_array().cloned().unwrap_or_default());
        if u64::from(page) >= response["data"]["total_page"].as_u64().unwrap_or(0) {
            break;
        }
    }
    items
}

async fn compare_errors(report: &mut Report, fixture: &Fixture, ledger: &SharedLedger) {
    let name = fixture.name.as_str();
    let (old_items, new_items) = (all_errors(Implementation::Old, ledger).await, all_errors(Implementation::New, ledger).await);
    report.compared += 1;
    let key = |it: &Value| format!("{} {} {} {}", it["id"], it["error_type"], it["span"]["start"], it["metas"]);
    let collect = |items: &[Value]| {
        let mut keyed: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for item in items {
            keyed.entry(key(item)).or_default().push(item.clone());
        }
        keyed
    };
    let (old_keys, new_keys) = (collect(&old_items), collect(&new_items));
    for (key, olds) in &old_keys {
        let news = new_keys.get(key).cloned().unwrap_or_default();
        if olds.len() != news.len() {
            report.add("/api/errors", name, "", None, format!("{}: {} old, {} new", key, olds.len(), news.len()));
            continue;
        }
        for (old_item, new_item) in olds.iter().zip(&news) {
            let mut differences = vec![];
            diff("", old_item, new_item, &mut differences);
            for (path, old_value, new_value) in differences {
                let reason = match path.as_str() {
                    "span.filename" => match (old_value.as_str(), new_value.as_str()) {
                        (Some(old), Some(new)) if old.ends_with(&format!("/{}", new)) => Some(Reason::ErrorFile),
                        _ => None,
                    },
                    "span.content" => (old_value.as_str().map(str::trim_end) == new_value.as_str()).then_some(Reason::ErrorSource),
                    _ => None,
                };
                report.add("/api/errors", name, "", reason, format!("{} {}: {} -> {}", key, path, old_value, new_value));
            }
        }
    }
    for key in new_keys.keys().filter(|it| !old_keys.contains_key(*it)) {
        report.add("/api/errors", name, "", None, format!("only new: {}", key));
    }
    if old_items.iter().map(key).collect::<Vec<_>>() != new_items.iter().map(key).collect::<Vec<_>>() {
        // the new order is by file (an error without one first), then by position in the file
        let position = |it: &Value| (it["span"]["filename"].as_str().map(str::to_owned), it["span"]["start"].as_u64());
        let by_position = new_items.windows(2).all(|pair| position(&pair[0]) <= position(&pair[1]));
        report.add("/api/errors", name, "", by_position.then_some(Reason::ErrorOrder), "order".to_owned());
    }
    // the pages count the same
    for (page, size) in [(1, 10), (2, 10), (3, 1)] {
        let old = errors_page(Implementation::Old, ledger, page, size).await;
        let new = errors_page(Implementation::New, ledger, page, size).await;
        report.compared += 1;
        for field in ["total_count", "total_page", "page_size", "current_page"] {
            if old["data"][field] != new["data"][field] {
                report.add(
                    "/api/errors",
                    name,
                    &format!("page={} size={}", page, size),
                    None,
                    format!("{}: {} -> {}", field, old["data"][field], new["data"][field]),
                );
            }
        }
        let count = |value: &Value| value["data"]["records"].as_array().map(Vec::len);
        if count(&old) != count(&new) {
            report.add(
                "/api/errors",
                name,
                &format!("page={} size={}", page, size),
                None,
                "records on the page".to_owned(),
            );
        }
    }
}

#[derive(Default)]
struct Timings(BTreeMap<String, (Duration, Duration, usize)>);

impl Timings {
    fn add(&mut self, label: String, old: Duration, new: Duration) {
        let entry = self.0.entry(label).or_default();
        entry.0 += old;
        entry.1 += new;
        entry.2 += 1;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_built_in_queries_answer_as_the_hand_written_endpoints_did() {
    let mut report = Report::default();
    let mut timings = Timings::default();
    for fixture in fixtures() {
        let ledger = load(&fixture).await;
        compare_journals(&mut report, &mut timings, &fixture, &ledger).await;
        compare_new_transaction_info(&mut report, &fixture, &ledger).await;
        compare_documents(&mut report, &fixture, &ledger).await;
        compare_errors(&mut report, &fixture, &ledger).await;
    }
    if std::env::var("ZHANG_GOLDEN_REPORT").is_ok() {
        report.print();
    }
    if std::env::var("ZHANG_GOLDEN_BENCH").is_ok() {
        for (label, (old, new, count)) in &timings.0 {
            println!(
                "BENCH {}: old {:.3} ms, new {:.3} ms (mean of {})",
                label,
                old.as_secs_f64() * 1e3 / *count as f64,
                new.as_secs_f64() * 1e3 / *count as f64,
                count
            );
        }
    }
    let unexpected = report.unexpected();
    assert!(
        unexpected.is_empty(),
        "{} unexpected differences, the first: {:?}",
        unexpected.len(),
        unexpected.first()
    );
}

/// The fastest of `runs` calls of each of `old` and `new`, alternating them so that both see the
/// same load of the machine.
async fn fastest<F, G, Fut, Gut>(runs: usize, mut old: F, mut new: G) -> (Duration, Duration)
where
    F: FnMut() -> Fut,
    G: FnMut() -> Gut,
    Fut: std::future::Future<Output = Value>,
    Gut: std::future::Future<Output = Value>,
{
    let (mut old_best, mut new_best) = (Duration::MAX, Duration::MAX);
    for _ in 0..runs {
        let start = Instant::now();
        let value = old().await;
        old_best = old_best.min(start.elapsed());
        assert!(value.get("status").is_none(), "{}", value);
        let start = Instant::now();
        let value = new().await;
        new_best = new_best.min(start.elapsed());
        assert!(value.get("status").is_none(), "{}", value);
    }
    (old_best, new_best)
}

/// Old against new response times on the fava demo ledger and the ledgers of `ZHANG_GOLDEN_EXTRA`
/// (`cargo test --release -p zhang-server --test journals_golden -- --ignored --nocapture`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn bench_old_and_new() {
    let mut ledgers = vec![Fixture {
        name: "fava-demo-ledger/main.zhang".to_owned(),
        dir: workspace().join("integration-tests/fava-demo-ledger"),
        entry: "main.zhang".to_owned(),
    }];
    ledgers.extend(fixtures().into_iter().filter(|it| it.dir.is_absolute() && !it.dir.starts_with(workspace())));
    for fixture in ledgers {
        let start = Instant::now();
        let ledger = load(&fixture).await;
        println!("{}: loaded in {:?}", fixture.name, start.elapsed());
        let (tag, payee) = {
            let guard = ledger.read().await;
            let store = guard.store.read().unwrap();
            let tag = store.transactions.values().flat_map(|it| it.tags.clone()).min();
            let payee = store.transactions.values().filter_map(|it| it.payee.clone()).min();
            (tag, payee)
        };
        // warm the engine's cache of the ledger, as the first query after a load does
        json(get_journals(State(ledger.clone()), UrlQuery(Search::default().request(1, 100))).await).await;
        let mut cases: Vec<(String, Search, u32)> = vec![
            ("journal page 1".to_owned(), Search::default(), 1),
            ("journal page 10".to_owned(), Search::default(), 10),
            (
                "journal keyword 'restaurant'".to_owned(),
                Search {
                    keyword: Some("restaurant".to_owned()),
                    ..Search::default()
                },
                1,
            ),
            (
                "journal keyword 'zzz' (no match)".to_owned(),
                Search {
                    keyword: Some("zzz".to_owned()),
                    ..Search::default()
                },
                1,
            ),
        ];
        if let Some(payee) = payee {
            cases.push((
                format!("journal keyword payee {:?}", payee),
                Search {
                    keyword: Some(payee),
                    ..Search::default()
                },
                1,
            ));
        }
        if let Some(tag) = tag {
            cases.push((
                format!("journal tag {:?}", tag),
                Search {
                    tags: Some(vec![tag]),
                    ..Search::default()
                },
                1,
            ));
        }
        let runs = 30;
        let print = |label: &str, (old, new): (Duration, Duration)| {
            println!(
                "BENCH {} {}: old {:.2} ms, new {:.2} ms",
                fixture.name,
                label,
                old.as_secs_f64() * 1e3,
                new.as_secs_f64() * 1e3
            )
        };
        for (label, search, page) in cases {
            print(
                &label,
                fastest(
                    runs,
                    || journal_page(Implementation::Old, &ledger, search.request(page, 100)),
                    || journal_page(Implementation::New, &ledger, search.request(page, 100)),
                )
                .await,
            );
        }
        print(
            "for-new-transaction",
            fastest(
                runs,
                || async { json(get_info_for_new_transactions_legacy(State(ledger.clone())).await).await },
                || async { json(get_info_for_new_transactions(State(ledger.clone())).await).await },
            )
            .await,
        );
        print(
            "documents",
            fastest(
                runs,
                || async { json(get_documents_legacy(State(ledger.clone())).await).await },
                || async { json(get_documents(State(ledger.clone())).await).await },
            )
            .await,
        );
        print(
            "errors",
            fastest(
                runs,
                || errors_page(Implementation::Old, &ledger, 1, 10),
                || errors_page(Implementation::New, &ledger, 1, 10),
            )
            .await,
        );
    }
}
