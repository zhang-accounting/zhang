use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, RwLock};

use bigdecimal::Zero;
use itertools::Itertools;
use log::{error, info};
use zhang_ast::{Directive, DirectiveType, Spanned, Transaction};

use crate::domains::Operations;
use crate::error::IoErrorIntoZhangError;
use crate::options::{BuiltinOption, InMemoryOptions};
use crate::process::DirectiveProcess;
use crate::store::Store;
use crate::transform::Transformer;
use crate::utils::bigdecimal_ext::BigDecimalExt;
use crate::ZhangResult;

pub struct Ledger {
    pub entry: (PathBuf, String),

    pub visited_files: Vec<PathBuf>,

    pub options: InMemoryOptions,

    pub directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,

    pub transformer: Arc<dyn Transformer>,

    pub store: Arc<RwLock<Store>>,

    pub(crate) trx_counter: AtomicI32,
}

impl Ledger {
    pub fn load<T: Transformer + Default + 'static>(entry: PathBuf, endpoint: String) -> ZhangResult<Ledger> {
        let transformer = Arc::new(T::default());
        Ledger::load_with_database(entry, endpoint, transformer)
    }

    pub fn load_with_database(entry: PathBuf, endpoint: String, transformer: Arc<dyn Transformer>) -> ZhangResult<Ledger> {
        let entry = entry.canonicalize().with_path(&entry)?;

        let transform_result = transformer.load(entry.clone(), endpoint.clone())?;
        Ledger::process(transform_result.directives, (entry, endpoint), transform_result.visited_files, transformer)
    }

    fn process(
        directives: Vec<Spanned<Directive>>, entry: (PathBuf, String), visited_files: Vec<PathBuf>, transformer: Arc<dyn Transformer>,
    ) -> ZhangResult<Ledger> {
        let (meta_directives, dated_directive): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) =
            directives.into_iter().partition(|it| it.datetime().is_none());
        let mut directives = Ledger::sort_directives_datetime(dated_directive);
        let mut ret_ledger = Self {
            options: InMemoryOptions::default(),
            entry,
            visited_files,
            directives: vec![],
            metas: vec![],
            transformer,
            store: Default::default(),
            trx_counter: AtomicI32::new(1),
        };
        let mut merged_metas = BuiltinOption::default_options()
            .into_iter()
            .chain(meta_directives)
            .rev()
            .dedup_by(|x, y| match (&x.data, &y.data) {
                (Directive::Option(option_x), Directive::Option(option_y)) => option_x.key.eq(&option_y.key),
                _ => false,
            })
            .collect_vec();
        for directive in merged_metas.iter_mut().rev().chain(directives.iter_mut()) {
            match &mut directive.data {
                Directive::Option(option) => option.handler(&mut ret_ledger, &directive.span)?,
                Directive::Open(open) => open.handler(&mut ret_ledger, &directive.span)?,
                Directive::Close(close) => close.handler(&mut ret_ledger, &directive.span)?,
                Directive::Commodity(commodity) => commodity.handler(&mut ret_ledger, &directive.span)?,
                Directive::Transaction(trx) => trx.handler(&mut ret_ledger, &directive.span)?,
                Directive::BalancePad(pad) => pad.handler(&mut ret_ledger, &directive.span)?,
                Directive::BalanceCheck(check) => check.handler(&mut ret_ledger, &directive.span)?,
                Directive::Note(_) => {}
                Directive::Document(document) => document.handler(&mut ret_ledger, &directive.span)?,
                Directive::Price(price) => price.handler(&mut ret_ledger, &directive.span)?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                Directive::Plugin(_) => {}
                Directive::Include(_) => {}
                Directive::Comment(_) => {}
                Directive::Budget(budget) => budget.handler(&mut ret_ledger, &directive.span)?,
                Directive::BudgetAdd(budget_add) => budget_add.handler(&mut ret_ledger, &directive.span)?,
                Directive::BudgetTransfer(budget_transfer) => budget_transfer.handler(&mut ret_ledger, &directive.span)?,
                Directive::BudgetClose(budget_close) => budget_close.handler(&mut ret_ledger, &directive.span)?,
            }
        }

        ret_ledger.metas = merged_metas;
        ret_ledger.directives = directives;
        let mut operations = ret_ledger.operations();
        let errors = operations.errors()?;
        if !errors.is_empty() {
            error!("Ledger loaded with {} error", errors.len());
        } else {
            info!("Ledger loaded");
        }
        Ok(ret_ledger)
    }

    fn sort_directives_datetime(mut directives: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        directives.sort_by(|a, b| match (a.datetime(), b.datetime()) {
            (Some(a_datetime), Some(b_datetime)) => match a_datetime.cmp(&b_datetime) {
                Ordering::Equal => match (a.directive_type(), b.directive_type()) {
                    (DirectiveType::BalancePad | DirectiveType::BalanceCheck, DirectiveType::BalancePad | DirectiveType::BalanceCheck) => Ordering::Equal,
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

    pub fn apply(mut self, applier: impl Fn(Directive) -> Directive) -> Self {
        let vec = self
            .directives
            .into_iter()
            .map(|mut it| {
                let directive = applier(it.data);
                it.data = directive;
                it
            })
            .collect_vec();
        self.directives = vec;
        self
    }

    pub fn is_transaction_balanced(&self, txn: &Transaction) -> ZhangResult<bool> {
        // 1. get the txn's inventory
        Ok(match txn.get_postings_inventory() {
            Ok(inventory) => {
                for (currency, amount) in inventory.currencies.iter() {
                    let mut operations = self.operations();
                    let commodity = operations.commodity(currency)?;
                    let precision = commodity
                        .as_ref()
                        .map(|it| it.precision)
                        .unwrap_or(self.options.default_balance_tolerance_precision);
                    let rounding = commodity
                        .and_then(|it| it.rounding)
                        .map(|s| s.eq("RoundUp"))
                        .unwrap_or_else(|| self.options.default_rounding.is_up());
                    let decimal = amount.total.round_with(precision as i64, rounding);
                    if !decimal.is_zero() {
                        return Ok(false);
                    }
                }
                true
            }
            Err(_) => false,
        })
    }

    pub fn reload(&mut self) -> ZhangResult<()> {
        let (entry, endpoint) = &mut self.entry;
        let transform_result = self.transformer.load(entry.clone(), endpoint.clone())?;
        let reload_ledger = Ledger::process(
            transform_result.directives,
            (entry.clone(), endpoint.clone()),
            transform_result.visited_files,
            self.transformer.clone(),
        )?;
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

#[cfg(test)]
mod test {
    use std::option::Option::None;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tempfile::tempdir;
    use zhang_ast::{Directive, SpanInfo, Spanned};

    use crate::ledger::Ledger;
    use crate::text::parser::parse as parse_zhang;
    use crate::transform::{TransformResult, Transformer};
    use crate::ZhangResult;

    fn fake_span_info() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 0,
            content: "".to_string(),
            filename: None,
        }
    }

    fn test_parse_zhang(content: &str) -> Vec<Spanned<Directive>> {
        parse_zhang(content, None).expect("cannot parse zhang")
    }
    struct TestTransformer {}

    impl Transformer for TestTransformer {
        fn load(&self, _entry: PathBuf, _endpoint: String) -> ZhangResult<TransformResult> {
            todo!()
        }

        fn get_content(&self, path: String) -> ZhangResult<Vec<u8>> {
            todo!()
        }

        fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
            todo!()
        }

        fn save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
            todo!()
        }
    }
    fn load_from_temp_str(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        Ledger::process(
            test_parse_zhang(content),
            (temp_dir.clone(), "example.zhang".to_string()),
            vec![temp_dir.join("example.zhang")],
            Arc::new(TestTransformer {}),
        )
        .unwrap()
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
                    1970-01-01 open Assets:Hello
                "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
                    1970-01-01 open Assets:Hello
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
            let mut operations = ledger.operations();

            assert_eq!("Example Beancount file", operations.option("title")?.unwrap().value);
            assert_eq!("USD", operations.option("operating_currency")?.unwrap().value);
            assert!(operations.option("operating_currency2")?.is_none());
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
