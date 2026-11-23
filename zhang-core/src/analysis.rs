//! Analyzers: plugins that read a loaded ledger and publish what they find, without changing it (RFC #698 §2).
//!
//! An analyzer runs once per load, so at every write too, on the final ledger: after booking and the core's checks,
//! before the errors are placed and identified. It reads the records of the kinds it consumes ([`input`]) and queries
//! the ledger, and answers diagnostics and the rows of the tables its manifest declares
//! ([`zhang_shared::plugin_abi::analysis`]). The core cannot run a query (the query engine depends on it), so what
//! needs the engine is an [`AnalysisHost`]'s, which the embedder installs with the load's options ([`LoadOptions`]): the
//! server does, the playground and a bare `Ledger::load` do not.
//!
//! **Lifecycle.** While the ledger loads ([`crate::ledger::Ledger`]):
//!
//! 1. the host builds the ledger's catalog from the analyzers' manifests, and the ledger keeps it
//!    ([`crate::extensions::Extensions`]); an analyzer whose tables the catalog cannot have is not run;
//! 2. each analyzer of the catalog runs in turn, the default plugins first, then in the order of the `plugin`
//!    directives. Its queries read the core's tables and those of the analyzers before it; its own and those after it
//!    are not available yet, nor are the ledger's errors, which are not placed yet;
//! 3. its output is checked whole, the diagnostics here ([`diagnostics`]) and the tables by the host, and published
//!    into the analyzer's slot; anything wrong publishes a failure and no data;
//! 4. the diagnostics join the ledger's errors where the native checks report theirs ([`merge`]), then the query cache,
//!    which the analyses' queries built while the errors were not placed, is reset, and the load places and identifies
//!    the errors.
//!
//! **Failure isolation.** Whatever goes wrong with an analyzer (its manifest, a trap, its time limit, its memory, an
//! output that does not hold), the ledger loads: its snapshot is failed, with no data, and the ledger has one error
//! about it, [`ErrorKind::PluginAnalysisFailed`], on the plugin's directive. The other analyzers are unaffected.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde_json::{json, Map};
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Date, Directive, SpanInfo, Spanned};
use zhang_shared::plugin_abi::analysis::Phase;
use zhang_shared::plugin_abi::{analysis as abi, LedgerInfo};

use crate::domains::schemas::ErrorDomain;
use crate::ledger::Ledger;
use crate::plugin_policy::PluginPolicy;

/// The provider the budget system's legacy names and error kinds are kept for (RFC #698 §2.6, §3.4).
pub const BUDGET: &str = "zhang:budget";

/// The error kinds the budget system has always reported, which `zhang:budget` reports as they are; any other plugin's
/// diagnostic is a [`ErrorKind::PluginError`] with its kind in the meta `kind`.
pub const LEGACY_KINDS: [&str; 4] = ["BudgetDoesNotExist", "DefineDuplicatedBudget", "BudgetCommodityMismatch", "BudgetClosed"];

/// The most diagnostics one analysis reports; past it, one more says how many were left out.
pub const MAX_DIAGNOSTICS: usize = 10_000;

/// The longest message or meta value of a diagnostic, in bytes; a longer one is cut there, with `…`.
pub const MAX_META_LEN: usize = 4096;

/// The most metas of a diagnostic.
pub const MAX_METAS: usize = 32;

/// The metas the load sets on an error itself: a plugin's diagnostic does not give them. `txn_id` names the
/// transaction a diagnostic is placed at; `account_name` is the account of the budget kinds, which only `zhang:budget`
/// reports.
pub const RESERVED_METAS: [&str; 2] = ["txn_id", "account_name"];

/// The metas of a [`ErrorKind::PluginAnalysisFailed`] and of a plugin's diagnostic.
pub const PLUGIN_META: &str = "plugin";
pub const REASON_META: &str = "reason";
pub const MESSAGE_META: &str = "message";
pub const KIND_META: &str = "kind";

/// What the query engine does for the analyses of a ledger while it loads: build its catalog, give an analyzer the
/// queries it reads the ledger with, and publish what it found. The server installs one with the load's options.
pub trait AnalysisHost: Send + Sync {
    /// The catalog of a ledger whose analyzers declare `analyzers`, in order: the core's tables, the native providers'
    /// and theirs ([`crate::extensions::Extensions::catalog`]).
    fn catalog(&self, analyzers: &[AnalyzerTables<'_>]) -> AnalysisCatalog;

    /// What an analyzer of `ledger` queries it with while it analyses it: BQL over the ledger's catalog, with the
    /// limits of `POST /api/query`, and conversions with its prices. The ledger is lent for the call only: once the
    /// analyzer returned, `ledger` reads nothing ([`LentLedger::with`]).
    #[cfg(feature = "plugin_runtime")]
    fn queries(&self, ledger: LentLedger) -> Arc<dyn crate::plugin::router::RouterHost>;

    /// Publish `output`, what the analyzer providing the tables of `provider` found in `ledger`, checked against its
    /// manifest in the ledger's catalog: the data of its snapshot, or what does not hold.
    fn publish(&self, ledger: &Ledger, provider: &str, output: &abi::Output) -> Result<Arc<dyn Any + Send + Sync>, String>;

    /// [`AnalysisHost::publish`] the quick way: `output` as the host reads it first, read straight into the cells of
    /// the analyzer's tables. The same data, or `None` when this does not take the output as it is: then the output is
    /// read the long way, with [`AnalysisHost::publish`], which says what does not hold. A host without a quick way
    /// answers `None`.
    fn publish_raw(&self, ledger: &Ledger, provider: &str, output: &abi::RawOutput<'_>) -> Option<Arc<dyn Any + Send + Sync>> {
        let _ = (ledger, provider, output);
        None
    }
}

/// A ledger lent to the host functions of one analysis, for as long as it runs: the host functions are linked once per
/// compiled module and keep what they reach behind `'static` handles, so they get this handle rather than a reference.
/// Once the analysis ended, the handle reads nothing.
#[cfg(feature = "plugin_runtime")]
#[derive(Clone)]
pub struct LentLedger(Arc<std::sync::Mutex<Option<LedgerPointer>>>);

/// Where the ledger lent is.
#[cfg(feature = "plugin_runtime")]
struct LedgerPointer(std::ptr::NonNull<Ledger>);

// SAFETY: the pointer only ever gives a shared reference to the ledger, which is `Sync`, and only while the `Lend` that
// set it borrows the ledger (see `LentLedger::with`)
#[cfg(feature = "plugin_runtime")]
unsafe impl Send for LedgerPointer {}
#[cfg(feature = "plugin_runtime")]
unsafe impl Sync for LedgerPointer {}

#[cfg(feature = "plugin_runtime")]
impl LentLedger {
    /// `read` with the ledger, while it is lent; `None` once the analysis it was lent for ended.
    pub fn with<T>(&self, read: impl FnOnce(&Ledger) -> T) -> Option<T> {
        let lent = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the pointer is set from a `&'l Ledger` by `Lend::new` and cleared, under this lock, when that `Lend`
        // drops, which it does before the borrow `'l` ends; so it points to a live ledger that nothing mutates for as
        // long as the lock is held here
        lent.as_ref().map(|ledger| read(unsafe { ledger.0.as_ref() }))
    }
}

/// The loan of a ledger to an analysis: its handle reads the ledger until this drops.
#[cfg(feature = "plugin_runtime")]
pub(crate) struct Lend<'l> {
    lent: LentLedger,
    _ledger: std::marker::PhantomData<&'l Ledger>,
}

#[cfg(feature = "plugin_runtime")]
impl<'l> Lend<'l> {
    pub(crate) fn new(ledger: &'l Ledger) -> Lend<'l> {
        Lend {
            lent: LentLedger(Arc::new(std::sync::Mutex::new(Some(LedgerPointer(std::ptr::NonNull::from(ledger)))))),
            _ledger: std::marker::PhantomData,
        }
    }

    pub(crate) fn handle(&self) -> LentLedger {
        self.lent.clone()
    }
}

#[cfg(feature = "plugin_runtime")]
impl Drop for Lend<'_> {
    fn drop(&mut self) {
        *self.lent.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The tables an analyzer declares, for the catalog: its manifest under its provider's name.
#[derive(Debug, Clone, Copy)]
pub struct AnalyzerTables<'a> {
    /// the plugin's name, the `provided_by` of its tables
    pub provider: &'a str,
    pub manifest: &'a abi::Manifest,
    /// whether it is `zhang:budget`, which keeps the budget system's names
    pub legacy: bool,
}

/// The catalog a host built for a ledger.
pub struct AnalysisCatalog {
    /// the query engine's catalog
    pub catalog: Arc<dyn Any + Send + Sync>,
    /// the providers of the analyzers the catalog has tables of, in order: those that run
    pub analyzers: Vec<String>,
    /// the analyzers it has no table of, by their position among those it was built for, and why
    pub skipped: Vec<(usize, String)>,
}

/// How a ledger loads: within what the operator lets its plugins do, with the host of their analyses, and with the
/// plugins the embedder registers for every ledger. A reload loads with the same options.
#[derive(Clone, Default)]
pub struct LoadOptions {
    pub plugin_policy: Arc<PluginPolicy>,
    /// the host of the analyzers' analyses; without one, an analyzer of the ledger does not run, and the ledger says so
    pub analysis: Option<Arc<dyn AnalysisHost>>,
    /// plugins registered for every ledger before its own, without a `plugin` directive and whether or not the ledger
    /// turns plugins on, within the operator's policy: an official plugin such as `zhang:budget`
    pub default_plugins: Vec<DefaultPlugin>,
}

impl LoadOptions {
    /// The options of a load whose plugins `policy` bounds, with no analysis host and no default plugin.
    pub fn with_policy(plugin_policy: Arc<PluginPolicy>) -> LoadOptions {
        LoadOptions {
            plugin_policy,
            ..LoadOptions::default()
        }
    }
}

impl fmt::Debug for LoadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadOptions")
            .field("plugin_policy", &self.plugin_policy)
            .field("analysis", &self.analysis.is_some())
            .field("default_plugins", &self.default_plugins)
            .finish()
    }
}

/// A plugin the embedder registers for every ledger ([`LoadOptions::default_plugins`]).
#[derive(Clone)]
pub struct DefaultPlugin {
    /// the module it is registered as, such as `zhang:budget`: what its errors name
    pub module: String,
    /// the file it is built from, whose extension says what it is (`.wasm`, `.lua`)
    pub file: String,
    pub content: Arc<[u8]>,
}

impl DefaultPlugin {
    /// `zhang:budget` as its bundled Lua script (`plugins/budget/budget.lua`, RFC #698): registered, it publishes the
    /// budget tables and reports the budget errors instead of the native provider and the native checks.
    #[cfg(feature = "plugin_runtime")]
    pub fn budget() -> DefaultPlugin {
        let script = &crate::plugin::bundled::BUDGET;
        DefaultPlugin {
            module: BUDGET.to_owned(),
            file: script.file.to_owned(),
            content: Arc::from(script.content),
        }
    }
}

impl fmt::Debug for DefaultPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultPlugin")
            .field("module", &self.module)
            .field("file", &self.file)
            .field("bytes", &self.content.len())
            .finish()
    }
}

/// Why an analysis failed: the `reason` meta of its error, and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisFailure {
    /// `manifest`, `timeout`, `memory`, `trap`, `output` or `unavailable`
    pub reason: &'static str,
    pub message: String,
}

impl AnalysisFailure {
    pub fn new(reason: &'static str, message: impl Into<String>) -> AnalysisFailure {
        AnalysisFailure {
            reason,
            message: message.into(),
        }
    }
}

/// Where a diagnostic is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// the directive at this index of [`Ledger::directives`]
    Directive(usize),
    /// the plugin's directive, or for a default plugin the first directive it reads
    Plugin,
}

/// A diagnostic of an analysis, checked: its kind as the ledger reports it, where, in which phase, and its metas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: ErrorKind,
    pub place: Place,
    pub phase: Phase,
    pub metas: HashMap<String, String>,
}

/// The kind of record a directive is for an analyzer, if it is one: see [`abi::Record::kind`].
pub fn record_kind(directive: &Directive) -> Option<String> {
    Some(match directive {
        Directive::Budget(_) => "budget".to_owned(),
        Directive::BudgetAdd(_) => "budget-add".to_owned(),
        Directive::BudgetTransfer(_) => "budget-transfer".to_owned(),
        Directive::BudgetClose(_) => "budget-close".to_owned(),
        Directive::Custom(custom) => format!("custom:{}", custom.custom_type.as_str()),
        _ => return None,
    })
}

/// The fields of the record of `directive` (see [`abi::Record::fields`]).
fn record_fields(directive: &Directive) -> Map<String, serde_json::Value> {
    let amount = |amount: &zhang_ast::amount::Amount| json!({"number": zhang_shared::decimal::plain_decimal(&amount.number), "currency": amount.commodity});
    let fields = match directive {
        Directive::Budget(budget) => json!({"name": budget.name, "commodity": budget.commodity}),
        Directive::BudgetAdd(add) => json!({"name": add.name, "amount": amount(&add.amount)}),
        Directive::BudgetTransfer(transfer) => json!({"from": transfer.from, "to": transfer.to, "amount": amount(&transfer.amount)}),
        Directive::BudgetClose(close) => json!({"name": close.name}),
        Directive::Custom(custom) => {
            let values = custom
                .values
                .iter()
                .map(|value| match value {
                    zhang_ast::StringOrAccount::String(text) => text.as_str().to_owned(),
                    zhang_ast::StringOrAccount::Account(account) => account.name().to_owned(),
                })
                .collect::<Vec<_>>();
            json!({ "values": values })
        }
        _ => json!({}),
    };
    match fields {
        serde_json::Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

/// `date` as `%Y-%m-%d` writes it, without parsing the format: the years of four digits, the others as chrono writes them.
fn ymd(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    match date.year() {
        0..=9999 => format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
        _ => date.format("%Y-%m-%d").to_string(),
    }
}

/// `time` as `%H:%M:%S` writes it, without parsing the format; a leap second as chrono writes it.
fn hms(time: chrono::NaiveTime) -> String {
    use chrono::Timelike;
    match time.nanosecond() < 1_000_000_000 {
        true => format!("{:02}:{:02}:{:02}", time.hour(), time.minute(), time.second()),
        false => time.format("%H:%M:%S").to_string(),
    }
}

/// The date of a record's directive.
fn record_date(directive: &Directive) -> Option<&Date> {
    match directive {
        Directive::Budget(it) => Some(&it.date),
        Directive::BudgetAdd(it) => Some(&it.date),
        Directive::BudgetTransfer(it) => Some(&it.date),
        Directive::BudgetClose(it) => Some(&it.date),
        Directive::Custom(it) => Some(&it.date),
        _ => None,
    }
}

/// What the analyzer declaring `manifest` reads of `ledger`: the records of the kinds it consumes, in the order zhang
/// processed the ledger (`seq`), and what the ledger is kept in.
pub fn input(ledger: &Ledger, manifest: &abi::Manifest) -> abi::Input {
    let timezone = &ledger.options.timezone;
    let mut records = vec![];
    // the file of the record before, as a record names it: the directives of a file follow each other
    let mut file_before: Option<(&std::path::Path, String)> = None;
    for (directive, outcome) in ledger.directives.iter().zip(&ledger.outcomes) {
        let (Some(seq), Some(kind)) = (outcome.seq, record_kind(&directive.data)) else {
            continue;
        };
        if !manifest.consumes.contains(&kind) {
            continue;
        }
        let Some(date) = record_date(&directive.data) else { continue };
        let at = date.to_timezone_datetime(timezone);
        let mut meta: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(directive_meta) = directive.data.meta() {
            for (key, value) in directive_meta.clone().get_flatten() {
                meta.entry(key).or_default().push(value.as_str().to_owned());
            }
        }
        let file = directive.span.filename.as_deref().map(|file| match &file_before {
            Some((before, name)) if *before == file => name.clone(),
            _ => {
                let name = ledger
                    .path_in_ledger(file)
                    .map_or_else(|| file.display().to_string(), |it| crate::data_source::slashed(&it));
                file_before = Some((file, name.clone()));
                name
            }
        });
        records.push(abi::Record {
            kind,
            seq,
            date: ymd(date.naive_date()),
            time: hms(at.time()),
            has_time: !matches!(date, Date::Date(_)),
            timestamp: at.timestamp(),
            fields: record_fields(&directive.data),
            meta,
            file,
            line: directive.span.line,
        });
    }
    // records are in ledger order already, which is `seq` order but for the checks of `balance ... with pad`, which
    // are no records
    records.sort_by_key(|record| record.seq);
    abi::Input {
        records,
        ledger: LedgerInfo {
            title: ledger.options.option::<String>("title").ok().flatten(),
            operating_currency: ledger.options.operating_currency.clone(),
            timezone: ledger.options.timezone.name().to_owned(),
        },
        last_transaction: last_transaction(ledger),
    }
}

/// The last transaction of `ledger`, of the latest date the one the load processed last: the last one it accepted
/// (it has a `seq`), as the directives are in the order of their dates.
fn last_transaction(ledger: &Ledger) -> Option<abi::EntryAt> {
    ledger
        .directives
        .iter()
        .zip(&ledger.outcomes)
        .rev()
        .find_map(|(directive, outcome)| match (&directive.data, outcome.seq) {
            (Directive::Transaction(transaction), Some(seq)) => Some(abi::EntryAt {
                seq,
                date: transaction
                    .date
                    .to_timezone_datetime(&ledger.options.timezone)
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string(),
            }),
            _ => None,
        })
}

/// Whether `ledger` gives the analyzer declaring `manifest` something to read: a record of a kind it consumes, or an
/// `open` with metadata it reads. Otherwise it is not called, and its tables are empty.
pub fn reads_something(ledger: &Ledger, manifest: &abi::Manifest, input: &abi::Input) -> bool {
    !input.records.is_empty() || manifest.open_meta.iter().any(|key| !ledger.open_meta_accounts(key).is_empty())
}

/// The place a diagnostic about the analyzer itself goes to: the span of its `plugin` directive, or for a default plugin
/// (which has none) the first directive it reads: a record of a kind it consumes, or an `open` with metadata it reads.
pub fn plugin_span(ledger: &Ledger, directive_span: &SpanInfo, manifest: Option<&abi::Manifest>) -> Option<SpanInfo> {
    if directive_span.filename.is_some() || !directive_span.content.is_empty() {
        return Some(directive_span.clone());
    }
    let manifest = manifest?;
    ledger
        .directives
        .iter()
        .find(|directive| match &directive.data {
            Directive::Open(open) => manifest.open_meta.iter().any(|key| open.meta.get_one(key).is_some()),
            data => record_kind(data).is_some_and(|kind| manifest.consumes.contains(&kind)),
        })
        .map(|directive| directive.span.clone())
}

/// The diagnostics `reported`, of the analyzer `plugin` declaring `manifest` (`legacy` for `zhang:budget`), checked: a
/// kind its manifest declares, in the phase it declares; a target that is an entry of `ledger`, a transaction of it, or
/// the plugin. A legacy budget kind of `zhang:budget` is reported as it is; any other kind as a
/// [`ErrorKind::PluginError`] with the metas `plugin`, `kind` and `message`. Past [`MAX_DIAGNOSTICS`], one more on the
/// plugin says how many were left out. An error names the first diagnostic that does not hold.
pub fn diagnostics(ledger: &Ledger, plugin: &str, manifest: &abi::Manifest, legacy: bool, reported: &[abi::Diagnostic]) -> Result<Vec<Diagnostic>, String> {
    let directive_of_seq = ledger
        .outcomes
        .iter()
        .enumerate()
        .filter_map(|(index, outcome)| Some((outcome.seq?, index)))
        .collect::<HashMap<_, _>>();
    let mut checked = Vec::with_capacity(reported.len().min(MAX_DIAGNOSTICS + 1));
    for (at, diagnostic) in reported.iter().enumerate() {
        let what = || format!("diagnostic {} ({})", at + 1, diagnostic.kind);
        let declared = manifest
            .diagnostics
            .iter()
            .find(|declared| declared.kind == diagnostic.kind)
            .ok_or_else(|| format!("{}: its manifest declares no diagnostic of this kind", what()))?;
        let target = &diagnostic.target;
        let place = match (target.seq, target.txn.as_deref(), target.plugin) {
            (Some(seq), None, None) => Place::Directive(
                *directive_of_seq
                    .get(&seq)
                    .ok_or_else(|| format!("{}: its target is the entry {}, which the ledger does not have", what(), seq))?,
            ),
            (None, Some(txn), None) => {
                let id = Uuid::from_str(txn).map_err(|_| format!("{}: its target is the transaction {:?}, which is no id", what(), txn))?;
                Place::Directive(
                    ledger
                        .directive_index(&id)
                        .ok_or_else(|| format!("{}: its target is the transaction {}, which the ledger does not have", what(), txn))?,
                )
            }
            (None, None, Some(true)) => Place::Plugin,
            _ => {
                return Err(format!(
                    "{}: its target is one of {{\"seq\": n}}, {{\"txn\": id}} or {{\"plugin\": true}}",
                    what()
                ))
            }
        };
        if at == MAX_DIAGNOSTICS {
            let mut metas = HashMap::from([(PLUGIN_META.to_owned(), plugin.to_owned())]);
            metas.insert(
                MESSAGE_META.to_owned(),
                format!("the analysis reported {} diagnostics; the first {} are listed", reported.len(), MAX_DIAGNOSTICS),
            );
            checked.push(Diagnostic {
                kind: ErrorKind::PluginError,
                place: Place::Plugin,
                phase: Phase::AfterFold,
                metas,
            });
            continue;
        }
        if at > MAX_DIAGNOSTICS {
            continue;
        }
        if diagnostic.metas.len() > MAX_METAS {
            return Err(format!("{}: it has {} metas, more than {}", what(), diagnostic.metas.len(), MAX_METAS));
        }
        let legacy_kind = legacy && LEGACY_KINDS.contains(&diagnostic.kind.as_str());
        // the metas the load sets itself are not the plugin's to give: `zhang:budget` names the account of a budget
        // kind, as the native checks do
        let reserved = |key: &str| key == "txn_id" || (!legacy_kind && RESERVED_METAS.contains(&key));
        let mut metas = diagnostic
            .metas
            .iter()
            .filter(|(key, _)| !reserved(key))
            .map(|(key, value)| (cut(key), cut(value)))
            .collect::<HashMap<_, _>>();
        let kind = match ErrorKind::from_str(&diagnostic.kind) {
            Ok(kind) if legacy_kind => kind,
            _ => {
                metas.insert(PLUGIN_META.to_owned(), plugin.to_owned());
                metas.insert(KIND_META.to_owned(), cut(&diagnostic.kind));
                metas.insert(MESSAGE_META.to_owned(), cut(diagnostic.message.as_deref().unwrap_or(&diagnostic.kind)));
                ErrorKind::PluginError
            }
        };
        checked.push(Diagnostic {
            kind,
            place,
            phase: declared.phase,
            metas,
        });
    }
    Ok(checked)
}

/// `text`, cut at [`MAX_META_LEN`] bytes on a character boundary, with `…` when it was longer.
fn cut(text: &str) -> String {
    if text.len() <= MAX_META_LEN {
        return text.to_owned();
    }
    let end = (0..=MAX_META_LEN).rev().find(|at| text.is_char_boundary(*at)).unwrap_or(0);
    format!("{}…", &text[..end])
}

/// The error a failed analysis reports.
pub fn failure_error(plugin: &str, failure: &AnalysisFailure, span: &SpanInfo) -> ErrorDomain {
    let metas = HashMap::from([
        (PLUGIN_META.to_owned(), plugin.to_owned()),
        (REASON_META.to_owned(), failure.reason.to_owned()),
        (MESSAGE_META.to_owned(), failure.message.clone()),
    ]);
    ErrorDomain::new(ErrorKind::PluginAnalysisFailed, span, metas)
}

/// What the analyses of a load report, in the order of the analyzers, each in the order it reported: their diagnostics
/// at directives, by phase, and those about the analyzers themselves, failures included.
#[derive(Default)]
pub struct Reported {
    /// with the fold's errors of the directive at the index
    pub at_directives: BTreeMap<usize, Vec<ErrorDomain>>,
    /// after the fold's errors of every directive
    pub after_fold: Vec<ErrorDomain>,
    /// after both: on the analyzers' directives
    pub on_plugins: Vec<ErrorDomain>,
}

impl Reported {
    /// Whether the analyses reported nothing.
    pub fn is_empty(&self) -> bool {
        self.at_directives.is_empty() && self.after_fold.is_empty() && self.on_plugins.is_empty()
    }

    /// These, then `later`: at each directive, after the fold and on the analyzers.
    pub fn then(mut self, later: Reported) -> Reported {
        for (index, errors) in later.at_directives {
            self.at_directives.entry(index).or_default().extend(errors);
        }
        self.after_fold.extend(later.after_fold);
        self.on_plugins.extend(later.on_plugins);
        self
    }

    /// Add `diagnostics`, of an analyzer whose own diagnostics go to `plugin_span` (none for a default plugin the ledger
    /// gives nothing to read), of `directives`.
    pub fn add(&mut self, diagnostics: Vec<Diagnostic>, plugin_span: Option<&SpanInfo>, directives: &[Spanned<Directive>]) {
        for diagnostic in diagnostics {
            match (diagnostic.place, diagnostic.phase) {
                (Place::Directive(index), phase) => {
                    let error = ErrorDomain::new(diagnostic.kind, &directives[index].span, diagnostic.metas);
                    match phase {
                        Phase::Fold => self.at_directives.entry(index).or_default().push(error),
                        Phase::AfterFold => self.after_fold.push(error),
                    }
                }
                (Place::Plugin, _) => {
                    if let Some(span) = plugin_span {
                        self.on_plugins.push(ErrorDomain::new(diagnostic.kind, span, diagnostic.metas));
                    }
                }
            }
        }
    }
}

/// `errors`, the errors of a load in the order it reported them, with `reported` placed where the native checks report
/// theirs (RFC #698 §2.6), which fixes their ids: a diagnostic of the phase `fold` after the fold's errors of the
/// directive it names (which start at `marks[index]`; `marks` has one more, where the fold's errors end), so after those
/// of the directives before it and before those of the directives after it; then the fold's errors after its loop (the
/// checks that need every price); then the diagnostics of the phase `after_fold`; then those about the analyzers.
pub fn merge(errors: Vec<ErrorDomain>, marks: &[usize], mut reported: Reported) -> Vec<ErrorDomain> {
    let Some((&fold_end, starts)) = marks.split_last() else {
        return errors.into_iter().chain(reported.after_fold).chain(reported.on_plugins).collect();
    };
    let total = errors.len() + reported.after_fold.len() + reported.on_plugins.len() + reported.at_directives.values().map(Vec::len).sum::<usize>();
    let mut merged = Vec::with_capacity(total);
    let mut errors = errors.into_iter();
    let mut taken = 0;
    for (index, start) in starts.iter().enumerate() {
        debug_assert!(taken <= *start, "the fold's errors start in order");
        let end = starts.get(index + 1).copied().unwrap_or(fold_end);
        // the errors up to this directive's own end: those before the fold for the first one, and its own
        merged.extend(errors.by_ref().take(end - taken));
        taken = end;
        if let Some(diagnostics) = reported.at_directives.remove(&index) {
            merged.extend(diagnostics);
        }
    }
    merged.extend(errors);
    merged.extend(reported.after_fold);
    merged.extend(reported.on_plugins);
    merged
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use serde_json::json;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::SpanInfo;
    use zhang_shared::plugin_abi::analysis as abi;

    use super::{diagnostics, hms, input, merge, ymd, Place, Reported, MAX_DIAGNOSTICS};

    #[test]
    fn a_records_date_and_time_are_written_as_chrono_writes_them() {
        use chrono::{NaiveDate, NaiveTime};
        for (y, m, d) in [
            (2024, 1, 5),
            (1, 12, 31),
            (0, 2, 29),
            (999, 7, 4),
            (9999, 12, 31),
            (10000, 1, 1),
            (-1, 6, 15),
            (-12345, 3, 9),
        ] {
            let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
            assert_eq!(ymd(date), date.format("%Y-%m-%d").to_string());
        }
        for (h, m, s, nano) in [
            (0, 0, 0, 0),
            (9, 5, 7, 123),
            (23, 59, 59, 999_999_999),
            (23, 59, 59, 1_000_000_000),
            (12, 30, 59, 1_500_000_000),
        ] {
            let time = NaiveTime::from_hms_nano_opt(h, m, s, nano).unwrap();
            assert_eq!(hms(time), time.format("%H:%M:%S").to_string());
        }
    }
    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::domains::schemas::ErrorDomain;
    use crate::ledger::Ledger;

    fn load(text: &str) -> Ledger {
        let dir = tempfile::tempdir().unwrap().keep();
        std::fs::write(dir.join("main.zhang"), text).unwrap();
        Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))).unwrap()
    }

    const TEXT: &str = "option \"timezone\" \"Asia/Shanghai\"\n\
                        1970-01-01 open Expenses:Food\n  budget: Food\n\
                        2024-01-01 budget Food CNY\n  alias: \"Groceries\"\n  tag: \"a\"\n  tag: \"b\"\n\
                        2024-01-02 10:30:00 budget-add Food 500.00 CNY\n\
                        2024-01-03 budget-transfer Food Fun 1.5 USD\n\
                        2024-01-04 budget-close Food\n\
                        2024-01-05 custom \"shop\" \"apple\" Expenses:Food\n";

    fn manifest(consumes: &[&str]) -> abi::Manifest {
        serde_json::from_value(json!({
            "analysis_abi": 1,
            "consumes": consumes,
            "diagnostics": [{"kind": "BudgetClosed"}, {"kind": "Mine", "phase": "after_fold"}]
        }))
        .unwrap()
    }

    #[test]
    fn an_analyzer_reads_the_records_of_the_kinds_it_consumes_in_ledger_order() {
        let ledger = load(TEXT);
        let all = input(&ledger, &manifest(&["budget", "budget-add", "budget-transfer", "budget-close", "custom:shop"]));
        let records = serde_json::to_value(&all.records).unwrap();
        assert_eq!(
            records,
            json!([
                {"kind": "budget", "seq": 1, "date": "2024-01-01", "time": "00:00:00", "has_time": false, "timestamp": 1704038400,
                 "fields": {"name": "Food", "commodity": "CNY"}, "meta": {"alias": ["Groceries"], "tag": ["a", "b"]}, "file": "main.zhang", "line": 4},
                {"kind": "budget-add", "seq": 2, "date": "2024-01-02", "time": "10:30:00", "has_time": true, "timestamp": 1704162600,
                 "fields": {"name": "Food", "amount": {"number": "500.00", "currency": "CNY"}}, "meta": {}, "file": "main.zhang", "line": 8},
                {"kind": "budget-transfer", "seq": 3, "date": "2024-01-03", "time": "00:00:00", "has_time": false, "timestamp": 1704211200,
                 "fields": {"from": "Food", "to": "Fun", "amount": {"number": "1.5", "currency": "USD"}}, "meta": {}, "file": "main.zhang", "line": 9},
                {"kind": "budget-close", "seq": 4, "date": "2024-01-04", "time": "00:00:00", "has_time": false, "timestamp": 1704297600,
                 "fields": {"name": "Food"}, "meta": {}, "file": "main.zhang", "line": 10},
                {"kind": "custom:shop", "seq": 5, "date": "2024-01-05", "time": "00:00:00", "has_time": false, "timestamp": 1704384000,
                 "fields": {"values": ["apple", "Expenses:Food"]}, "meta": {}, "file": "main.zhang", "line": 11}
            ])
        );
        assert_eq!(all.ledger.timezone, "Asia/Shanghai");
        let some = input(&ledger, &manifest(&["budget-close", "custom:other"]));
        assert_eq!(some.records.iter().map(|it| it.kind.as_str()).collect::<Vec<_>>(), ["budget-close"]);
    }

    fn reported(kind: &str, target: serde_json::Value) -> abi::Diagnostic {
        serde_json::from_value(json!({"kind": kind, "target": target, "metas": {"budget_name": "Food"}, "message": "why"})).unwrap()
    }

    #[test]
    fn a_diagnostic_has_a_declared_kind_and_a_target_of_the_ledger() {
        let ledger = load(TEXT);
        let manifest = manifest(&["budget"]);
        let checked = diagnostics(&ledger, "shop", &manifest, false, &[reported("BudgetClosed", json!({"seq": 4}))]).unwrap();
        // a legacy kind of any plugin but zhang:budget is the plugin's own kind
        assert_eq!(checked[0].kind, ErrorKind::PluginError);
        assert_eq!(checked[0].place, Place::Directive(4));
        let mut metas = checked[0].metas.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>();
        metas.sort();
        assert_eq!(metas, ["budget_name=Food", "kind=BudgetClosed", "message=why", "plugin=shop"]);
        let legacy = diagnostics(&ledger, "zhang:budget", &manifest, true, &[reported("BudgetClosed", json!({"plugin": true}))]).unwrap();
        assert_eq!(legacy[0].kind, ErrorKind::BudgetClosed);
        assert_eq!(legacy[0].place, Place::Plugin);
        assert_eq!(legacy[0].metas.len(), 1, "a legacy kind keeps its metas as they are");
        let own = diagnostics(&ledger, "zhang:budget", &manifest, true, &[reported("Mine", json!({"seq": 1}))]).unwrap();
        assert_eq!((own[0].kind.clone(), own[0].phase), (ErrorKind::PluginError, abi::Phase::AfterFold));
        for (diagnostic, error) in [
            (
                reported("Other", json!({"seq": 1})),
                "diagnostic 1 (Other): its manifest declares no diagnostic of this kind",
            ),
            (
                reported("Mine", json!({"seq": 99})),
                "diagnostic 1 (Mine): its target is the entry 99, which the ledger does not have",
            ),
            (
                reported("Mine", json!({"txn": "nope"})),
                "diagnostic 1 (Mine): its target is the transaction \"nope\", which is no id",
            ),
            (
                reported("Mine", json!({"txn": "6f1a0f1d-6c1e-4e0f-8b1a-2c3d4e5f6a7b"})),
                "diagnostic 1 (Mine): its target is the transaction 6f1a0f1d-6c1e-4e0f-8b1a-2c3d4e5f6a7b, which the ledger does not have",
            ),
            (
                reported("Mine", json!({"seq": 1, "plugin": true})),
                "diagnostic 1 (Mine): its target is one of {\"seq\": n}, {\"txn\": id} or {\"plugin\": true}",
            ),
            (reported("Mine", json!({})), "diagnostic 1 (Mine): its target is one of"),
        ] {
            let err = diagnostics(&ledger, "shop", &manifest, false, &[diagnostic]).unwrap_err();
            assert!(err.starts_with(error), "{err}\n  expected: {error}");
        }
        // past the limit, one more says how many were left out
        let many = vec![reported("Mine", json!({"seq": 1})); MAX_DIAGNOSTICS + 5];
        let checked = diagnostics(&ledger, "shop", &manifest, false, &many).unwrap();
        assert_eq!(checked.len(), MAX_DIAGNOSTICS + 1);
        assert_eq!(checked[MAX_DIAGNOSTICS].place, Place::Plugin);
        assert_eq!(
            checked[MAX_DIAGNOSTICS].metas["message"],
            "the analysis reported 10005 diagnostics; the first 10000 are listed"
        );
    }

    fn error(name: &str) -> ErrorDomain {
        let span = SpanInfo {
            content: name.to_owned(),
            ..SpanInfo::default()
        };
        ErrorDomain::new(ErrorKind::PluginError, &span, [("n".to_owned(), name.to_owned())].into_iter().collect())
    }

    #[test]
    fn diagnostics_go_with_the_fold_errors_of_their_directives_then_after_the_fold() {
        // before the fold: l0; directive 0: f0; directive 1: none; directive 2: f2a, f2b; after the loop: a0
        let errors = ["l0", "f0", "f2a", "f2b", "a0"].map(error).to_vec();
        let marks = [1, 2, 2, 4];
        let mut reported = Reported::default();
        reported.at_directives.insert(0, vec![error("d0")]);
        reported.at_directives.insert(1, vec![error("d1a"), error("d1b")]);
        reported.at_directives.insert(2, vec![error("d2")]);
        reported.after_fold.push(error("after"));
        reported.on_plugins.push(error("plugin"));
        let merged = merge(errors, &marks, reported);
        let names = merged.iter().map(|it| it.metas["n"].as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["l0", "f0", "d0", "d1a", "d1b", "f2a", "f2b", "d2", "a0", "after", "plugin"]);
        // a load without a fold of directives
        let merged = merge(vec![error("l0")], &[], Reported::default());
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn reports_follow_one_another_at_every_place() {
        let mut first = Reported::default();
        first.at_directives.insert(1, vec![error("n1")]);
        first.after_fold.push(error("n-after"));
        let mut later = Reported::default();
        later.at_directives.insert(1, vec![error("d1")]);
        later.at_directives.insert(2, vec![error("d2")]);
        later.after_fold.push(error("d-after"));
        later.on_plugins.push(error("failed"));
        let both = first.then(later);
        let names = |errors: &[ErrorDomain]| errors.iter().map(|it| it.metas["n"].clone()).collect::<Vec<_>>();
        assert_eq!(names(&both.at_directives[&1]), ["n1", "d1"]);
        assert_eq!(names(&both.at_directives[&2]), ["d2"]);
        assert_eq!(names(&both.after_fold), ["n-after", "d-after"]);
        assert_eq!(names(&both.on_plugins), ["failed"]);
    }

    #[cfg(feature = "plugin_runtime")]
    #[test]
    fn a_lent_ledger_reads_nothing_once_the_loan_ended() {
        use super::Lend;

        let ledger = load(TEXT);
        let handle = {
            let lend = Lend::new(&ledger);
            let handle = lend.handle();
            assert_eq!(handle.with(|it| it.directives.len()), Some(ledger.directives.len()));
            handle
        };
        assert_eq!(handle.with(|it| it.directives.len()), None);
    }
}
