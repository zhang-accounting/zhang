use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, RwLock};

use itertools::Itertools;
use log::{error, info};
use zhang_ast::{Directive, Flag, Options, Plugin, SpanInfo, Spanned};

use crate::data_source::DataSource;
use crate::domains::Operations;
use crate::error::IoErrorIntoZhangError;
use crate::options::{BuiltinOption, InMemoryOptions};
use crate::pipeline::{builtin_stages, run_pipeline, ProcessStage, StageContext};
use crate::process::{DirectivePreProcess, DirectiveProcess};
use crate::store::Store;
use crate::{ZhangError, ZhangResult};

pub struct Ledger {
    pub entry: (PathBuf, String),

    pub data_source: Arc<dyn DataSource>,

    pub visited_files: Vec<PathBuf>,

    pub options: InMemoryOptions,

    pub directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,

    pub store: Arc<RwLock<Store>>,

    pub(crate) trx_counter: AtomicI32,

    #[cfg(feature = "plugin_runtime")]
    pub plugins: crate::plugin::store::PluginStore,
}

pub struct LedgerProcessContext {
    pub directives: Vec<Spanned<Directive>>,
    pub entry: (PathBuf, String),
    pub visited_files: Vec<PathBuf>,
    pub data_source: Arc<dyn DataSource>,
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
    pub fn load<T: DataSource + Default + 'static>(entry: PathBuf, endpoint: String) -> ZhangResult<Ledger> {
        let data_source = Arc::new(T::default());
        Ledger::load_with_data_source(entry, endpoint, data_source)
    }

    pub fn load_with_data_source(entry: PathBuf, endpoint: String, data_source: Arc<dyn DataSource>) -> ZhangResult<Ledger> {
        let entry = entry.canonicalize().with_path(&entry)?;

        let load_result = data_source.load(entry.to_string_lossy().to_string(), endpoint.clone())?;
        Ledger::process(LedgerProcessContext {
            directives: load_result.directives,
            entry: (entry, endpoint),
            visited_files: load_result.visited_files,
            data_source,
        })
    }
    pub async fn async_load(entry: PathBuf, endpoint: String, data_source: Arc<dyn DataSource>) -> ZhangResult<Ledger> {
        let load_result = data_source.async_load(entry.to_string_lossy().to_string(), endpoint.clone()).await?;

        Ledger::async_process(LedgerProcessContext {
            directives: load_result.directives,
            entry: (entry, endpoint),
            visited_files: load_result.visited_files,
            data_source,
        })
        .await
    }

    fn init(context: LedgerProcessContext) -> (Self, SplitDirectives) {
        let ledger = Self {
            options: InMemoryOptions::default(),
            entry: context.entry,
            visited_files: context.visited_files,
            directives: vec![],
            metas: vec![],
            data_source: context.data_source,
            store: Default::default(),
            trx_counter: AtomicI32::new(1),
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
        let processed = self.run_stages(full_stream)?;
        let (metas, mut dated) = Ledger::partition_processed_directives(processed);
        self.handle_other_directives(&mut dated)?;
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
        })
        .await?;
        *self = reload_ledger;
        Ok(())
    }

    pub fn operations(&self) -> Operations {
        let timezone = self.options.timezone;
        Operations {
            store: self.store.clone(),
            timezone,
        }
    }
}

impl Ledger {
    /// sort the stream by the key `(datetime, rank)`, stable — directives with an
    /// equal key keep their source order. This is a total order, so sorting is
    /// idempotent and well-defined for any input.
    ///
    /// - undated directives (option, plugin, include, comment) come first
    /// - within one datetime: `open` and `commodity` (rank 0, together so that an
    ///   `open` naming a same-day commodity keeps its source order), then balance
    ///   entries (rank 1), then everything else (rank 2)
    /// - balance entries are balance pad/check directives *and* transactions
    ///   flagged `P`/`C` — the flags of the transactions those directives
    ///   materialize into. A stage inserting a balance transaction right after its
    ///   directive can therefore rely on it staying there. Hand-written `P`/`C`
    ///   transactions are balance entries too.
    pub(crate) fn sort_directives_datetime(mut directives: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        fn rank(directive: &Directive) -> u8 {
            match directive {
                Directive::Open(_) | Directive::Commodity(_) => 0,
                Directive::BalancePad(_) | Directive::BalanceCheck(_) => 1,
                Directive::Transaction(txn) if matches!(txn.flag, Some(Flag::BalancePad | Flag::BalanceCheck)) => 1,
                _ => 2,
            }
        }
        // `sort_by_key` is stable; `None` (undated) sorts before any datetime
        directives.sort_by_key(|it| (it.datetime(), rank(&it.data)));
        directives
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

    /// fold the pipeline's output into the store
    fn handle_other_directives(&mut self, directives: &mut [Spanned<Directive>]) -> Result<(), ZhangError> {
        for directive in directives.iter_mut() {
            match &mut directive.data {
                // only dated directives reach the fold: options/plugins were handled
                // before the pipeline, and the undated arms below are unreachable
                Directive::Option(_) => {}
                Directive::Open(open) => open.handler(self, &directive.span)?,
                Directive::Close(close) => close.handler(self, &directive.span)?,
                Directive::Commodity(commodity) => commodity.handler(self, &directive.span)?,
                Directive::Transaction(trx) => trx.handler(self, &directive.span)?,
                // the pad/check stages materialized these into transactions
                Directive::BalancePad(_) => {}
                Directive::BalanceCheck(_) => {}
                Directive::Note(_) => {}
                Directive::Document(document) => document.handler(self, &directive.span)?,
                Directive::Price(price) => price.handler(self, &directive.span)?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                Directive::Plugin(_) => {}
                Directive::Include(_) => {}
                Directive::Comment(_) => {}
                Directive::Budget(budget) => budget.handler(self, &directive.span)?,
                Directive::BudgetAdd(budget_add) => budget_add.handler(self, &directive.span)?,
                Directive::BudgetTransfer(budget_transfer) => budget_transfer.handler(self, &directive.span)?,
                Directive::BudgetClose(budget_close) => budget_close.handler(self, &directive.span)?,
            }
        }
        Ok(())
    }

    /// split a stage-processed stream back into (`metas`, `directives`) by datedness.
    /// option/plugin directives stay in the meta list exactly as stages left them; a
    /// stage-synthesized option/plugin does not take effect though — both were handled
    /// before the pipeline ran.
    fn partition_processed_directives(processed: Vec<Spanned<Directive>>) -> (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) {
        let (dated, metas): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) = processed.into_iter().partition(|it| it.datetime().is_some());
        (metas, dated)
    }

    /// the stages to run: the user's WASM plugins in declaration order (only with
    /// the plugin runtime and `features.plugins` on), then the built-in stages
    fn build_stages(&self) -> Vec<Box<dyn ProcessStage>> {
        #[cfg(feature = "plugin_runtime")]
        let plugin_stages = if self.options.features.plugins { self.plugins.build_stages() } else { vec![] };
        #[cfg(not(feature = "plugin_runtime"))]
        let plugin_stages: Vec<Box<dyn ProcessStage>> = vec![];
        plugin_stages.into_iter().chain(builtin_stages()).collect()
    }

    /// run the pipeline over the full directive stream; stage-reported errors are
    /// materialized into the store before the fold
    fn run_stages(&mut self, directives: Vec<Spanned<Directive>>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let directives = Ledger::sort_directives_datetime(directives);
        let stages = self.build_stages();
        let options = self.operations().options()?;
        let mut ctx = StageContext::new(&options);
        let directives = run_pipeline(&stages, directives, &mut ctx)?;

        let mut operations = self.operations();
        for error in ctx.into_errors() {
            operations.new_error(error.kind, &error.span, error.metas)?;
        }
        Ok(directives)
    }
}

#[cfg(test)]
mod test {

    use std::sync::Arc;

    use tempfile::tempdir;
    use zhang_ast::{Directive, SpanInfo, Spanned};

    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;

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
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap()
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
            // a stage inserts a balance directive's transaction (flag `P`/`C`) right after it;
            // the re-sort keeps it there instead of moving it behind every balance entry
            let stream = test_parse_zhang(indoc! {r#"
                1970-01-01 open Assets:Hello
                1970-01-02 balance Assets:Hello 2 CNY with pad Equity:Open
                1970-01-02 P "Balance Pad" "pad"
                  Assets:Hello 2 CNY
                  Equity:Open
                1970-01-02 balance Assets:Hello 2 CNY
                1970-01-02 C "Balance Check" "check"
                  Assets:Hello 0 CNY
                1970-01-02 * "regular"
                  Assets:Hello 1 CNY
                  Equity:Open
            "#});
            let expected = stream.iter().map(|it| it.data.clone()).collect_vec();
            assert_eq!(expected, Ledger::sort_directives_datetime(stream).into_iter().map(|it| it.data).collect_vec());
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
            2023-01-03 C "check" ""
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
                    "txn check",
                    "txn t1",
                    "Note",
                    "Document",
                    "Event",
                ]
            );
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

            let store = ledger.store.read().unwrap();
            let journal = store
                .transactions
                .values()
                .sorted_by_key(|it| it.sequence)
                .map(|it| (it.flag.clone(), it.postings.iter().map(|p| p.after_amount.number.clone()).collect_vec()))
                .collect_vec();
            assert_eq!(
                journal,
                vec![
                    (Flag::BalancePad, vec![BigDecimal::from(100), BigDecimal::from(-100)]),
                    (Flag::BalanceCheck, vec![BigDecimal::from(100)]),
                    (Flag::BalanceCheck, vec![BigDecimal::from(90)]),
                ]
            );
            let errors = store
                .errors
                .iter()
                .map(|it| (it.error_type.clone(), it.metas.get("account_name").cloned()))
                .collect_vec();
            assert_eq!(errors, vec![(ErrorKind::AccountBalanceCheckError, Some("Assets:A".to_owned()))]);
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
            store
                .transactions
                .values()
                .sorted_by_key(|it| it.sequence)
                .map(|it| {
                    let postings = it
                        .postings
                        .iter()
                        .map(|p| (p.account.name().to_owned(), p.inferred_amount.number.clone(), p.after_amount.number.clone()))
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
        fn should_book_same_day_check_then_pad_in_directive_order() {
            // the same journal the store fold produced before the stages existed
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
                    (Flag::BalanceCheck, vec![posting("Assets:A", 40, 50)]),
                    (Flag::BalancePad, vec![posting("Assets:A", 50, 100), posting("Equity:Open", -50, -60)]),
                    (Flag::BalanceCheck, vec![posting("Assets:A", 0, 100)]),
                ]
            );
            assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, Some("Assets:A".to_owned()))]);
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
            assert_eq!(same_day, vec!["Open", "BalanceCheck", "txn C", "BalancePad", "txn P", "txn *", "txn *"]);
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
                    if matches!(txn.flag, Some(Flag::BalancePad | Flag::BalanceCheck)) {
                        synthesized += 1;
                        assert!(matches!(previous.data, Directive::BalancePad(_) | Directive::BalanceCheck(_)));
                        assert_eq!(previous.span, directive.span);
                    }
                }
            }
            assert_eq!(synthesized, 5);
            // re-sorting the final stream changes nothing
            assert_eq!(ledger.directives.clone(), Ledger::sort_directives_datetime(ledger.directives.clone()));
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

    mod price {
        use bigdecimal::BigDecimal;
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
        use indoc::indoc;

        use crate::ledger::test::load_from_temp_str;

        #[test]
        fn should_get_price() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 commodity CNY
                    1970-01-01 commodity USD
                    1970-02-01 price USD 7 CNY
                "#});

            let mut operations = ledger.operations();

            let option = operations
                .get_price(
                    NaiveDateTime::new(NaiveDate::from_ymd_opt(1970, 2, 1).unwrap(), NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
                    "USD",
                    "CNY",
                )
                .unwrap()
                .unwrap();
            assert_eq!(BigDecimal::from(7), option.amount)
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
}
