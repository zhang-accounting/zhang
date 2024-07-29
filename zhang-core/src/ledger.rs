use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, RwLock};

use cfg_if::cfg_if;
use itertools::Itertools;
use log::{error, info};
use zhang_ast::{Directive, DirectiveType, Options, Plugin, SpanInfo, Spanned};

use crate::data_source::DataSource;
use crate::domains::Operations;
use crate::error::IoErrorIntoZhangError;
use crate::options::{BuiltinOption, InMemoryOptions};
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
    meta_directives: Vec<Spanned<Directive>>,
    dated_directives: Vec<Spanned<Directive>>,

    options_directives: Vec<(Options, SpanInfo)>,
    plugin_directives: Vec<(Plugin, SpanInfo)>,
    other_directives: Vec<Spanned<Directive>>,
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

        // merge built-in options and user-defined options
        let merged_metas = BuiltinOption::default_options(options_key)
            .into_iter()
            .chain(meta_directives)
            .rev()
            .collect_vec();
        let grouped_directives = merged_metas.iter().rev().chain(dated_directives.iter()).cloned().collect_vec();

        let mut options_directives = vec![];
        let mut plugin_directives = vec![];
        let mut other_directives = Vec::with_capacity(grouped_directives.len());

        // extract plugins first before handling other directives
        for directive in grouped_directives.into_iter() {
            match directive.data {
                Directive::Plugin(plugin) => plugin_directives.push((plugin, directive.span)),
                Directive::Option(option) => options_directives.push((option, directive.span)),
                _ => other_directives.push(directive),
            }
        }

        Self {
            meta_directives: merged_metas,
            dated_directives,
            options_directives,
            plugin_directives,
            other_directives,
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

    pub fn process(context: LedgerProcessContext) -> ZhangResult<Ledger> {
        let mut ret_ledger = Self {
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
        let SplitDirectives {
            meta_directives,
            dated_directives,
            mut options_directives,
            mut plugin_directives,
            other_directives,
        } = SplitDirectives::new(context.directives);

        ret_ledger.handle_options(&mut options_directives)?;

        ret_ledger.handle_plugins_pre_process(&mut plugin_directives)?;
        ret_ledger.handle_plugins(&mut plugin_directives)?;

        let other_directives = ret_ledger.handle_plugin_execution(other_directives)?;

        ret_ledger.handle_other_directives(other_directives)?;

        ret_ledger.metas = meta_directives;
        ret_ledger.directives = dated_directives;
        let mut operations = ret_ledger.operations();
        let errors = operations.errors()?;
        if !errors.is_empty() {
            error!("Ledger loaded with {} error", errors.len());
        } else {
            info!("Ledger loaded");
        }
        Ok(ret_ledger)
    }

    async fn async_process(context: LedgerProcessContext) -> ZhangResult<Ledger> {
        let mut ret_ledger = Self {
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
        let SplitDirectives {
            meta_directives,
            dated_directives,
            mut options_directives,
            mut plugin_directives,
            other_directives,
        } = SplitDirectives::new(context.directives);
        ret_ledger.handle_options(&mut options_directives)?;
        ret_ledger.async_handle_plugins_pre_process(&mut plugin_directives).await?;
        ret_ledger.handle_plugins(&mut plugin_directives)?;
        let other_directives = ret_ledger.handle_plugin_execution(other_directives)?;
        ret_ledger.handle_other_directives(other_directives)?;

        ret_ledger.metas = meta_directives;
        ret_ledger.directives = dated_directives;
        let mut operations = ret_ledger.operations();
        let errors = operations.errors()?;
        if !errors.is_empty() {
            error!("Ledger loaded with {} error", errors.len());
        } else {
            info!("Ledger loaded");
        }
        Ok(ret_ledger)
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
    fn sort_directives_datetime(mut directives: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        directives.sort_by(|a, b| match (a.datetime(), b.datetime()) {
            (Some(a_datetime), Some(b_datetime)) => match a_datetime.cmp(&b_datetime) {
                Ordering::Equal => match (a.directive_type(), b.directive_type()) {
                    (DirectiveType::BalancePad | DirectiveType::BalanceCheck, DirectiveType::BalancePad | DirectiveType::BalanceCheck) => Ordering::Equal,
                    (DirectiveType::Open, DirectiveType::BalancePad | DirectiveType::BalanceCheck) => Ordering::Less,
                    (DirectiveType::BalancePad | DirectiveType::BalanceCheck, DirectiveType::Open) => Ordering::Greater,
                    (DirectiveType::BalancePad | DirectiveType::BalanceCheck, _) => Ordering::Less,
                    (_, DirectiveType::BalancePad | DirectiveType::BalanceCheck) => Ordering::Greater,
                    (_, _) => Ordering::Equal,
                },
                other => other,
            },
            _ => Ordering::Greater,
        });
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

    fn handle_other_directives(&mut self, mut other_directives: Vec<Spanned<Directive>>) -> Result<(), ZhangError> {
        // handle other directives
        for directive in other_directives.iter_mut() {
            match &mut directive.data {
                Directive::Option(_) => unreachable!("option directive should not be passed into the processor here"),
                Directive::Open(open) => open.handler(self, &directive.span)?,
                Directive::Close(close) => close.handler(self, &directive.span)?,
                Directive::Commodity(commodity) => commodity.handler(self, &directive.span)?,
                Directive::Transaction(trx) => trx.handler(self, &directive.span)?,
                Directive::BalancePad(pad) => pad.handler(self, &directive.span)?,
                Directive::BalanceCheck(check) => check.handler(self, &directive.span)?,
                Directive::Note(_) => {}
                Directive::Document(document) => document.handler(self, &directive.span)?,
                Directive::Price(price) => price.handler(self, &directive.span)?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                Directive::Plugin(_) => unreachable!("plugin directive should not be passed into the processor here"),
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

    fn handle_plugin_execution(&mut self, other_directives: Vec<Spanned<Directive>>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let other_directives = Ledger::sort_directives_datetime(other_directives);
        let d = if self.options.features.plugins {
            cfg_if! {
                if #[cfg(feature = "plugin_runtime")] {
                    let mut directives = other_directives;
                    let options = self.operations().options()?;
                    // execute the plugins of processor type
                    for plugin in self.plugins.processors.iter() {
                        directives = plugin.execute_as_processor(directives, &options)?;
                    }
                    directives = Ledger::sort_directives_datetime(directives);

                    // execute the plugins of mapper type
                    for plugin in self.plugins.mappers.iter() {
                        let plugin_ret: ZhangResult<Vec<Vec<Spanned<Directive>>>> =
                            directives.into_iter().map(|d| plugin.execute_as_mapper(d, &options)).collect();
                        directives = plugin_ret?.into_iter().flatten().collect_vec();
                    }
                    Ledger::sort_directives_datetime(directives)

                }else {
                    other_directives
                }
            }
        } else {
            other_directives
        };
        Ok(d)
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
        fn should_keep_original_order_given_none_datetime_and_datetime() {
            let original = test_parse_zhang(indoc! {r#"
                1970-01-01 open Assets:Hello
                option "description" "Description"
            "#});
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
                    option "description" "Description"
                "#}),
                sorted
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
        fn should_sorted_between_none_datatime() {
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
                    1970-02-01 open Assets:Hello
                    1970-03-01 open Assets:Hello
                    option "2" "2"
                    1970-01-01 open Assets:Hello
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
