use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, RwLock};

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use itertools::Itertools;
use log::{error, info};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalancePad, Date, Directive, Flag, Options, Plugin, SpanInfo, Spanned};

use crate::booking::Booker;
use crate::clock::{Clock, LoadClock};
use crate::data_source::DataSource;
use crate::derived::Derived;
use crate::domains::Operations;
use crate::error::IoErrorIntoZhangError;
use crate::inputs::ExtraInput;
use crate::options::{BuiltinOption, InMemoryOptions};
use crate::pipeline::{
    builtin_stages, run_pipeline, AssertionOutcome, AssertionOutcomes, BookingStage, FinalValidation, PluginStage, ProcessStage, StageContext,
};
use crate::process::{DirectivePreProcess, DirectiveProcess};
use crate::store::{BalanceAssertionDomain, CommodityLotRecord, Store};
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

/// stages with the slot ([`PluginStage`]) they run in, relative to the booking stage
type SlottedStages = Vec<(PluginStage, Box<dyn ProcessStage>)>;

pub struct Ledger {
    pub entry: (PathBuf, String),

    pub data_source: Arc<dyn DataSource>,

    /// the ledger's own files; the file editor lists them
    pub visited_files: Vec<PathBuf>,

    /// what the load read besides [`Ledger::visited_files`] (plugin modules, files plugins read, the date), each once
    /// in the order first recorded. A change to any of them makes the ledger stale
    pub extra_inputs: IndexSet<ExtraInput>,

    pub options: InMemoryOptions,

    pub directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,

    pub store: Arc<RwLock<Store>>,

    pub(crate) trx_counter: AtomicI32,

    /// Final stage results consumed while materializing the store.
    validation: Option<FinalValidation>,

    /// Names defined so far, solely for budget validation during the store fold. The query engine
    /// computes budget figures from directives and booked postings; no budget totals are stored.
    pub(crate) defined_budgets: Option<HashSet<String>>,

    /// the (account, budget) pairs whose undefined budget the store fold already reported
    pub(crate) reported_undefined_budgets: HashSet<(String, String)>,

    /// the clock of this load, read at most once, on first use; a reload starts a new reading of the same [`Clock`]
    pub(crate) clock: LoadClock,

    /// what readers compute from this ledger once and keep with it, such as the query engine's booked postings; a
    /// reload replaces the ledger, and starts this empty
    pub derived: Derived,

    /// whether the ledger may no longer be what its files hold: they were written, or found changed, since it was
    /// loaded. A writer reloads it first, to read the files as they are, and edit the places they have now
    pub stale: bool,

    #[cfg(feature = "plugin_runtime")]
    pub plugins: crate::plugin::store::PluginStore,
}

pub struct LedgerProcessContext {
    pub directives: Vec<Spanned<Directive>>,
    pub entry: (PathBuf, String),
    pub visited_files: Vec<PathBuf>,
    pub data_source: Arc<dyn DataSource>,
    /// where the load reads the current time from, if anything asks for it (a plugin calling `zhang_now`).
    /// Nothing reads [`Clock::System`] unless something asks, so it is the right choice even on targets without
    /// a system clock, as long as they run no plugins. The `zhang-query` counterpart is `execute`/`execute_at`
    pub clock: Clock,
}

struct SplitDirectives {
    options_directives: Vec<(Options, SpanInfo)>,
    plugin_directives: Vec<(Plugin, SpanInfo)>,
    /// the whole stream incl. option/plugin directives — what pipeline stages see.
    /// options/plugins are *handled* before stages run; they stay visible in the
    /// stream so stages get the same full-stream contract as beancount plugins.
    full_stream: Vec<Spanned<Directive>>,
}

impl SplitDirectives {
    fn new(directives: Vec<Spanned<Directive>>) -> Self {
        // split directive into two groups.
        // first is meta which is no date
        // second is dated directives
        let (meta_directives, dated_directive): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) =
            directives.into_iter().partition(|it| it.datetime().is_none());

        let dated_directives = Ledger::sort_directives_datetime(dated_directive);

        // find all options which are not defined by users
        let options_key: HashSet<Cow<str>> = meta_directives
            .iter()
            .filter_map(|it| match &it.data {
                Directive::Option(option) => Some(Cow::Borrowed(option.key.as_str())),
                _ => None,
            })
            .collect();

        // merge built-in options and user-defined options, then the dated directives
        let full_stream = BuiltinOption::default_options(options_key)
            .into_iter()
            .chain(meta_directives)
            .chain(dated_directives)
            .collect_vec();

        // extract plugins and options to handle them before the pipeline
        let mut options_directives = vec![];
        let mut plugin_directives = vec![];
        for directive in full_stream.iter() {
            match &directive.data {
                Directive::Plugin(plugin) => plugin_directives.push((plugin.clone(), directive.span.clone())),
                Directive::Option(option) => options_directives.push((option.clone(), directive.span.clone())),
                _ => {}
            }
        }

        Self {
            options_directives,
            plugin_directives,
            full_stream,
        }
    }
}

impl Ledger {
    pub fn load_with_data_source(entry: PathBuf, endpoint: String, data_source: Arc<dyn DataSource>) -> ZhangResult<Ledger> {
        let entry = entry.canonicalize().with_path(&entry)?;

        let load_result = data_source.load(entry.to_string_lossy().to_string(), endpoint.clone())?;
        Ledger::process(LedgerProcessContext {
            directives: load_result.directives,
            entry: (entry, endpoint),
            visited_files: load_result.visited_files,
            data_source,
            clock: Clock::System,
        })
    }
    pub async fn async_load(entry: PathBuf, endpoint: String, data_source: Arc<dyn DataSource>) -> ZhangResult<Ledger> {
        Ledger::async_load_with_clock(entry, endpoint, data_source, Clock::System).await
    }

    /// [`Ledger::async_load`] with the current time read from `clock`: [`Clock::Fixed`] pins "today" for the
    /// load, its reloads and everything that asks the ledger for the time ([`Ledger::now`])
    pub async fn async_load_with_clock(entry: PathBuf, endpoint: String, data_source: Arc<dyn DataSource>, clock: Clock) -> ZhangResult<Ledger> {
        let load_result = data_source.async_load(entry.to_string_lossy().to_string(), endpoint.clone()).await?;

        Ledger::async_process(LedgerProcessContext {
            directives: load_result.directives,
            entry: (entry, endpoint),
            visited_files: load_result.visited_files,
            data_source,
            clock,
        })
        .await
    }

    /// The current time by the ledger's [`Clock`]: the system's, or the instant a [`Clock::Fixed`] pins. Unlike
    /// the load's own reading, which stays the same for the whole load, it is read anew on every call, so a server
    /// that keeps a ledger loaded for days sees the date change.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.clock().read()
    }

    /// Today's date in the ledger's timezone, by [`Ledger::now`].
    pub fn today(&self) -> NaiveDate {
        self.now().with_timezone(&self.options.timezone).date_naive()
    }

    fn init(context: LedgerProcessContext) -> (Self, SplitDirectives) {
        let ledger = Self {
            options: InMemoryOptions::default(),
            entry: context.entry,
            visited_files: context.visited_files,
            extra_inputs: IndexSet::new(),
            directives: vec![],
            metas: vec![],
            data_source: context.data_source,
            store: Default::default(),
            trx_counter: AtomicI32::new(1),
            validation: None,
            defined_budgets: None,
            reported_undefined_budgets: HashSet::new(),
            clock: LoadClock::new(context.clock),
            derived: Derived::default(),
            stale: false,
            #[cfg(feature = "plugin_runtime")]
            plugins: crate::plugin::store::PluginStore::default(),
        };
        let split = SplitDirectives::new(context.directives);
        (ledger, split)
    }

    pub fn process(context: LedgerProcessContext) -> ZhangResult<Ledger> {
        let (mut ret_ledger, mut split) = Ledger::init(context);
        ret_ledger.handle_options(&mut split.options_directives)?;
        ret_ledger.handle_plugins_pre_process(&mut split.plugin_directives)?;
        ret_ledger.finish_process(split)
    }

    async fn async_process(context: LedgerProcessContext) -> ZhangResult<Ledger> {
        let (mut ret_ledger, mut split) = Ledger::init(context);
        ret_ledger.handle_options(&mut split.options_directives)?;
        ret_ledger.async_handle_plugins_pre_process(&mut split.plugin_directives).await?;
        ret_ledger.finish_process(split)
    }

    /// the shared tail of `process`/`async_process`, after plugin modules have been fetched
    fn finish_process(mut self, split: SplitDirectives) -> ZhangResult<Ledger> {
        let SplitDirectives {
            options_directives: _,
            mut plugin_directives,
            full_stream,
        } = split;
        self.handle_plugins(&mut plugin_directives)?;

        // the pipeline always runs (built-in stages at least); `directives`/`metas`
        // reflect its output, so the store and the directive list agree
        let stage_error_start = self.operations().read().errors.len();
        let (processed, assertions, validation) = self.run_stages(full_stream)?;
        self.validation = Some(validation);
        let (metas, mut dated) = Ledger::partition_processed_directives(processed);
        self.handle_other_directives(&mut dated, assertions)?;
        let validation = self.validation.take().expect("validation results are consumed during materialization");
        let stage_error_count = validation.errors.len();
        let mut operations = self.operations();
        operations.write().commodity_lots = validation.lots;
        for error in validation.errors {
            operations.new_error(error.kind, &error.span, error.metas)?;
        }
        // Stage errors precede materialization errors. Delay insertion until transaction IDs have
        // been bound, then move them before the fold's errors without disturbing either group.
        operations.write().errors[stage_error_start..].rotate_right(stage_error_count);
        self.metas = metas;
        self.directives = dated;

        let mut operations = self.operations();
        let errors = operations.errors()?;
        if !errors.is_empty() {
            error!("Ledger loaded with {} error", errors.len());
        } else {
            info!("Ledger loaded");
        }
        Ok(self)
    }

    pub fn reload(&mut self) -> ZhangResult<()> {
        let (entry, endpoint) = &mut self.entry;
        let transform_result = self.data_source.load(entry.to_string_lossy().to_string(), endpoint.clone())?;
        let reload_ledger = Ledger::process(LedgerProcessContext {
            directives: transform_result.directives,
            entry: (entry.clone(), endpoint.clone()),
            visited_files: transform_result.visited_files,
            data_source: self.data_source.clone(),
            clock: self.clock.clock(),
        })?;
        *self = reload_ledger;
        Ok(())
    }

    pub async fn async_reload(&mut self) -> ZhangResult<()> {
        let (entry, endpoint) = &mut self.entry;
        let transform_result = self.data_source.async_load(entry.to_string_lossy().to_string(), endpoint.clone()).await?;
        let reload_ledger = Ledger::async_process(LedgerProcessContext {
            directives: transform_result.directives,
            entry: (entry.clone(), endpoint.clone()),
            visited_files: transform_result.visited_files,
            data_source: self.data_source.clone(),
            clock: self.clock.clock(),
        })
        .await?;
        *self = reload_ledger;
        Ok(())
    }

    /// the lots every account holds at the end of `day`: those of the transactions dated on it or before, booked as
    /// the load books them. A transaction dated after it, such as a sale planned ahead, is left out.
    ///
    /// The directives hold the booked postings, and booking a booked transaction again changes nothing, so this
    /// replay gives exactly the lots the final validation stage (pass 2) built from the same transactions
    pub fn lots_at_end_of(&self, day: NaiveDate) -> HashMap<String, Vec<CommodityLotRecord>> {
        let mut booker = Booker::new(self.options.default_booking_method);
        for commodity in self.operations().read().commodities.values() {
            booker.define_commodity(&commodity.name, commodity.precision, commodity.rounding);
        }
        for directive in &self.directives {
            match &directive.data {
                Directive::Open(open) => {
                    booker.apply_open(open);
                }
                Directive::Transaction(transaction) if transaction.date.naive_date() <= day => {
                    booker.book(&mut transaction.clone());
                }
                _ => {}
            }
        }
        booker.into_lots()
    }

    pub fn operations(&self) -> Operations {
        let timezone = self.options.timezone;
        Operations {
            store: self.store.clone(),
            timezone,
        }
    }

    /// the clock this ledger was loaded with; a reload reads the same clock again
    pub fn clock(&self) -> Clock {
        self.clock.clock()
    }

    /// the current time the load used, in the ledger timezone; `None` when nothing asked for it, and the load never
    /// read the clock. The load depends on the date only when [`Ledger::extra_inputs`] holds
    /// [`ExtraInput::Clock`]: a plugin reading the time while it registers is not recorded
    pub fn clock_reading(&self) -> Option<DateTime<Tz>> {
        self.clock.reading().map(|it| it.with_timezone(&self.options.timezone))
    }
}

impl Ledger {
    /// sort the stream by the key `(datetime, rank)` of [`Ledger::sort_keys`], stable — directives with an
    /// equal key keep their source order. The key of a directive depends on the stream only through the
    /// `pad` directives in it, and sorting changes no key, so sorting is idempotent and well-defined for any
    /// input.
    ///
    /// - undated directives (option, plugin, include, comment) come first
    /// - within one datetime: `open` and `commodity` (rank 0, together so that an
    ///   `open` naming a same-day commodity keeps its source order), then balance
    ///   entries (rank 1), then everything else (rank 2)
    /// - balance entries are balance pad/check directives *and* transactions
    ///   flagged `P` — the padding transactions balance pads materialize into. The
    ///   pad stage inserting one right after its directive can therefore rely on it
    ///   staying there. Hand-written `P` transactions are balance entries too. A
    ///   balance check materializes into nothing: it changes no balance
    /// - a `pad` is no balance entry: like beancount, which sorts a day's balances
    ///   before its pads, every balance entry of the day of a `pad` comes before it,
    ///   whatever their times. A `pad` sorts at the time of the last balance entry of
    ///   its day when that is later than its own. The padding transaction of a `pad`,
    ///   which carries the span of its `pad`, is no balance entry either: it sorts with
    ///   its `pad`, and stays right after it, where the pad stage puts it
    pub(crate) fn sort_directives_datetime(directives: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        let keys = Ledger::sort_keys(&directives);
        let mut keyed = keys.into_iter().zip(directives).collect::<Vec<_>>();
        // `sort_by_key` is stable; `None` (undated) sorts before any datetime
        keyed.sort_by_key(|(key, _)| *key);
        keyed.into_iter().map(|(_, directive)| directive).collect()
    }

    /// the key `(datetime, rank)` [`Ledger::sort_directives_datetime`] sorts each directive of `directives` by
    pub(crate) fn sort_keys(directives: &[Spanned<Directive>]) -> Vec<(Option<NaiveDateTime>, u8)> {
        // the places of the `pad` directives, which their padding transactions share
        let pads = directives
            .iter()
            .filter(|it| matches!(it.data, Directive::Pad(_)))
            .map(|it| (&it.span.filename, it.span.start, it.span.end))
            .collect::<HashSet<_>>();
        let rank = |directive: &Spanned<Directive>| match &directive.data {
            Directive::Open(_) | Directive::Commodity(_) => 0,
            Directive::Transaction(_) if pads.contains(&(&directive.span.filename, directive.span.start, directive.span.end)) => 2,
            data if Ledger::is_balance_entry(data) => 1,
            _ => 2,
        };
        let mut keys = directives.iter().map(|it| (it.datetime(), rank(it))).collect::<Vec<_>>();
        if pads.is_empty() {
            return keys;
        }
        // the time of the last balance entry of each day: the `pad`s of that day sort after it
        let mut last_balance_entries: HashMap<NaiveDate, NaiveDateTime> = HashMap::new();
        for (datetime, rank) in &keys {
            if let (Some(datetime), 1) = (datetime, rank) {
                let last = last_balance_entries.entry(datetime.date()).or_insert(*datetime);
                *last = (*last).max(*datetime);
            }
        }
        for (key, directive) in keys.iter_mut().zip(directives) {
            let Some(datetime) = key.0 else { continue };
            if key.1 == 2
                && (matches!(directive.data, Directive::Pad(_)) || pads.contains(&(&directive.span.filename, directive.span.start, directive.span.end)))
            {
                if let Some(last) = last_balance_entries.get(&datetime.date()) {
                    key.0 = Some(datetime.max(*last));
                }
            }
        }
        keys
    }

    /// whether a directive is a balance entry, which [`Ledger::sort_directives_datetime`] puts before everything but
    /// `open` and `commodity` within its datetime: a balance pad/check directive or a `P` transaction. The sort ranks
    /// the padding transaction of a `pad` with its `pad` instead, which comes right before it
    pub(crate) fn is_balance_entry(directive: &Directive) -> bool {
        match directive {
            Directive::BalancePad(_) | Directive::BalanceCheck(_) => true,
            Directive::Transaction(txn) => txn.flag == Some(Flag::BalancePad),
            _ => false,
        }
    }

    fn handle_options(&mut self, options_directives: &mut [(Options, SpanInfo)]) -> ZhangResult<()> {
        // handle option
        for (option, span) in options_directives.iter_mut() {
            option.handler(self, span)?;
        }
        Ok(())
    }

    fn handle_plugins_pre_process(&mut self, plugin_directives: &mut [(Plugin, SpanInfo)]) -> Result<(), ZhangError> {
        for (plugin, _) in plugin_directives.iter_mut() {
            plugin.pre_process(self)?;
        }
        Ok(())
    }
    async fn async_handle_plugins_pre_process(&mut self, plugin_directives: &mut [(Plugin, SpanInfo)]) -> Result<(), ZhangError> {
        for (plugin, _) in plugin_directives.iter_mut() {
            plugin.async_pre_process(self).await?;
        }
        Ok(())
    }
    fn handle_plugins(&mut self, plugin_directives: &mut [(Plugin, SpanInfo)]) -> Result<(), ZhangError> {
        for (plugin, span) in plugin_directives.iter_mut() {
            plugin.handler(self, span)?;
        }
        Ok(())
    }

    /// fold the pipeline's output into the store; `assertions` are what the balance-check stage found
    /// for the `balance` directives
    fn handle_other_directives(&mut self, directives: &mut [Spanned<Directive>], mut assertions: AssertionOutcomes) -> Result<(), ZhangError> {
        self.defined_budgets = Some(HashSet::new());
        // the `balance ... with pad` directives of the balance entries being folded: their checks are kept after
        // the last one, where the balance-check stage checked them, so they follow their padding in the journal
        let mut pads: Vec<(BalancePad, SpanInfo)> = vec![];
        let mut pads_at = None;
        for directive in directives.iter_mut() {
            if !pads.is_empty() && !(Ledger::is_balance_entry(&directive.data) && directive.datetime() == pads_at) {
                self.insert_pad_assertions(&mut pads, &mut assertions)?;
            }
            match &mut directive.data {
                // only dated directives reach the fold: options/plugins were handled
                // before the pipeline, and the undated arms below are unreachable
                Directive::Option(_) => {}
                Directive::Open(open) => open.handler(self, &directive.span)?,
                Directive::Close(close) => close.handler(self, &directive.span)?,
                Directive::Commodity(commodity) => commodity.handler(self, &directive.span)?,
                Directive::Transaction(trx) => trx.handler(self, &directive.span)?,
                // the pad stage materialized it into its padding transactions
                Directive::Pad(_) => {}
                // the pad stage materialized its padding into a transaction; its check is kept for the journal
                Directive::BalancePad(pad) => {
                    pads.push((pad.clone(), directive.span.clone()));
                    pads_at = directive.datetime();
                }
                // books nothing: the check is kept for the journal
                Directive::BalanceCheck(check) => {
                    if let Some(outcome) = assertions.take(&directive.span) {
                        self.insert_balance_assertion(&check.date, &check.account, &check.amount, check.tolerance.clone(), &directive.span, outcome)?;
                    }
                }
                Directive::Note(_) => {}
                Directive::Document(document) => document.handler(self, &directive.span)?,
                Directive::Price(price) => price.handler(self, &directive.span)?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                Directive::Query(query) => query.handler(self, &directive.span)?,
                Directive::Plugin(_) => {}
                Directive::Include(_) => {}
                Directive::Comment(_) => {}
                Directive::Budget(budget) => budget.handler(self, &directive.span)?,
                Directive::BudgetAdd(budget_add) => budget_add.handler(self, &directive.span)?,
                Directive::BudgetTransfer(budget_transfer) => budget_transfer.handler(self, &directive.span)?,
                Directive::BudgetClose(budget_close) => budget_close.handler(self, &directive.span)?,
            }
        }
        self.insert_pad_assertions(&mut pads, &mut assertions)?;
        self.defined_budgets = None;
        Ok(())
    }

    /// keep the checks of the `balance ... with pad` directives in `pads` in the store
    fn insert_pad_assertions(&mut self, pads: &mut Vec<(BalancePad, SpanInfo)>, assertions: &mut AssertionOutcomes) -> ZhangResult<()> {
        for (pad, span) in pads.drain(..) {
            if let Some(outcome) = assertions.take(&span) {
                self.insert_balance_assertion(&pad.date, &pad.account, &pad.amount, None, &span, outcome)?;
            }
        }
        Ok(())
    }

    /// keep a checked balance assertion in the store, in its place among the transactions
    fn insert_balance_assertion(
        &mut self, date: &Date, account: &Account, amount: &Amount, tolerance: Option<BigDecimal>, span: &SpanInfo, outcome: AssertionOutcome,
    ) -> ZhangResult<()> {
        let sequence = self.trx_counter.fetch_add(1, Ordering::Relaxed);
        let mut operations = self.operations();
        // the padding transaction of a `balance ... with pad` has the id of its span already
        let id = operations.unused_id(Uuid::from_span(span));
        operations.insert_balance_assertion(BalanceAssertionDomain {
            id,
            sequence,
            datetime: date.to_timezone_datetime(&self.options.timezone),
            account: account.clone(),
            amount: amount.clone(),
            tolerance,
            balance: outcome.balance,
            passed: outcome.passed,
            span: span.clone(),
        })
    }

    /// Consume final validation for the transaction the store is about to materialize.
    pub(crate) fn take_validated_transaction(&mut self, span: &SpanInfo, id: Uuid) -> bool {
        self.validation
            .as_mut()
            .expect("validation results exist while materializing the store")
            .take_transaction(span, id)
    }

    /// split a stage-processed stream back into (`metas`, `directives`) by datedness.
    /// option/plugin directives stay in the meta list exactly as stages left them; a
    /// stage-synthesized option/plugin does not take effect though — both were handled
    /// before the pipeline ran.
    fn partition_processed_directives(processed: Vec<Spanned<Directive>>) -> (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) {
        let (dated, metas): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) = processed.into_iter().partition(|it| it.datetime().is_some());
        (metas, dated)
    }

    /// the stages to run: the user's WASM plugins declared `stage: "raw"`, then [`BookingStage`],
    /// then the other plugins, each in declaration order (only with the plugin runtime and
    /// `features.plugins` on) and on the stream as plugins of ABI v1 see it ([`AbiV1View`]), then,
    /// when a plugin ran after booking, [`BookingStage`] once more, so that the pad and
    /// balance-check stages see what the plugins added or changed booked against the real lots,
    /// exactly as the final validation stage will (booking a booked transaction again changes nothing), then
    /// the built-in stages. A load without such plugins books exactly twice: booking and final validation
    fn build_stages(&self) -> Vec<Box<dyn ProcessStage>> {
        #[cfg(feature = "plugin_runtime")]
        let plugin_stages = |stage: PluginStage| -> Vec<Box<dyn ProcessStage>> {
            if self.options.features.plugins {
                self.plugins
                    .build_stages(stage)
                    .into_iter()
                    .map(|stage| Box::new(crate::pipeline::AbiV1View::new(stage)) as Box<dyn ProcessStage>)
                    .collect()
            } else {
                vec![]
            }
        };
        #[cfg(not(feature = "plugin_runtime"))]
        let plugin_stages = |_stage: PluginStage| -> Vec<Box<dyn ProcessStage>> { vec![] };
        // the native stages a test runs in a plugin's place, after the plugins of their slot
        #[cfg(test)]
        let (test_raw, test_booked): (SlottedStages, SlottedStages) = test::TEST_STAGES.take().into_iter().partition(|(slot, _)| *slot == PluginStage::Raw);
        #[cfg(not(test))]
        let (test_raw, test_booked): (SlottedStages, SlottedStages) = (vec![], vec![]);
        let booked_slot: Vec<Box<dyn ProcessStage>> = plugin_stages(PluginStage::Booked)
            .into_iter()
            .chain(test_booked.into_iter().map(|(_, stage)| stage))
            .collect();
        let rebook: Option<Box<dyn ProcessStage>> = (!booked_slot.is_empty()).then(|| Box::new(BookingStage) as Box<dyn ProcessStage>);
        plugin_stages(PluginStage::Raw)
            .into_iter()
            .chain(test_raw.into_iter().map(|(_, stage)| stage))
            .chain(std::iter::once(Box::new(BookingStage) as Box<dyn ProcessStage>))
            .chain(booked_slot)
            .chain(rebook)
            .chain(builtin_stages())
            .collect()
    }

    /// Run the pipeline, retaining final validation for materialization; the inputs stages recorded
    /// join [`Ledger::extra_inputs`]. Errors are inserted after binding transaction IDs in the fold.
    fn run_stages(&mut self, directives: Vec<Spanned<Directive>>) -> ZhangResult<(Vec<Spanned<Directive>>, AssertionOutcomes, FinalValidation)> {
        let directives = Ledger::sort_directives_datetime(directives);
        let stages = self.build_stages();
        let options = self.operations().options()?;
        let commodities = self.operations().read().commodities.values().cloned().collect_vec();
        let mut ctx = StageContext::new(&options)
            .with_commodities(commodities)
            .with_clock(self.clock.clone(), self.options.timezone);
        let directives = run_pipeline(&stages, directives, &mut ctx)?;

        self.extra_inputs.extend(ctx.inputs().iter().cloned());
        let (assertions, validation) = ctx.into_materialize_results();
        Ok((directives, assertions, validation))
    }
}

#[cfg(test)]
mod test {

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use indoc::indoc;
    use tempfile::tempdir;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Directive, SpanInfo, Spanned, Transaction};

    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;
    use crate::pipeline::{PluginStage, ProcessStage, StageContext};
    use crate::utils::id::FromSpan;
    use crate::ZhangResult;

    thread_local! {
        /// the stages the next load of this thread runs in a plugin's place, in the slot of their
        /// [`PluginStage`]: a seam for tests of what plugins can do to the stream, without WASM.
        /// [`Ledger::build_stages`] takes them
        pub(crate) static TEST_STAGES: RefCell<super::SlottedStages> = const { RefCell::new(vec![]) };
    }

    /// what a test stage does to the stream
    type Edit = Box<dyn Fn(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>>;

    /// a stage standing in for a plugin: `edit` gets the stream, and a log of what every test stage
    /// saw, in the order they ran
    struct TestStage {
        name: &'static str,
        log: Rc<RefCell<Vec<String>>>,
        edit: Edit,
    }

    impl ProcessStage for TestStage {
        fn name(&self) -> &str {
            self.name
        }

        fn process(&self, directives: Vec<Spanned<Directive>>, _ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            // whether the transactions the stage sees are booked: every posting has units
            let booked = directives
                .iter()
                .filter_map(|it| match &it.data {
                    Directive::Transaction(txn) => Some(txn.postings.iter().all(|posting| posting.units.is_some())),
                    _ => None,
                })
                .all(|it| it);
            self.log
                .borrow_mut()
                .push(format!("{}: {}", self.name, if booked { "booked" } else { "as written" }));
            Ok((self.edit)(directives))
        }
    }

    /// load `content` with `stages` running in a plugin's place, in their slots
    fn load_with_stages(content: &str, stages: Vec<(PluginStage, Box<dyn ProcessStage>)>) -> Ledger {
        TEST_STAGES.replace(stages);
        let ledger = load_from_temp_str(content);
        assert!(TEST_STAGES.take().is_empty(), "the load took the test stages");
        ledger
    }

    fn stage(
        name: &'static str, log: &Rc<RefCell<Vec<String>>>, edit: impl Fn(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> + 'static,
    ) -> Box<dyn ProcessStage> {
        Box::new(TestStage {
            name,
            log: log.clone(),
            edit: Box::new(edit),
        })
    }

    /// the transaction of the stream whose narration is `narration`
    fn transaction<'a>(directives: &'a [Spanned<Directive>], narration: &str) -> &'a Transaction {
        directives
            .iter()
            .find_map(|it| match &it.data {
                Directive::Transaction(txn) if txn.narration.as_ref().is_some_and(|it| it.as_str() == narration) => Some(txn),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a transaction {narration:?}"))
    }

    /// the store's rows of the transaction `narration`: `account unit-as-written = inferred`
    fn rows(ledger: &Ledger, narration: &str) -> Vec<String> {
        let store = ledger.store.read().unwrap();
        let mut txns = store
            .transactions
            .values()
            .filter(|it| it.narration.as_deref() == Some(narration))
            .collect::<Vec<_>>();
        txns.sort_by_key(|it| it.sequence);
        txns.iter()
            .flat_map(|txn| &txn.postings)
            .map(|row| {
                format!(
                    "{} {} = {}",
                    row.account.name(),
                    row.unit.as_ref().map_or("?".to_owned(), ToString::to_string),
                    row.inferred_amount
                )
            })
            .collect()
    }

    fn error_kinds(ledger: &Ledger) -> Vec<ErrorKind> {
        ledger.store.read().unwrap().errors.iter().map(|it| it.error_type.clone()).collect()
    }

    fn lots(ledger: &Ledger, account: &str) -> Vec<String> {
        let store = ledger.store.read().unwrap();
        store
            .commodity_lots
            .get(account)
            .map(|lots| {
                lots.iter()
                    .map(|lot| {
                        format!(
                            "{} {} {:?} {:?}",
                            lot.amount,
                            lot.commodity,
                            lot.cost.as_ref().map(ToString::to_string),
                            lot.acquisition_date
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    const LOTS: &str = indoc! {r#"
        1970-01-01 commodity USD
        1970-01-01 commodity CNY
        1970-01-01 open Assets:S
        1970-01-01 open Income:I
        2024-05-16 * "buy"
          Assets:S 10 USD { 10 CNY }
          Income:I -100 CNY
        2024-05-17 * "buy more"
          Assets:S 10 USD { 11 CNY }
          Income:I -110 CNY
        2024-05-18 * "sell"
          Assets:S -15 USD {}
          Income:I
    "#};

    #[test]
    fn raw_stages_see_the_stream_as_written_and_booked_stages_see_it_booked_in_slot_order() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            LOTS,
            vec![
                (PluginStage::Booked, stage("second booked", &log, |it| it)),
                (PluginStage::Raw, stage("raw", &log, |it| it)),
                (PluginStage::Booked, stage("first booked", &log, |it| it)),
            ],
        );
        // raw stages before booking, then the booked ones in their declaration order; stages that
        // change nothing change nothing
        assert_eq!(log.borrow().as_slice(), ["raw: as written", "second booked: booked", "first booked: booked"]);
        assert_eq!(error_kinds(&ledger), vec![]);
        assert_eq!(rows(&ledger, "sell"), vec!["Assets:S -15 USD = -15 USD", "Income:I ? = 155 CNY"]);
    }

    #[test]
    fn a_plugin_selling_a_lot_earlier_leaves_the_booked_legs_pinned_and_the_fold_reports_the_shortfall() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            LOTS,
            vec![(
                PluginStage::Booked,
                stage("insert an earlier sale", &log, |mut directives| {
                    // sells the whole first lot on 05-16, after the sale of 05-18 was booked against it
                    let sale = ZhangDataType {}
                        .transform(
                            indoc! {r#"
                                2024-05-16 * "sell first"
                                  Assets:S -10 USD { 10 CNY }
                                  Income:I 100 CNY
                            "#}
                            .to_owned(),
                            None,
                        )
                        .unwrap();
                    directives.extend(sale);
                    directives
                }),
            )],
        );
        // the 05-18 legs still name the lots pass 1 matched: -10 of the 05-16 lot (now empty) and
        // -5 of the 05-17 one. Pass 2 books the final stream: the 05-16 leg is short by 10, which
        // it reports once, and the lots are those of the final stream
        let sell = transaction(&ledger.directives, "sell");
        let legs: Vec<String> = sell
            .postings
            .iter()
            .map(|it| {
                format!(
                    "{} {:?}",
                    it.units.as_ref().unwrap(),
                    it.cost.as_ref().and_then(|cost| cost.date.as_ref()).map(|date| date.naive_date().to_string())
                )
            })
            .collect();
        assert_eq!(legs, vec!["-10 USD Some(\"2024-05-16\")", "-5 USD Some(\"2024-05-17\")", "155 CNY None"]);
        assert_eq!(error_kinds(&ledger), vec![ErrorKind::NoEnoughCommodityLot]);
        assert_eq!(
            lots(&ledger, "Assets:S"),
            vec!["5 USD Some(\"11 CNY\") Some(2024-05-17)", "-10 USD Some(\"10 CNY\") Some(2024-05-16)"]
        );
        assert_eq!(rows(&ledger, "sell"), vec!["Assets:S -15 USD = -15 USD", "Income:I ? = 155 CNY"]);
    }

    #[test]
    fn a_plugin_duplicating_a_transaction_gets_both_booked_and_the_second_reported() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            LOTS,
            vec![(
                PluginStage::Booked,
                stage("duplicate the sale", &log, |mut directives| {
                    let sale = directives
                        .iter()
                        .find(|it| matches!(&it.data, Directive::Transaction(txn) if txn.narration.as_ref().is_some_and(|it| it.as_str() == "sell")))
                        .unwrap()
                        .clone();
                    directives.push(sale);
                    directives
                }),
            )],
        );
        // both copies share a span and get distinct ids; the second finds no lots left
        assert_eq!(
            rows(&ledger, "sell"),
            vec![
                "Assets:S -15 USD = -15 USD",
                "Income:I ? = 155 CNY",
                "Assets:S -15 USD = -15 USD",
                "Income:I ? = 155 CNY"
            ]
        );
        assert_eq!(error_kinds(&ledger), vec![ErrorKind::NoEnoughCommodityLot]);
    }

    #[test]
    fn final_validation_binds_duplicate_span_errors_to_the_ids_materialization_allocates() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            indoc! {r#"
                option "operating_currency" "USD"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:E
                2024-01-01 balance Assets:A 0 USD
                2024-01-02 * "template"
                  Assets:A 2 USD
                  Equity:E -1 USD
            "#},
            vec![(
                PluginStage::Booked,
                stage("share an assertion span", &log, |mut directives| {
                    let span = directives.iter().find(|it| matches!(it.data, Directive::BalanceCheck(_))).unwrap().span.clone();
                    let mut txn = directives.pop().unwrap();
                    txn.span = span;
                    let mut rejected = txn.clone();
                    let Directive::Transaction(rejected_txn) = &mut rejected.data else {
                        unreachable!()
                    };
                    for posting in &mut rejected_txn.postings {
                        posting.units = None;
                        posting.written = None;
                    }
                    directives.extend([txn.clone(), rejected.clone(), txn, rejected]);
                    directives
                }),
            )],
        );
        let store = ledger.store.read().unwrap();
        let base = store.balance_assertions[0].id;
        let first = uuid::Uuid::derived(&base, 1);
        let second = uuid::Uuid::derived(&base, 2);
        let after = uuid::Uuid::derived(&base, 3);
        assert_eq!(store.transactions.len(), 2);
        assert!(store.transactions.contains_key(&first));
        assert!(store.transactions.contains_key(&second));
        assert_eq!(store.postings.len(), 4);
        assert_eq!(
            store
                .errors
                .iter()
                .map(|error| (error.error_type.clone(), error.metas[crate::constants::TXN_ID].clone()))
                .collect::<Vec<_>>(),
            [
                (ErrorKind::UnbalancedTransaction, first.to_string()),
                (ErrorKind::TransactionHasMultipleImplicitPosting, second.to_string()),
                (ErrorKind::UnbalancedTransaction, second.to_string()),
                (ErrorKind::TransactionHasMultipleImplicitPosting, after.to_string()),
            ]
        );
        assert_eq!(ledger.directives.iter().filter(|it| matches!(it.data, Directive::Transaction(_))).count(), 4);
        assert!(ledger.validation.is_none(), "temporary validation results are dropped after loading");
    }

    /// the names of the stages the next load runs, with `stages` in a plugin's place
    fn stage_names(stages: Vec<(PluginStage, Box<dyn ProcessStage>)>) -> Vec<String> {
        TEST_STAGES.replace(stages);
        let ledger = load_from_temp_str(LOTS);
        TEST_STAGES.replace(TEST_STAGES.take());
        // the load took the queued stages; queue them again for this call, which takes them too
        let names = ledger.build_stages().iter().map(|it| it.name().to_owned()).collect();
        assert!(TEST_STAGES.take().is_empty());
        names
    }

    #[test]
    fn booking_runs_once_without_plugins_and_again_after_the_plugins_that_run_booked() {
        assert_eq!(stage_names(vec![]), ["booking", "active-accounts", "balance-pad", "balance-check", "validate"]);

        let log = Rc::new(RefCell::new(vec![]));
        TEST_STAGES.replace(vec![(PluginStage::Raw, stage("raw", &log, |it| it))]);
        let ledger = load_from_temp_str(LOTS);
        TEST_STAGES.replace(vec![(PluginStage::Raw, stage("raw", &log, |it| it))]);
        let names: Vec<String> = ledger.build_stages().iter().map(|it| it.name().to_owned()).collect();
        assert_eq!(
            names,
            ["raw", "booking", "active-accounts", "balance-pad", "balance-check", "validate"],
            "a raw plugin needs no second pass"
        );

        TEST_STAGES.replace(vec![(PluginStage::Booked, stage("booked", &log, |it| it))]);
        let names: Vec<String> = ledger.build_stages().iter().map(|it| it.name().to_owned()).collect();
        assert_eq!(
            names,
            ["booking", "booked", "booking", "active-accounts", "balance-pad", "balance-check", "validate"]
        );
    }

    /// what the pad and balance-check stages see is what the store holds: a plugin's `{}` sale with
    /// an implicit posting is booked against the real lots before they run
    #[test]
    fn a_plugins_sale_with_an_implicit_posting_is_booked_before_the_balance_check() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            &format!(
                "{LOTS}{}",
                indoc! {r#"
                    2024-05-20 balance Assets:S 2 USD
                    2024-05-20 balance Income:I -22 CNY
                "#}
            ),
            vec![(
                PluginStage::Booked,
                stage("insert a sale", &log, |mut directives| {
                    directives.extend(
                        ZhangDataType {}
                            .transform(
                                indoc! {r#"
                                    2024-05-19 * "plugin sale"
                                      Assets:S -3 USD {}
                                      Income:I
                                "#}
                                .to_owned(),
                                None,
                            )
                            .unwrap(),
                    );
                    directives
                }),
            )],
        );
        // 20 bought, 15 sold by hand, 3 by the plugin: 2 left; the plugin's income is 3 × 11 CNY
        assert_eq!(error_kinds(&ledger), vec![]);
        assert_eq!(rows(&ledger, "plugin sale"), vec!["Assets:S -3 USD = -3 USD", "Income:I ? = 33 CNY"]);
        let store = ledger.store.read().unwrap();
        assert!(
            store.balance_assertions.iter().all(|it| it.passed),
            "{:?}",
            store.balance_assertions.iter().map(|it| (&it.account, &it.balance)).collect::<Vec<_>>()
        );
    }

    /// a lot a plugin adds before a hand-written `{}` sale: the sale books against it (its `{}`
    /// leg is not booked until then), and the pad stage sees the account hold nothing at cost
    /// afterwards, like the store. Cash is implicit so the initial empty cost cannot infer a
    /// new short lot before the plugin supplies the purchase.
    #[test]
    fn a_plugins_purchase_before_a_written_sale_is_booked_before_the_pad_stage() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            indoc! {r#"
                1970-01-01 commodity USD
                1970-01-01 commodity CNY
                1970-01-01 open Assets:S
                1970-01-01 open Assets:Cash
                1970-01-01 open Equity:Open
                2024-05-18 * "sell"
                  Assets:S -2 USD { }
                  Assets:Cash
                2024-05-20 pad Assets:S Equity:Open
                2024-05-21 balance Assets:S 5 USD
            "#},
            vec![(
                PluginStage::Booked,
                stage("insert a purchase", &log, |mut directives| {
                    directives.extend(
                        ZhangDataType {}
                            .transform(
                                indoc! {r#"
                                    2024-05-16 * "plugin buy"
                                      Assets:S 2 USD { 7 CNY }
                                      Assets:Cash -14 CNY
                                "#}
                                .to_owned(),
                                None,
                            )
                            .unwrap(),
                    );
                    directives
                }),
            )],
        );
        // the sale reduces the plugin's lot in full: nothing is held at cost, so the pad is not an
        // error, and it pads the 5 USD the assertion needs
        assert_eq!(error_kinds(&ledger), vec![]);
        assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD None None"]);
        let store = ledger.store.read().unwrap();
        assert!(store.balance_assertions.iter().all(|it| it.passed));
        let sale = transaction(&ledger.directives, "sell");
        assert_eq!(
            sale.postings[0].cost.as_ref().and_then(|it| it.base.as_ref()).map(ToString::to_string),
            Some("7 CNY".to_owned())
        );
    }

    #[test]
    fn a_plugin_moving_the_legs_of_a_split_apart_gets_them_as_booked() {
        let log = Rc::new(RefCell::new(vec![]));
        let ledger = load_with_stages(
            LOTS,
            vec![(
                PluginStage::Booked,
                stage("reorder the legs", &log, |mut directives| {
                    for directive in &mut directives {
                        if let Directive::Transaction(txn) = &mut directive.data {
                            if txn.narration.as_ref().is_some_and(|it| it.as_str() == "sell") {
                                // [S -10, S -5, I] -> [S -10, I, S -5]
                                let income = txn.postings.remove(2);
                                txn.postings.insert(1, income);
                            }
                        }
                    }
                    directives
                }),
            )],
        );
        assert_eq!(error_kinds(&ledger), vec![]);
        // one row per leg, as booked: the written form is not restored for a split broken apart
        assert_eq!(
            rows(&ledger, "sell"),
            vec!["Assets:S -10 USD = -10 USD", "Income:I ? = 155 CNY", "Assets:S -5 USD = -5 USD"]
        );
        assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD Some(\"11 CNY\") Some(2024-05-17)"]);
    }

    fn fake_span_info() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 0,
            content: "".to_string(),
            filename: None,
        }
    }
    fn test_parse_zhang(content: &str) -> Vec<Spanned<Directive>> {
        let data_type = ZhangDataType {};
        data_type.transform(content.to_owned(), None).unwrap()
    }

    fn load_from_temp_str(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().keep();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap()
    }

    #[test]
    fn assertions_sharing_a_span_get_distinct_ids() {
        // a plugin may emit several balances for one of the ledger: they share its span, which ids are derived from
        let mut directives = test_parse_zhang("1970-01-01 open Assets:A\n2024-01-02 balance Assets:A 0 CNY\n");
        let mut copy = directives.iter().find(|it| matches!(it.data, Directive::BalanceCheck(_))).unwrap().clone();
        if let Directive::BalanceCheck(check) = &mut copy.data {
            check.date = zhang_ast::Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 3).unwrap());
        }
        directives.push(copy);
        let ledger = Ledger::process(super::LedgerProcessContext {
            directives,
            entry: (tempdir().unwrap().keep(), "main.zhang".to_owned()),
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            clock: crate::clock::Clock::System,
        })
        .unwrap();
        let store = ledger.store.read().unwrap();
        let ids = store.balance_assertions.iter().map(|it| it.id).collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        // and none is the id of a transaction or a posting
        assert!(ids
            .iter()
            .all(|id| !store.transactions.contains_key(id) && store.postings.iter().all(|posting| posting.id != *id)));
    }

    mod write_back {
        use indoc::indoc;
        use zhang_ast::{Comment, Directive, Spanned};

        use super::{fake_span_info, test_parse_zhang};
        use crate::ledger::Ledger;

        #[test]
        fn should_split_processed_stream_by_datedness() {
            // the stream stages see contains option/plugin directives too
            let mut processed = test_parse_zhang(indoc! {r#"
                option "title" "t"
                plugin "foo"
                1970-01-01 open Assets:Cash
                1970-01-02 commodity CNY
            "#});
            processed.push(Spanned::new(
                Directive::Comment(Comment {
                    content: "; synthesized".to_owned(),
                }),
                fake_span_info(),
            ));

            let (new_metas, dated) = Ledger::partition_processed_directives(processed);

            assert_eq!(new_metas.len(), 3);
            assert!(matches!(new_metas[0].data, Directive::Option(_)));
            assert!(matches!(new_metas[1].data, Directive::Plugin(_)));
            assert!(matches!(new_metas[2].data, Directive::Comment(_)));
            assert_eq!(dated.len(), 2);
            assert!(matches!(dated[0].data, Directive::Open(_)));
            assert!(matches!(dated[1].data, Directive::Commodity(_)));
        }

        #[test]
        fn should_reflect_directives_removed_by_stages() {
            // the stages filtered everything out
            let processed = vec![];

            let (new_metas, dated) = Ledger::partition_processed_directives(processed);

            assert!(new_metas.is_empty());
            assert!(dated.is_empty());
        }
    }

    mod sort_directive_datetime {
        use indoc::indoc;
        use itertools::Itertools;
        use zhang_ast::{Directive, Options, Spanned, ZhangString};

        use crate::ledger::test::{fake_span_info, test_parse_zhang};
        use crate::ledger::Ledger;

        #[test]
        fn should_keep_order_given_two_none_datetime() {
            let original = vec![
                Spanned::new(
                    Directive::Option(Options {
                        key: ZhangString::quote("title"),
                        value: ZhangString::quote("Title"),
                    }),
                    fake_span_info(),
                ),
                Spanned::new(
                    Directive::Option(Options {
                        key: ZhangString::quote("description"),
                        value: ZhangString::quote("Description"),
                    }),
                    fake_span_info(),
                ),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Spanned::new(
                        Directive::Option(Options {
                            key: ZhangString::quote("title"),
                            value: ZhangString::quote("Title"),
                        }),
                        fake_span_info(),
                    ),
                    Spanned::new(
                        Directive::Option(Options {
                            key: ZhangString::quote("description"),
                            value: ZhangString::quote("Description"),
                        }),
                        fake_span_info(),
                    ),
                ],
                sorted
            )
        }

        #[test]
        fn should_put_undated_directives_first() {
            let original = test_parse_zhang(indoc! {r#"
                1970-01-01 open Assets:Hello
                option "description" "Description"
            "#});
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec!["option description", "open Assets:Hello"],
                sorted.iter().map(|it| label(&it.data)).collect_vec()
            );
            let original = test_parse_zhang(indoc! {r#"
                    option "description" "Description"
                    1970-01-01 open Assets:Hello
                "#});
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    option "description" "Description"
                    1970-01-01 open Assets:Hello
                "#}),
                sorted
            )
        }

        #[test]
        fn should_order_by_datetime() {
            let original = test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    1970-02-01 open Assets:Hello
                "#});

            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    1970-02-01 open Assets:Hello
                "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                sorted.into_iter().map(|it| it.data).collect_vec()
            );
            let original = test_parse_zhang(indoc! {r#"
                    1970-02-01 open Assets:Hello
                    1970-01-01 open Assets:Hello
                "#});
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    1970-02-01 open Assets:Hello
                "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                sorted.into_iter().map(|it| it.data).collect_vec()
            )
        }
        #[test]
        fn should_sort_dated_directives_across_undated_ones() {
            let original = test_parse_zhang(indoc! {r#"
                    option "1" "1"
                    1970-03-01 open Assets:Hello
                    1970-02-01 open Assets:Hello
                    option "2" "2"
                    1970-01-01 open Assets:Hello
                "#});

            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    option "1" "1"
                    option "2" "2"
                    1970-01-01 open Assets:Hello
                    1970-02-01 open Assets:Hello
                    1970-03-01 open Assets:Hello
                "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                sorted.into_iter().map(|it| it.data).collect_vec()
            );
        }

        #[test]
        fn should_keep_order_given_same_datetime() {
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    1970-01-01 close Assets:Hello
                "#}),
                Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    1970-01-01 close Assets:Hello
                "#}))
            );
        }

        #[test]
        fn should_move_balance_to_the_top() {
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 balance Assets:Hello 2 CNY
                    1970-01-01 document Assets:Hello ""
                "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
                    1970-01-01 document Assets:Hello ""
                    1970-01-01 balance Assets:Hello 2 CNY
                "#}))
                .into_iter()
                .map(|it| it.data)
                .collect_vec()
            );
        }
        #[test]
        fn should_keep_balance_order() {
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 balance Assets:Hello 2 CNY
                    1970-01-01 balance Assets:Hello2 2 CNY
                "#}),
                Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
                    1970-01-01 balance Assets:Hello 2 CNY
                    1970-01-01 balance Assets:Hello2 2 CNY
                "#}))
            );
        }

        #[test]
        fn should_keep_balance_transaction_behind_its_directive() {
            // the pad stage inserts a balance pad's transaction (flag `P`) right after it;
            // the re-sort keeps it there instead of moving it behind every balance entry
            let stream = test_parse_zhang(indoc! {r#"
                1970-01-01 open Assets:Hello
                1970-01-02 balance Assets:Hello 2 CNY with pad Equity:Open
                1970-01-02 P "Balance Pad" "pad"
                  Assets:Hello 2 CNY
                  Equity:Open
                1970-01-02 balance Assets:Hello 2 CNY
                1970-01-02 * "regular"
                  Assets:Hello 1 CNY
                  Equity:Open
            "#});
            let expected = stream.iter().map(|it| it.data.clone()).collect_vec();
            assert_eq!(expected, Ledger::sort_directives_datetime(stream).into_iter().map(|it| it.data).collect_vec());
        }

        #[test]
        fn should_sort_a_c_flagged_transaction_like_any_other() {
            // a balance check materializes into nothing; a transaction flagged `C` (beancount's
            // conversions) is an ordinary transaction, after the balance entries of its day
            let sorted = Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
                1970-01-02 C "Conversion" ""
                  Assets:Hello 0 CNY
                1970-01-02 balance Assets:Hello 2 CNY
            "#}));
            assert_eq!(
                sorted.iter().map(|it| label(&it.data)).collect_vec(),
                vec!["check Assets:Hello 2", "txn Conversion"]
            );
        }

        /// more than 20 directives, so the real sort algorithm runs (not the
        /// insertion sort used for short slices)
        const MIXED: &str = indoc! {r#"
            2023-01-03 * "t1" ""
              Assets:A 1 CNY
              Equity:Open
            option "o1" "1"
            2023-01-02 balance Assets:A 1 CNY
            2023-01-02 open Assets:A
            2023-01-01 close Assets:B
            2023-01-01 open Assets:B
            2023-01-02 * "t2" ""
              Assets:A 1 CNY
              Equity:Open
            2023-01-02 balance Assets:A 2 CNY with pad Equity:Open
            2023-01-02 P "pad" ""
              Assets:A 1 CNY
              Equity:Open
            plugin "p1"
            2023-01-03 note Assets:A "n"
            2023-01-03 balance Assets:A 3 CNY
            2023-01-03 C "conversion" ""
              Assets:A 0 CNY
            2023-01-01 commodity CNY
            2023-01-02 10:00 * "t3" ""
              Assets:A 1 CNY
              Equity:Open
            2023-01-02 10:00 open Assets:C
            2023-01-02 10:00 balance Assets:C 0 CNY
            option "o2" "2"
            2023-01-03 open Assets:D
            2023-01-01 price USD 7 CNY
            2023-01-04 event "e" "v"
            2023-01-03 document Assets:A "x"
            2023-01-02 close Assets:A
            2023-01-01 * "t0" ""
              Assets:B 1 CNY
              Equity:Open
        "#};

        fn label(directive: &Directive) -> String {
            match directive {
                Directive::Open(it) => format!("open {}", it.account.name()),
                Directive::Close(it) => format!("close {}", it.account.name()),
                Directive::Transaction(it) => format!("txn {}", it.payee.as_ref().map(|p| p.as_str()).unwrap_or_default()),
                Directive::BalanceCheck(it) => format!("check {} {}", it.account.name(), it.amount.number),
                Directive::BalancePad(it) => format!("pad {} {}", it.account.name(), it.amount.number),
                Directive::Option(it) => format!("option {}", it.key.as_str()),
                Directive::Plugin(it) => format!("plugin {}", it.module.as_str()),
                other => other.directive_type().to_string(),
            }
        }

        #[test]
        fn should_sort_by_datetime_then_rank_keeping_source_order() {
            let original = test_parse_zhang(MIXED);
            assert!(original.len() > 20);

            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                sorted.iter().map(|it| label(&it.data)).collect_vec(),
                vec![
                    // undated first, in source order
                    "option o1",
                    "plugin p1",
                    "option o2",
                    // per datetime: open/commodity, then balance entries, then the rest — each in source order
                    "open Assets:B",
                    "Commodity",
                    "close Assets:B",
                    "Price",
                    "txn t0",
                    "open Assets:A",
                    "check Assets:A 1",
                    "pad Assets:A 2",
                    "txn pad",
                    "txn t2",
                    "close Assets:A",
                    "open Assets:C",
                    "check Assets:C 0",
                    "txn t3",
                    "open Assets:D",
                    "check Assets:A 3",
                    // a `C` transaction is no balance entry
                    "txn t1",
                    "Note",
                    "txn conversion",
                    "Document",
                    "Event",
                ]
            );
        }

        #[test]
        fn should_sort_a_pad_after_the_balance_entries_of_its_day() {
            let mut stream = test_parse_zhang(indoc! {r#"
                2024-03-01 * "before" ""
                  Assets:A 1 CNY
                  Equity:Open
                2024-03-01 pad Assets:A Equity:Open
                2024-03-01 * "after" ""
                  Assets:A 1 CNY
                  Equity:Open
                2024-03-01 09:00:00 * "morning" ""
                  Assets:A 1 CNY
                  Equity:Open
                2024-03-01 10:00:00 balance Assets:A 1 CNY
                2024-03-01 11:00:00 * "late" ""
                  Assets:A 1 CNY
                  Equity:Open
                2024-03-02 pad Assets:B Equity:Open
                2024-03-02 balance Assets:B 1 CNY
                2024-03-03 08:00:00 pad Assets:C Equity:Open
                2024-03-03 balance Assets:C 1 CNY
            "#});
            // the padding transaction of a `pad` carries its span
            let pad = stream.iter().find(|it| matches!(it.data, Directive::Pad(_))).unwrap().span.clone();
            stream.push(Spanned::new(
                test_parse_zhang("2024-03-01 10:00:00 P \"padding\" \"\"\n  Assets:A 1 CNY\n  Equity:Open\n")
                    .remove(0)
                    .data,
                pad,
            ));
            let sorted = Ledger::sort_directives_datetime(stream);
            assert_eq!(
                sorted.iter().map(|it| label(&it.data)).collect_vec(),
                vec![
                    // a `pad` sorts at the time of the last balance entry of its day, after it, and its padding with it
                    "txn before",
                    "txn after",
                    "txn morning",
                    "check Assets:A 1",
                    "Pad",
                    "txn padding",
                    "txn late",
                    // without times, a day's balance entries come before its pads
                    "check Assets:B 1",
                    "Pad",
                    // a `pad` later than every balance entry of its day keeps its time
                    "check Assets:C 1",
                    "Pad",
                ]
            );
            assert_eq!(Ledger::sort_directives_datetime(sorted.clone()), sorted);
        }

        #[test]
        fn should_be_idempotent() {
            let original = test_parse_zhang(MIXED);
            // a fixed shuffle: 7 is coprime to the length, so this is a permutation
            let len = original.len();
            assert_ne!(len % 7, 0);
            let shuffled = (0..len).map(|i| original[(i * 7) % len].clone()).collect_vec();

            let once = Ledger::sort_directives_datetime(shuffled);
            let twice = Ledger::sort_directives_datetime(once.clone());
            assert_eq!(once, twice);
        }
    }
    mod pipeline {
        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use itertools::Itertools;
        use zhang_ast::error::ErrorKind;
        use zhang_ast::{Directive, Flag};

        use crate::ledger::test::load_from_temp_str;
        use crate::ledger::Ledger;

        #[test]
        fn should_run_builtin_stages_without_plugins() {
            // neither the plugin runtime nor `features.plugins` is needed for pad/check
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 balance Assets:A 100 CNY with pad Equity:Open
                2023-01-01 balance Assets:A 100 CNY
                2023-01-02 balance Assets:A 90 CNY
            "#});

            // the pad is the only transaction: checks book nothing
            assert_eq!(
                journal(&ledger),
                vec![(Flag::BalancePad, vec![posting("Assets:A", 100, 100), posting("Equity:Open", -100, -100)])]
            );
            // the `balance ... with pad` is checked, and kept, after the balance entries of its time
            assert_eq!(
                assertions(&ledger),
                vec![
                    assertion(2, "Assets:A", 100, 100, true),
                    assertion(3, "Assets:A", 100, 100, true),
                    assertion(4, "Assets:A", 90, 100, false)
                ]
            );
            assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, Some("Assets:A".to_owned()))]);
        }

        #[test]
        fn should_write_synthesized_transactions_back_into_directives() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 balance Assets:A 100 CNY with pad Equity:Open
            "#});

            let kinds = ledger
                .directives
                .iter()
                .map(|it| match &it.data {
                    Directive::Transaction(txn) => format!("txn {}", txn.flag.clone().unwrap()),
                    other => other.directive_type().to_string(),
                })
                .collect_vec();
            assert_eq!(kinds, vec!["Open", "Open", "BalancePad", "txn P"]);
            // the transaction carries its directive's span, so its id is stable across reloads
            assert_eq!(ledger.directives[2].span, ledger.directives[3].span);
        }

        /// (account, inferred units, balance after)
        type BookedPosting = (String, BigDecimal, BigDecimal);

        fn journal(ledger: &Ledger) -> Vec<(Flag, Vec<BookedPosting>)> {
            let store = ledger.store.read().unwrap();
            let mut balances: std::collections::HashMap<(String, String), BigDecimal> = std::collections::HashMap::new();
            store
                .transactions
                .values()
                .sorted_by_key(|it| it.sequence)
                .map(|it| {
                    let postings = it
                        .postings
                        .iter()
                        .map(|p| {
                            let amount = balances.entry((p.account.name().to_owned(), p.inferred_amount.commodity.clone())).or_default();
                            *amount += &p.inferred_amount.number;
                            (p.account.name().to_owned(), p.inferred_amount.number.clone(), amount.clone())
                        })
                        .collect_vec();
                    (it.flag.clone(), postings)
                })
                .collect_vec()
        }

        fn errors(ledger: &Ledger) -> Vec<(ErrorKind, Option<String>)> {
            let store = ledger.store.read().unwrap();
            store
                .errors
                .iter()
                .map(|it| (it.error_type.clone(), it.metas.get("account_name").cloned()))
                .collect_vec()
        }

        fn posting(account: &str, inferred: i32, after: i32) -> BookedPosting {
            (account.to_owned(), BigDecimal::from(inferred), BigDecimal::from(after))
        }

        /// (sequence, account, asserted number, balance number, passed)
        type CheckedAssertion = (i32, String, BigDecimal, BigDecimal, bool);

        fn assertions(ledger: &Ledger) -> Vec<CheckedAssertion> {
            let store = ledger.store.read().unwrap();
            store
                .balance_assertions
                .iter()
                .map(|it| {
                    assert_eq!(it.amount.commodity, it.balance.commodity);
                    (
                        it.sequence,
                        it.account.name().to_owned(),
                        it.amount.number.clone(),
                        it.balance.number.clone(),
                        it.passed,
                    )
                })
                .collect_vec()
        }

        fn assertion(sequence: i32, account: &str, asserted: i32, balance: i32, passed: bool) -> CheckedAssertion {
            (sequence, account.to_owned(), BigDecimal::from(asserted), BigDecimal::from(balance), passed)
        }

        #[test]
        fn should_not_book_a_pad_already_at_its_target() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 * "x"
                  Assets:A 10 CNY
                  Equity:Open
                2023-01-02 balance Assets:A 10 CNY with pad Equity:Open
                2023-01-03 balance Assets:A 10 CNY with pad Equity:Missing
            "#});

            // loads (this used to abort with `TransactionCannotInferTradeAmount`), books no
            // padding, and still reports the pad's missing account once
            assert_eq!(
                journal(&ledger),
                vec![(Flag::Okay, vec![posting("Assets:A", 10, 10), posting("Equity:Open", -10, -10)])]
            );
            assert_eq!(errors(&ledger), vec![(ErrorKind::AccountDoesNotExist, Some("Equity:Missing".to_owned()))]);
        }

        #[test]
        fn should_report_each_account_error_once() {
            // stages report pad/check account errors; the fold must not report them again
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Assets:Closed
                1970-01-01 open Equity:Open
                1970-01-02 close Assets:Closed
                2023-01-01 balance Assets:A 10 CNY with pad Equity:Missing
                2023-01-02 balance Assets:Closed 5 CNY with pad Equity:Open
                2023-01-03 balance Assets:Missing 0 CNY
                2023-01-04 balance Assets:Closed 5 CNY
            "#});

            let errors = errors(&ledger)
                .into_iter()
                .sorted_by_key(|(kind, account)| (kind.to_string(), account.clone()))
                .collect_vec();
            assert_eq!(
                errors,
                vec![
                    (ErrorKind::AccountClosed, Some("Assets:Closed".to_owned())),
                    (ErrorKind::AccountClosed, Some("Assets:Closed".to_owned())),
                    (ErrorKind::AccountDoesNotExist, Some("Assets:Missing".to_owned())),
                    (ErrorKind::AccountDoesNotExist, Some("Equity:Missing".to_owned())),
                ]
            );
            // one `AccountClosed` each for the pad and the check, on their own spans
            let store = ledger.store.read().unwrap();
            let closed_spans = store
                .errors
                .iter()
                .filter(|it| it.error_type == ErrorKind::AccountClosed)
                .map(|it| it.span.as_ref().unwrap().content.clone())
                .collect_vec();
            assert_eq!(
                closed_spans,
                vec![
                    "2023-01-02 balance Assets:Closed 5 CNY with pad Equity:Open",
                    "2023-01-04 balance Assets:Closed 5 CNY"
                ]
            );
        }

        #[test]
        fn should_report_undeclared_commodity_of_a_pad() {
            // the padding transaction is validated like any other transaction
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 balance Assets:A 10 XYZ with pad Equity:Open
            "#});

            assert_eq!(errors(&ledger), vec![(ErrorKind::CommodityDoesNotDefine, None)]);
            assert_eq!(
                journal(&ledger),
                vec![(Flag::BalancePad, vec![posting("Assets:A", 10, 10), posting("Equity:Open", -10, -10)])]
            );
        }

        #[test]
        fn should_size_a_pad_from_the_true_balance_after_a_failing_check() {
            // the failing check changes nothing: the account still holds 10 when padded to 100,
            // so the pad books 90 and the books net to zero
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 * "x"
                  Assets:A 10 CNY
                  Equity:Open
                2023-02-01 balance Assets:A 50 CNY
                2023-02-01 balance Assets:A 100 CNY with pad Equity:Open
                2023-02-03 balance Assets:A 100 CNY
            "#});

            assert_eq!(
                journal(&ledger),
                vec![
                    (Flag::Okay, vec![posting("Assets:A", 10, 10), posting("Equity:Open", -10, -10)]),
                    (Flag::BalancePad, vec![posting("Assets:A", 90, 100), posting("Equity:Open", -90, -100)]),
                ]
            );
            // the checks keep their place in the journal, between the transactions; the `balance ... with pad` is one
            assert_eq!(
                assertions(&ledger),
                vec![
                    assertion(2, "Assets:A", 50, 10, false),
                    assertion(4, "Assets:A", 100, 100, true),
                    assertion(5, "Assets:A", 100, 100, true)
                ]
            );
            assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, Some("Assets:A".to_owned()))]);
        }

        #[test]
        fn should_pass_a_check_within_its_tolerance_without_moving_the_balance() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                2023-01-01 * "x"
                  Assets:A 50.004 CNY
                  Equity:Open
                2023-01-02 balance Assets:A 50 ~ 0.01 CNY
                2023-01-03 balance Assets:A 50.004 CNY
            "#});

            let store = ledger.store.read().unwrap();
            let checks = store
                .balance_assertions
                .iter()
                .map(|it| {
                    (
                        it.amount.number.to_string(),
                        it.tolerance.as_ref().map(|it| it.to_string()),
                        it.balance.number.to_string(),
                        it.passed,
                    )
                })
                .collect_vec();
            assert_eq!(
                checks,
                vec![
                    ("50".to_owned(), Some("0.01".to_owned()), "50.004".to_owned(), true),
                    ("50.004".to_owned(), None, "50.004".to_owned(), true),
                ]
            );
            assert!(store.errors.is_empty());
            assert_eq!(store.transactions.len(), 1);
        }

        #[test]
        fn should_check_a_balance_with_pad_after_the_pads_of_its_time() {
            // the parent is padded to 500 first; the pad of its sub-account at the same time then moves it to 545,
            // which the parent's assertion must report instead of holding silently
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Bank
                1970-01-01 open Assets:Bank:Checking
                1970-01-01 open Equity:Open
                2024-01-02 * "init"
                  Assets:Bank 345 CNY
                  Assets:Bank:Checking 155 CNY
                  Equity:Open
                2024-01-10 balance Assets:Bank 500 CNY with pad Equity:Open
                2024-01-10 balance Assets:Bank:Checking 200 CNY with pad Equity:Open
            "#});

            assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, Some("Assets:Bank".to_owned()))]);
            let store = ledger.store.read().unwrap();
            let checks = store
                .balance_assertions
                .iter()
                .map(|it| (it.account.name().to_owned(), it.balance.number.clone(), it.passed))
                .collect_vec();
            assert_eq!(
                checks,
                vec![
                    ("Assets:Bank".to_owned(), BigDecimal::from(545), false),
                    ("Assets:Bank:Checking".to_owned(), BigDecimal::from(200), true),
                ]
            );
            // a check kept for a `balance ... with pad` has an id of its own, apart from its padding's and its postings'
            let mut ids = store
                .transactions
                .keys()
                .chain(store.postings.iter().map(|it| &it.id))
                .chain(store.balance_assertions.iter().map(|it| &it.id))
                .collect_vec();
            let all = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), all);
        }

        #[test]
        fn should_validate_a_c_flagged_transaction_like_any_other() {
            // `C` is beancount's flag for conversions, not a balance check: an unbalanced one is an error
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                2023-01-01 C "one-sided"
                  Assets:A 10 CNY
            "#});

            assert_eq!(journal(&ledger), vec![(Flag::BalanceCheck, vec![posting("Assets:A", 10, 10)])]);
            assert_eq!(errors(&ledger), vec![(ErrorKind::UnbalancedTransaction, None)]);
            assert!(assertions(&ledger).is_empty());
        }

        /// a ledger whose comparator-based sort panicked ("user-provided comparison
        /// function does not correctly implement a total order"): a balance written
        /// before its same-day `open`, among more than 20 directives
        const BALANCE_BEFORE_OPEN: &str = indoc! {r#"
                option "operating_currency" "CNY"
                2023-01-01 * "t0"
                  Assets:D0 1 CNY
                  Equity:Open
                2023-01-01 balance Assets:D0 1 CNY
                2023-01-09 * "t21"
                  Assets:D8 1 CNY
                  Equity:Open
                2023-01-01 * "t2"
                  Assets:D0 1 CNY
                  Equity:Open
                2023-01-02 balance Assets:D1 3 CNY
                2023-01-02 balance Assets:D1 4 CNY
                2023-01-02 * "t5"
                  Assets:D1 1 CNY
                  Equity:Open
                2023-01-02 * "t7"
                  Assets:D1 1 CNY
                  Equity:Open
                2023-01-04 * "t8"
                  Assets:D3 1 CNY
                  Equity:Open
                2023-01-04 * "t9"
                  Assets:D3 1 CNY
                  Equity:Open
                2023-01-02 * "t6"
                  Assets:D1 1 CNY
                  Equity:Open
                2023-01-04 balance Assets:D3 10 CNY
                2023-01-04 open Assets:D3
                2023-01-04 balance Assets:D3 12 CNY with pad Equity:Open
                2023-01-05 * "t13"
                  Assets:D4 1 CNY
                  Equity:Open
                2023-01-06 * "t14"
                  Assets:D5 1 CNY
                  Equity:Open
                2023-01-07 * "t15"
                  Assets:D6 1 CNY
                  Equity:Open
                2023-01-07 * "t16"
                  Assets:D6 1 CNY
                  Equity:Open
                2023-01-07 open Assets:D6
                2023-01-08 * "t18"
                  Assets:D7 1 CNY
                  Equity:Open
                2023-01-08 * "t19"
                  Assets:D7 1 CNY
                  Equity:Open
                2023-01-09 * "t20"
                  Assets:D8 1 CNY
                  Equity:Open
                2023-01-09 * "t22"
                  Assets:D8 1 CNY
                  Equity:Open
        "#};

        #[test]
        fn should_sort_open_before_same_day_balance() {
            let ledger = load_from_temp_str(BALANCE_BEFORE_OPEN);

            let same_day = ledger
                .directives
                .iter()
                .filter(|it| it.datetime().map(|d| d.date().to_string()) == Some("2023-01-04".to_owned()))
                .map(|it| match &it.data {
                    Directive::Transaction(txn) => format!("txn {}", txn.flag.clone().unwrap()),
                    other => other.directive_type().to_string(),
                })
                .collect_vec();
            assert_eq!(same_day, vec!["Open", "BalanceCheck", "BalancePad", "txn P", "txn *", "txn *"]);
            let missing = errors(&ledger)
                .into_iter()
                .filter(|(kind, _)| *kind == ErrorKind::AccountDoesNotExist)
                .filter_map(|(_, account)| account)
                .collect_vec();
            assert!(!missing.contains(&"Assets:D3".to_owned()));
        }

        #[test]
        fn should_keep_synthesized_transactions_behind_their_directives() {
            let ledger = load_from_temp_str(BALANCE_BEFORE_OPEN);

            let mut synthesized = 0;
            for (previous, directive) in ledger.directives.iter().tuple_windows() {
                if let Directive::Transaction(txn) = &directive.data {
                    // only pads synthesize transactions; checks book nothing
                    assert_ne!(txn.flag, Some(Flag::BalanceCheck));
                    if txn.flag == Some(Flag::BalancePad) {
                        synthesized += 1;
                        assert!(matches!(previous.data, Directive::BalancePad(_)));
                        assert_eq!(previous.span, directive.span);
                    }
                }
            }
            assert_eq!(synthesized, 1);
            // four checks and the `balance ... with pad`
            assert_eq!(ledger.store.read().unwrap().balance_assertions.len(), 5);
            // re-sorting the final stream changes nothing
            assert_eq!(ledger.directives.clone(), Ledger::sort_directives_datetime(ledger.directives.clone()));
        }
    }
    mod active_accounts {
        use indoc::indoc;
        use itertools::Itertools;
        use zhang_ast::error::ErrorKind;

        use crate::ledger::test::load_from_temp_str;
        use crate::ledger::Ledger;

        /// (kind, first line of the span, `account_name` meta) of every error, in store order
        fn errors(ledger: &Ledger) -> Vec<(ErrorKind, String, Option<String>)> {
            let store = ledger.store.read().unwrap();
            store
                .errors
                .iter()
                .map(|it| {
                    let span = it.span.as_ref().and_then(|span| span.content.lines().next()).unwrap_or_default();
                    (it.error_type.clone(), span.to_owned(), it.metas.get("account_name").cloned())
                })
                .collect_vec()
        }

        fn error(kind: ErrorKind, span: &str, account: &str) -> (ErrorKind, String, Option<String>) {
            (kind, span.to_owned(), Some(account.to_owned()))
        }

        #[test]
        fn should_report_a_typo_and_a_closed_account_once_each() {
            // the ledger of #444
            let ledger = load_from_temp_str(indoc! {r#"
                option "operating_currency" "CNY"
                1970-01-01 commodity CNY
                2023-01-01 open Assets:Cash CNY
                2023-01-01 open Expenses:Old CNY
                2023-01-05 close Expenses:Old
                2023-02-01 "Shop" "typo in account name"
                  Assets:Cash -10 CNY
                  Expenses:Fodo 10 CNY
                2023-02-02 "Shop" "posting to a closed account"
                  Assets:Cash -5 CNY
                  Expenses:Old 5 CNY
            "#});

            assert_eq!(
                errors(&ledger),
                vec![
                    error(ErrorKind::AccountDoesNotExist, r#"2023-02-01 "Shop" "typo in account name""#, "Expenses:Fodo"),
                    error(ErrorKind::AccountClosed, r#"2023-02-02 "Shop" "posting to a closed account""#, "Expenses:Old"),
                ]
            );
            // report-only: both transactions are still booked
            assert_eq!(ledger.store.read().unwrap().transactions.len(), 2);
        }

        #[test]
        fn should_keep_an_account_active_through_its_close_day() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Cash
                1970-01-01 open Expenses:Old
                2023-01-05 close Expenses:Old
                2023-01-05 * "on the close day, written after the close"
                  Assets:Cash -1 CNY
                  Expenses:Old 1 CNY
                2023-01-05 23:59:59 * "late on the close day"
                  Assets:Cash -1 CNY
                  Expenses:Old 1 CNY
                2023-01-06 * "the day after the close"
                  Assets:Cash -1 CNY
                  Expenses:Old 1 CNY
            "#});

            assert_eq!(
                errors(&ledger),
                vec![error(ErrorKind::AccountClosed, r#"2023-01-06 * "the day after the close""#, "Expenses:Old")]
            );
        }

        #[test]
        fn should_report_a_posting_before_the_open() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Cash
                2023-01-01 * "the day before the open"
                  Assets:Cash -1 CNY
                  Expenses:Late 1 CNY
                2023-01-02 * "on the open day, written before the open"
                  Assets:Cash -1 CNY
                  Expenses:Late 1 CNY
                2023-01-02 open Expenses:Late
            "#});

            assert_eq!(
                errors(&ledger),
                vec![error(
                    ErrorKind::AccountDoesNotExist,
                    r#"2023-01-01 * "the day before the open""#,
                    "Expenses:Late"
                )]
            );
        }

        #[test]
        fn should_report_each_account_once_per_transaction() {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Cash
                2023-01-01 * "one typo, twice"
                  Assets:Cash -2 CNY
                  Expenses:Fodo 1 CNY
                  Expenses:Fodo 1 CNY
                2023-01-02 * "two typos"
                  Expenses:Fodo 1 CNY
                  Assets:Csh -1 CNY
            "#});

            assert_eq!(
                errors(&ledger),
                vec![
                    error(ErrorKind::AccountDoesNotExist, r#"2023-01-01 * "one typo, twice""#, "Expenses:Fodo"),
                    error(ErrorKind::AccountDoesNotExist, r#"2023-01-02 * "two typos""#, "Expenses:Fodo"),
                    error(ErrorKind::AccountDoesNotExist, r#"2023-01-02 * "two typos""#, "Assets:Csh"),
                ]
            );
        }

        #[test]
        fn should_not_report_the_accounts_of_pad_and_balance_twice() {
            // the stages report them on their directives; the `P` transactions the pads synthesize
            // post to the same accounts and must not report them again
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Assets:Closed
                1970-01-01 open Equity:Open
                1970-01-02 close Assets:Closed
                2023-01-01 balance Assets:A 10 CNY with pad Equity:Missing
                2023-01-02 balance Assets:Gone 10 CNY with pad Equity:Open
                2023-01-03 balance Assets:Missing 0 CNY
                2023-01-04 balance Assets:Closed 0 CNY
            "#});

            let pad_a = "2023-01-01 balance Assets:A 10 CNY with pad Equity:Missing";
            let pad_gone = "2023-01-02 balance Assets:Gone 10 CNY with pad Equity:Open";
            assert_eq!(
                errors(&ledger),
                vec![
                    error(ErrorKind::AccountDoesNotExist, pad_a, "Equity:Missing"),
                    error(ErrorKind::AccountDoesNotExist, pad_gone, "Assets:Gone"),
                    error(ErrorKind::AccountDoesNotExist, "2023-01-03 balance Assets:Missing 0 CNY", "Assets:Missing"),
                    error(ErrorKind::AccountClosed, "2023-01-04 balance Assets:Closed 0 CNY", "Assets:Closed"),
                ]
            );
            // the padding transactions are still booked, and the checks kept for the journal, those of the
            // `balance ... with pad` directives too
            let store = ledger.store.read().unwrap();
            assert_eq!(store.transactions.len(), 2);
            assert_eq!(store.balance_assertions.len(), 4);
        }
    }
    mod options {
        use indoc::indoc;

        use crate::ledger::test::load_from_temp_str;

        #[test]
        fn should_get_price() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                    option "title" "Example Beancount file"
                    option "operating_currency" "USD"
                "#});
            let operations = ledger.operations();

            assert_eq!("Example Beancount file", operations.option::<String>("title")?.unwrap());
            assert_eq!("USD", operations.option::<String>("operating_currency")?.unwrap());
            assert!(operations.option::<String>("operating_currency2")?.is_none());
            Ok(())
        }
    }

    mod extract_info {
        use indoc::indoc;

        use crate::domains::schemas::AccountStatus;
        use crate::ledger::test::load_from_temp_str;

        #[test]
        fn should_extract_account_open() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 open Assets:Hello CNY
                "#});
            let store = ledger.store.read().unwrap();
            let account = store.accounts.get("Assets:Hello").unwrap();
            assert_eq!(account.status, AccountStatus::Open);
        }

        #[test]
        fn should_mark_as_close_after_opening_account() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 open Assets:Hello CNY
                    1970-02-01 close Assets:Hello
                "#});
            let store = ledger.store.read().unwrap();
            let account = store.accounts.get("Assets:Hello").unwrap();
            assert_eq!(account.status, AccountStatus::Close);
        }

        #[test]
        fn should_extract_commodities() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 commodity CNY
                    1970-02-01 commodity HKD
                "#});
            let store = ledger.store.read().unwrap();

            assert_eq!(2, store.commodities.len(), "should have 2 commodity");
            assert!(store.commodities.contains_key("CNY"), "should have CNY record");
            assert!(store.commodities.contains_key("HKD"), "should have HKD record");
        }
    }

    mod query {
        use std::sync::Arc;

        use chrono::NaiveDate;
        use indoc::indoc;
        use itertools::Itertools;
        use tempfile::tempdir;
        use zhang_ast::Directive;

        use crate::data_source::LocalFileSystemDataSource;
        use crate::data_type::text::ZhangDataType;
        use crate::domains::schemas::QueryDomain;
        use crate::ledger::test::load_from_temp_str;
        use crate::ledger::Ledger;

        fn saved(date: (i32, u32, u32), name: &str, query: &str) -> QueryDomain {
            QueryDomain {
                date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap(),
                name: name.to_owned(),
                query: query.to_owned(),
            }
        }

        #[test]
        fn should_store_queries_in_ledger_order_and_keep_duplicates() {
            let ledger = load_from_temp_str(indoc! {r#"
                2024-02-01 query "late" "SELECT 1"
                2024-01-01 query "cash" "SELECT account, sum(position) WHERE account ~ 'Cash'"
                  owner: "alice"
                2024-01-01 query "future" "BALANCES AT cost"
                2024-03-01 query "cash" "SELECT date, position"
            "#});

            let operations = ledger.operations();
            assert_eq!(
                operations.queries().unwrap(),
                vec![
                    saved((2024, 1, 1), "cash", "SELECT account, sum(position) WHERE account ~ 'Cash'"),
                    saved((2024, 1, 1), "future", "BALANCES AT cost"),
                    saved((2024, 2, 1), "late", "SELECT 1"),
                    saved((2024, 3, 1), "cash", "SELECT date, position"),
                ]
            );
            // the text is not validated at load time
            assert!(operations.read().errors.is_empty());
            // the directives stay in the ledger's directive list
            assert_eq!(ledger.directives.iter().filter(|it| matches!(it.data, Directive::Query(_))).count(), 4);
        }

        #[test]
        fn should_store_queries_from_included_files() {
            let temp_dir = tempdir().unwrap().keep();
            std::fs::write(
                temp_dir.join("main.zhang"),
                indoc! {r#"
                    include "queries.zhang"
                    2024-01-01 query "main" "SELECT account"
                "#},
            )
            .unwrap();
            std::fs::write(temp_dir.join("queries.zhang"), "2024-01-02 query \"included\" \"SELECT payee\"\n").unwrap();
            let source = LocalFileSystemDataSource::new(ZhangDataType {});
            let ledger = Ledger::load_with_data_source(temp_dir, "main.zhang".to_string(), Arc::new(source)).unwrap();

            let names = ledger.operations().queries().unwrap().into_iter().map(|it| it.name).collect_vec();
            assert_eq!(names, vec!["main", "included"]);
            assert_eq!(ledger.visited_files.len(), 2);
        }
    }

    mod extra_inputs {
        use indoc::indoc;

        use crate::ledger::test::load_from_temp_str;

        #[test]
        fn should_not_record_the_module_of_a_plugin_that_is_not_loaded() {
            let ledger = load_from_temp_str(indoc! {r#"
                plugin "plugins/echo.wasm"
                1970-01-01 open Assets:Cash
            "#});

            // plugins are off, so the module is never read
            assert!(ledger.extra_inputs.is_empty());
            assert_eq!(ledger.visited_files.len(), 1);
        }

        #[cfg(feature = "plugin_runtime")]
        mod plugin_module {
            use std::path::PathBuf;
            use std::sync::Arc;

            use indoc::indoc;
            use itertools::Itertools;
            use tempfile::tempdir;

            use crate::data_source::LocalFileSystemDataSource;
            use crate::data_type::text::ZhangDataType;
            use crate::inputs::ExtraInput;
            use crate::ledger::Ledger;

            const ECHO: &str = include_str!("../tests/plugins/echo.wat");

            /// load `main.zhang` from a fresh ledger root holding the echo plugin at
            /// `plugins/echo.wat`; `{module}` in `content` is the module's absolute path
            fn load_with_plugin(content: &str) -> (PathBuf, Ledger) {
                // canonical, like the root `load_with_data_source` resolves
                let root = tempdir().unwrap().keep().canonicalize().unwrap();
                std::fs::create_dir(root.join("plugins")).unwrap();
                let module = root.join("plugins/echo.wat");
                std::fs::write(&module, ECHO).unwrap();
                let content = content.replace("{module}", &module.to_string_lossy());
                std::fs::write(root.join("main.zhang"), content).unwrap();
                let source = LocalFileSystemDataSource::new(ZhangDataType {});
                let ledger = Ledger::load_with_data_source(root.clone(), "main.zhang".to_string(), Arc::new(source)).unwrap();
                (root, ledger)
            }

            const LEDGER: &str = indoc! {r#"
                option "features.plugin" "true"
                plugin "{module}"
                1970-01-01 open Assets:Cash
                1970-01-01 open Equity:Open
                2024-01-01 * "lunch"
                  Assets:Cash -10 CNY
                  Equity:Open
            "#};

            #[test]
            fn should_record_a_local_plugin_module_relative_to_the_root() {
                let (root, ledger) = load_with_plugin(LEDGER);

                // the plugin ran: its echo of the stream reached the store
                assert_eq!(ledger.plugins.ordered.len(), 1);
                assert_eq!(ledger.store.read().unwrap().transactions.len(), 1);
                assert_eq!(
                    ledger.extra_inputs.iter().cloned().collect_vec(),
                    vec![ExtraInput::File(PathBuf::from("plugins/echo.wat"))]
                );
                // the module stays out of the file editor's list
                assert_eq!(ledger.visited_files, vec![root.join("main.zhang")]);
            }

            #[test]
            fn should_record_a_module_declared_twice_once() {
                let (_, ledger) = load_with_plugin(&LEDGER.replace("plugin \"{module}\"", "plugin \"{module}\"\nplugin \"{module}\""));

                assert_eq!(ledger.plugins.ordered.len(), 2);
                assert_eq!(ledger.extra_inputs.len(), 1);
            }

            #[test]
            fn should_record_extra_inputs_again_on_reload() {
                let (root, mut ledger) = load_with_plugin(LEDGER);

                ledger.reload().unwrap();

                assert_eq!(
                    ledger.extra_inputs.iter().cloned().collect_vec(),
                    vec![ExtraInput::File(PathBuf::from("plugins/echo.wat"))]
                );
                assert_eq!(ledger.visited_files, vec![root.join("main.zhang")]);
            }
        }
    }

    mod clock {
        use std::sync::Arc;

        use indoc::indoc;
        use tempfile::tempdir;

        use crate::clock::Clock;
        use crate::data_source::{DataSource, LocalFileSystemDataSource};
        use crate::data_type::text::ZhangDataType;
        use crate::inputs::ExtraInput;
        use crate::ledger::test::load_from_temp_str;
        use crate::ledger::{Ledger, LedgerProcessContext};

        const LEDGER: &str = indoc! {r#"
            1970-01-01 open Assets:Cash
            1970-01-01 open Equity:Open
            2024-01-01 * "lunch"
              Assets:Cash -10 CNY
              Equity:Open
        "#};

        #[test]
        fn should_not_read_the_clock_when_nothing_asks_for_the_time() {
            let ledger = load_from_temp_str(LEDGER);

            assert_eq!(ledger.clock(), Clock::System);
            assert_eq!(ledger.clock_reading(), None);
            assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock));
        }

        #[test]
        fn should_reload_with_the_same_clock() {
            let root = tempdir().unwrap().keep().canonicalize().unwrap();
            std::fs::write(root.join("main.zhang"), LEDGER).unwrap();
            let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
            let loaded = source.load(root.to_string_lossy().to_string(), "main.zhang".to_owned()).unwrap();
            let fixed = "2024-03-15T16:30:00Z".parse().unwrap();
            let mut ledger = Ledger::process(LedgerProcessContext {
                directives: loaded.directives,
                entry: (root, "main.zhang".to_owned()),
                visited_files: loaded.visited_files,
                data_source: source,
                clock: Clock::Fixed(fixed),
            })
            .unwrap();

            ledger.reload().unwrap();

            assert_eq!(ledger.clock(), Clock::Fixed(fixed));
            assert_eq!(ledger.clock_reading(), None, "nothing asked for the time on reload either");
            assert_eq!(ledger.store.read().unwrap().transactions.len(), 1);
        }
    }

    mod account {
        use indoc::indoc;

        use crate::ledger::test::load_from_temp_str;

        #[test]
        fn should_return_true_given_exists_account() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Bank
            "#});

            let mut operations = ledger.operations();
            assert!(operations.exist_account("Assets:Bank")?);
            assert!(!operations.exist_account("Assets:Bank2")?);
            Ok(())
        }
    }

    mod budget {
        use std::collections::BTreeMap;

        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use itertools::Itertools;
        use zhang_ast::error::ErrorKind;

        use crate::ledger::test::load_from_temp_str;
        use crate::ledger::Ledger;

        const ACCOUNTS: &str = indoc! {r#"
            option "operating_currency" "CNY"
            1970-01-01 commodity CNY
            2023-01-01 open Assets:Cash CNY
            2023-01-01 open Expenses:Food CNY
              budget: "Food"
        "#};

        fn load(body: &str) -> Ledger {
            load_from_temp_str(&format!("{ACCOUNTS}{body}"))
        }

        /// reported errors in store order, with the first line of their span and their metas
        fn errors(ledger: &Ledger) -> Vec<(ErrorKind, String, BTreeMap<String, String>)> {
            let store = ledger.store.read().unwrap();
            store
                .errors
                .iter()
                .map(|it| {
                    let span = it.span.as_ref().and_then(|span| span.content.lines().next()).unwrap_or_default();
                    (it.error_type.clone(), span.to_owned(), it.metas.clone().into_iter().collect())
                })
                .collect_vec()
        }

        fn undefined_budget(span: &str, account: &str, budget: &str) -> (ErrorKind, String, BTreeMap<String, String>) {
            let metas = BTreeMap::from([("account_name".to_owned(), account.to_owned()), ("budget_name".to_owned(), budget.to_owned())]);
            (ErrorKind::BudgetDoesNotExist, span.to_owned(), metas)
        }

        /// balance of `account` after the ledger, in CNY
        fn balance(ledger: &Ledger, account: &str) -> BigDecimal {
            let store = ledger.store.read().unwrap();
            store
                .transactions
                .values()
                .flat_map(|txn| txn.postings.iter())
                .filter(|posting| posting.account.name() == account)
                .map(|posting| posting.inferred_amount.number.clone())
                .sum()
        }

        #[test]
        fn should_report_an_undefined_budget_and_book_the_transaction() {
            // this used to panic with `budget does not exist` (#446)
            let ledger = load(indoc! {r#"
                2023-02-01 "Shop" "lunch"
                  Assets:Cash -10 CNY
                  Expenses:Food 10 CNY
            "#});

            assert_eq!(errors(&ledger), vec![undefined_budget(r#"2023-02-01 "Shop" "lunch""#, "Expenses:Food", "Food")]);
            assert_eq!(ledger.store.read().unwrap().transactions.len(), 1);
            assert_eq!(balance(&ledger, "Assets:Cash"), BigDecimal::from(-10));
            assert_eq!(balance(&ledger, "Expenses:Food"), BigDecimal::from(10));
            assert!(ledger.defined_budgets.is_none(), "validation state is dropped after loading");
        }

        #[test]
        fn should_report_an_undefined_budget_once_per_account() {
            let ledger = load(indoc! {r#"
                2023-01-01 open Expenses:Snack CNY
                  budget: "Food"
                2023-02-01 "Shop" "lunch"
                  Assets:Cash -10 CNY
                  Expenses:Food 10 CNY
                2023-02-02 "Shop" "dinner"
                  Assets:Cash -20 CNY
                  Expenses:Food 20 CNY
                2023-03-01 "Shop" "snack"
                  Assets:Cash -5 CNY
                  Expenses:Snack 5 CNY
            "#});

            assert_eq!(
                errors(&ledger),
                vec![
                    undefined_budget(r#"2023-02-01 "Shop" "lunch""#, "Expenses:Food", "Food"),
                    undefined_budget(r#"2023-03-01 "Shop" "snack""#, "Expenses:Snack", "Food"),
                ]
            );
            assert_eq!(ledger.store.read().unwrap().transactions.len(), 3);
            assert_eq!(balance(&ledger, "Expenses:Food"), BigDecimal::from(30));
            assert_eq!(balance(&ledger, "Assets:Cash"), BigDecimal::from(-35));
        }

        #[test]
        fn should_book_postings_to_a_defined_budget() {
            let ledger = load(indoc! {r#"
                2023-01-01 budget Food CNY
                2023-02-01 "Shop" "lunch"
                  Assets:Cash -10 CNY
                  Expenses:Food 10 CNY
                2023-02-02 "Shop" "dinner"
                  Assets:Cash -20 CNY
                  Expenses:Food 20 CNY
            "#});

            assert_eq!(errors(&ledger), vec![]);
            assert_eq!(balance(&ledger, "Expenses:Food"), BigDecimal::from(30));
        }

        #[test]
        fn should_report_postings_before_the_budget_is_defined() {
            // like `budget-add`, a budget only exists from its definition on in the stream
            let ledger = load(indoc! {r#"
                2023-02-01 "Shop" "lunch"
                  Assets:Cash -10 CNY
                  Expenses:Food 10 CNY
                2023-03-01 budget Food CNY
                2023-03-02 "Shop" "dinner"
                  Assets:Cash -20 CNY
                  Expenses:Food 20 CNY
            "#});

            assert_eq!(errors(&ledger), vec![undefined_budget(r#"2023-02-01 "Shop" "lunch""#, "Expenses:Food", "Food")]);
            assert_eq!(ledger.store.read().unwrap().transactions.len(), 2);
            assert_eq!(balance(&ledger, "Expenses:Food"), BigDecimal::from(30));
        }
    }
}
