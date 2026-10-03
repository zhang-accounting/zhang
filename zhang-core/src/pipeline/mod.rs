//! The directive-processing pipeline.
//!
//! A [`ProcessStage`] is a pure transform over the whole directive stream with an
//! error channel: `(directives, ctx) -> directives`, problems reported via
//! [`StageContext::emit_error`]. Core ops (as native Rust stages) and user plugins
//! (adapted from WASM) share this one interface; the executor runs them in
//! declaration order and re-sorts the stream after every stage, so a stage never
//! has to maintain ordering itself.
//!
//! A stage that reads anything besides the stream (a file, the current date)
//! records it with [`StageContext::add_input`], so the ledger knows when it is stale.
//! [`StageContext::now`] gives the current time of the load and records the date as
//! an input itself.
//!
//! Built-in stages ([`builtin_stages`]) run after the user's plugin stages:
//! [`ActiveAccountsStage`], which only reports references to inactive accounts,
//! then [`PadStage`] then [`BalanceCheckStage`], two independent folds over the
//! stream that share only the pure helpers in the `balance` module.
//!
//! A balance assertion never moves a balance (as in beancount): [`PadStage`] adds the
//! padding transactions, the only directives that book anything on behalf of an
//! assertion, and [`BalanceCheckStage`] only checks. It records what it found for each
//! assertion with [`StageContext::record_assertion`], which the store keeps for the journal.

mod active_accounts;
pub(crate) mod balance;
mod balance_check;
mod pad;
mod plugin_view;

use std::collections::{HashMap, VecDeque};

pub use active_accounts::ActiveAccountsStage;
pub use balance_check::BalanceCheckStage;
use chrono::DateTime;
use chrono_tz::Tz;
use indexmap::IndexSet;
use log::debug;
pub use pad::PadStage;
pub use plugin_view::AbiV1View;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Spanned};

use crate::clock::{Clock, LoadClock};
use crate::domains::schemas::{CommodityDomain, OptionDomain};
use crate::inputs::ExtraInput;
use crate::ledger::Ledger;
use crate::utils::id::FromSpan;
use crate::ZhangResult;

/// a problem reported by a stage; collected by the executor and materialized
/// into the store after the pipeline finishes
pub struct StageError {
    pub kind: ErrorKind,
    pub span: SpanInfo,
    pub metas: HashMap<String, String>,
}

/// what [`BalanceCheckStage`] found for one `balance` assertion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionOutcome {
    /// the account's balance in the asserted currency where the assertion stands: the sum of the
    /// postings of the account and all its sub-accounts before it. The assertion itself changes
    /// no balance
    pub balance: Amount,
    /// whether `balance` is within the assertion's tolerance of the asserted amount. A failing
    /// assertion is also reported as an [`ErrorKind::AccountBalanceCheckError`]
    pub passed: bool,
}

/// the [`AssertionOutcome`]s of a load, by the id of the assertion's span (the id the store gives it).
/// Assertions sharing a span, which a plugin may emit, are taken in the order they were recorded
#[derive(Debug, Default)]
pub struct AssertionOutcomes(HashMap<Uuid, VecDeque<AssertionOutcome>>);

impl AssertionOutcomes {
    pub fn record(&mut self, span: &SpanInfo, outcome: AssertionOutcome) {
        self.0.entry(Uuid::from_span(span)).or_default().push_back(outcome);
    }

    /// the outcome of the next assertion with this span, `None` if none was recorded
    pub fn take(&mut self, span: &SpanInfo) -> Option<AssertionOutcome> {
        self.0.get_mut(&Uuid::from_span(span)).and_then(VecDeque::pop_front)
    }
}

/// context handed to every stage
pub struct StageContext<'a> {
    /// the ledger's resolved options
    pub options: &'a [OptionDomain],
    /// the commodities the options defined before the pipeline ran (the operating currency);
    /// `commodity` directives are in the stream
    pub commodities: Vec<CommodityDomain>,
    errors: Vec<StageError>,
    assertions: AssertionOutcomes,
    inputs: IndexSet<ExtraInput>,
    /// the clock of the load, read on first use
    clock: LoadClock,
    /// the ledger timezone, which [`StageContext::now`] gives the time in
    timezone: Tz,
}

impl<'a> StageContext<'a> {
    /// a context on the system clock, in UTC
    pub fn new(options: &'a [OptionDomain]) -> Self {
        Self {
            options,
            commodities: vec![],
            errors: vec![],
            assertions: AssertionOutcomes::default(),
            inputs: IndexSet::new(),
            clock: LoadClock::new(Clock::System),
            timezone: Tz::UTC,
        }
    }

    /// the context with the commodities the options defined
    pub fn with_commodities(mut self, commodities: Vec<CommodityDomain>) -> Self {
        self.commodities = commodities;
        self
    }

    /// the context reading the time from `clock`, the clock of the load, in the ledger timezone `timezone`
    pub fn with_clock(mut self, clock: LoadClock, timezone: Tz) -> Self {
        self.clock = clock;
        self.timezone = timezone;
        self
    }

    /// the current time of the load, in the ledger timezone. The first call of the load reads the clock, and every
    /// later call returns the same instant, from any stage, so all stages agree on "today". It records
    /// [`ExtraInput::Clock`]: the output of the load now depends on the date
    pub fn now(&mut self) -> DateTime<Tz> {
        self.add_input(ExtraInput::Clock);
        self.clock.now().with_timezone(&self.timezone)
    }

    /// the clock of the load, for a stage that hands it on, such as to a WASM plugin's host functions. Reading it
    /// records nothing: a stage doing so records [`ExtraInput::Clock`] itself
    pub fn clock(&self) -> &LoadClock {
        &self.clock
    }

    /// the ledger timezone
    pub fn timezone(&self) -> Tz {
        self.timezone
    }

    /// report a problem without aborting the pipeline
    pub fn emit_error(&mut self, kind: ErrorKind, span: SpanInfo, metas: HashMap<String, String>) {
        self.errors.push(StageError { kind, span, metas });
    }

    /// record something a stage read besides the stream; recording it again changes nothing
    pub fn add_input(&mut self, input: ExtraInput) {
        self.inputs.insert(input);
    }

    /// the inputs stages recorded, in the order they were first recorded
    pub fn inputs(&self) -> &IndexSet<ExtraInput> {
        &self.inputs
    }

    /// record what a `balance` assertion, the directive at `span`, was checked against
    pub fn record_assertion(&mut self, span: &SpanInfo, outcome: AssertionOutcome) {
        self.assertions.record(span, outcome);
    }

    /// consume the context and take the errors stages reported
    pub fn into_errors(self) -> Vec<StageError> {
        self.errors
    }

    /// consume the context and take the errors stages reported, and the outcomes of the balance assertions
    pub fn into_results(self) -> (Vec<StageError>, AssertionOutcomes) {
        (self.errors, self.assertions)
    }
}

/// one step of the pipeline: transform the whole directive stream
pub trait ProcessStage {
    fn name(&self) -> &str;

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>>;
}

/// the native core stages, in execution order; they run after all plugin stages.
/// [`ActiveAccountsStage`] checks the stream before the pad stage adds its `P`
/// transactions; the pad/check stages report the accounts of their directives themselves
pub fn builtin_stages() -> Vec<Box<dyn ProcessStage>> {
    vec![Box::new(ActiveAccountsStage), Box::new(PadStage), Box::new(BalanceCheckStage)]
}

/// run stages in order; the stream is re-sorted after every stage
/// ("don't trust the stages" — beancount does the same after every plugin).
///
/// The sort is stable and by datetime; within one datetime, balance entries
/// (balance pad/check directives and `P`-flagged transactions) come first,
/// after `open`. A stage inserting a padding transaction right after its
/// directive can rely on it staying there.
pub fn run_pipeline(stages: &[Box<dyn ProcessStage>], mut directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
    for stage in stages {
        debug!("running pipeline stage: {}", stage.name());
        directives = stage.process(directives, ctx)?;
        directives = Ledger::sort_directives_datetime(directives);
    }
    Ok(directives)
}

#[cfg(test)]
pub(crate) mod test {
    use std::collections::HashMap;

    use chrono::{DateTime, Utc};
    use chrono_tz::Tz;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Comment, Directive, SpanInfo, Spanned};

    use super::{builtin_stages, run_pipeline, AssertionOutcome, ProcessStage, StageContext};
    use crate::clock::{Clock, LoadClock};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::inputs::ExtraInput;
    use crate::ledger::Ledger;
    use crate::ZhangResult;

    /// parse a ledger, run the built-in stages over it and return the output
    /// stream with the kinds of the errors the stages reported
    pub(crate) fn run_builtin_stages(content: &str) -> (Vec<Directive>, Vec<ErrorKind>) {
        let (directives, errors, _) = run_builtin_stages_with_assertions(content);
        (directives, errors)
    }

    /// [`run_builtin_stages`], with the outcomes of the balance checks in stream order
    pub(crate) fn run_builtin_stages_with_assertions(content: &str) -> (Vec<Directive>, Vec<ErrorKind>, Vec<AssertionOutcome>) {
        let directives = ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        let mut ctx = StageContext::new(&[]);
        let out = run_pipeline(&builtin_stages(), Ledger::sort_directives_datetime(directives), &mut ctx).unwrap();
        let (errors, mut assertions) = ctx.into_results();
        let outcomes = out
            .iter()
            .filter(|it| matches!(it.data, Directive::BalanceCheck(_)))
            .map(|it| assertions.take(&it.span).expect("every check records its outcome"))
            .collect();
        (
            out.into_iter().map(|it| it.data).collect(),
            errors.into_iter().map(|it| it.kind).collect(),
            outcomes,
        )
    }

    fn span() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 0,
            content: "".to_string(),
            filename: None,
        }
    }

    struct AppendCommentStage(&'static str);
    impl ProcessStage for AppendCommentStage {
        fn name(&self) -> &str {
            "append-comment"
        }
        fn process(&self, mut directives: Vec<Spanned<Directive>>, _ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            directives.push(Spanned::new(Directive::Comment(Comment { content: self.0.to_owned() }), span()));
            Ok(directives)
        }
    }

    struct EmitErrorStage;
    impl ProcessStage for EmitErrorStage {
        fn name(&self) -> &str {
            "emit-error"
        }
        fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            ctx.emit_error(ErrorKind::ParseInvalidMeta, span(), HashMap::default());
            Ok(directives)
        }
    }

    struct ReadStage(Vec<ExtraInput>);
    impl ProcessStage for ReadStage {
        fn name(&self) -> &str {
            "read"
        }
        fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            for input in &self.0 {
                ctx.add_input(input.clone());
            }
            Ok(directives)
        }
    }

    #[test]
    fn should_collect_each_input_once_in_first_recorded_order() {
        let receipt = ExtraInput::File("documents/receipt.pdf".into());
        let documents = ExtraInput::Dir("documents".into());
        let stages: Vec<Box<dyn ProcessStage>> = vec![
            Box::new(ReadStage(vec![receipt.clone(), ExtraInput::Clock, receipt.clone()])),
            Box::new(ReadStage(vec![documents.clone(), ExtraInput::Clock])),
        ];
        let mut ctx = StageContext::new(&[]);

        run_pipeline(&stages, vec![], &mut ctx).unwrap();

        assert_eq!(ctx.inputs().iter().cloned().collect::<Vec<_>>(), vec![receipt, ExtraInput::Clock, documents]);
    }

    /// appends a comment holding the time `ctx.now()` gives
    struct NowStage;
    impl ProcessStage for NowStage {
        fn name(&self) -> &str {
            "now"
        }
        fn process(&self, mut directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            let content = ctx.now().to_rfc3339();
            directives.push(Spanned::new(Directive::Comment(Comment { content }), span()));
            Ok(directives)
        }
    }

    fn comments(directives: &[Spanned<Directive>]) -> Vec<&str> {
        directives
            .iter()
            .filter_map(|it| match &it.data {
                Directive::Comment(c) => Some(c.content.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn should_give_every_stage_the_same_time_in_the_ledger_timezone_and_record_the_date() {
        let fixed = "2024-03-15T16:30:00Z".parse::<DateTime<Utc>>().unwrap();
        let clock = LoadClock::new(Clock::Fixed(fixed));
        let stages: Vec<Box<dyn ProcessStage>> = vec![Box::new(NowStage), Box::new(AppendCommentStage("between")), Box::new(NowStage)];
        let mut ctx = StageContext::new(&[]).with_clock(clock.clone(), Tz::Asia__Shanghai);

        let out = run_pipeline(&stages, vec![], &mut ctx).unwrap();

        assert_eq!(comments(&out), vec!["2024-03-16T00:30:00+08:00", "between", "2024-03-16T00:30:00+08:00"]);
        assert_eq!(ctx.inputs().iter().cloned().collect::<Vec<_>>(), vec![ExtraInput::Clock]);
        assert_eq!(clock.reading(), Some(fixed));
    }

    #[test]
    fn should_not_read_the_clock_or_record_the_date_when_no_stage_asks() {
        let clock = LoadClock::new(Clock::System);
        let stages: Vec<Box<dyn ProcessStage>> = vec![Box::new(AppendCommentStage("only"))];
        let mut ctx = StageContext::new(&[]).with_clock(clock.clone(), Tz::UTC);

        run_pipeline(&stages, vec![], &mut ctx).unwrap();

        assert!(ctx.inputs().is_empty());
        assert_eq!(clock.reading(), None);
    }

    #[test]
    fn should_run_stages_in_order_and_collect_errors() {
        let stages: Vec<Box<dyn ProcessStage>> = vec![
            Box::new(AppendCommentStage("first")),
            Box::new(EmitErrorStage),
            Box::new(AppendCommentStage("second")),
        ];
        let mut ctx = StageContext::new(&[]);

        let out = run_pipeline(&stages, vec![], &mut ctx).unwrap();

        assert_eq!(comments(&out), vec!["first", "second"]);

        let errors = ctx.into_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ErrorKind::ParseInvalidMeta);
    }
}
