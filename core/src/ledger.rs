use std::cmp::Ordering;
use std::option::Option::None;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::Zero;
use glob::Pattern;
use itertools::Itertools;
use log::{error, info};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool};

use zhang_ast::{Directive, DirectiveType, Spanned, Transaction};

use crate::database::migrations::Migration;
use crate::domains::Operations;
use crate::error::IoErrorIntoZhangError;
use crate::options::{default_options, InMemoryOptions};
use crate::process::DirectiveProcess;
use crate::transform::Transformer;
use crate::utils::bigdecimal_ext::BigDecimalExt;
use crate::ZhangResult;

pub struct Ledger {
    pub entry: (PathBuf, String),
    pub database: Option<PathBuf>,
    pub pool_connection: SqlitePool,
    pub visited_files: Vec<Pattern>,

    pub options: InMemoryOptions,

    pub directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,

    transformer: Arc<dyn Transformer>,
}

impl Ledger {
    pub async fn load<T: Transformer + Default + 'static>(entry: PathBuf, endpoint: String) -> ZhangResult<Ledger> {
        let transformer = Arc::new(T::default());
        Ledger::load_with_database(entry, endpoint, None, transformer).await
    }

    pub async fn load_with_database(entry: PathBuf, endpoint: String, database: Option<PathBuf>, transformer: Arc<dyn Transformer>) -> ZhangResult<Ledger> {
        let entry = entry.canonicalize().with_path(&entry)?;

        let transform_result = transformer.load(entry.clone(), endpoint.clone())?;
        Ledger::process(
            transform_result.directives,
            (entry, endpoint),
            database,
            transform_result.visited_files,
            transformer,
        )
        .await
    }

    pub async fn connection(&self) -> PoolConnection<Sqlite> {
        self.pool_connection.acquire().await.unwrap()
    }

    async fn process(
        directives: Vec<Spanned<Directive>>, entry: (PathBuf, String), database: Option<PathBuf>, visited_files: Vec<Pattern>,
        transformer: Arc<dyn Transformer>,
    ) -> ZhangResult<Ledger> {
        let sqlite_pool = if let Some(ref path) = database {
            info!("database store at {}", path.display());
            SqlitePool::connect_with(
                SqliteConnectOptions::default()
                    .filename(path)
                    .journal_mode(SqliteJournalMode::Wal)
                    .create_if_missing(true),
            )
            .await?
        } else {
            info!("using in memory database");
            SqlitePoolOptions::new()
                .max_lifetime(None)
                .idle_timeout(None)
                .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").unwrap().journal_mode(SqliteJournalMode::Wal))
                .await?
        };
        let mut connection = sqlite_pool.acquire().await?;

        Migration::init_database_if_missing(&mut connection).await?;

        let (meta_directives, dated_directive): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) =
            directives.into_iter().partition(|it| it.datetime().is_none());
        let mut directives = Ledger::sort_directives_datetime(dated_directive);
        let mut ret_ledger = Self {
            options: InMemoryOptions::default(),
            entry,
            database,
            pool_connection: sqlite_pool,
            visited_files,
            directives: vec![],
            metas: vec![],
            transformer,
        };
        let mut merged_metas = default_options()
            .into_iter()
            .chain(meta_directives.into_iter())
            .rev()
            .dedup_by(|x, y| match (&x.data, &y.data) {
                (Directive::Option(option_x), Directive::Option(option_y)) => option_x.key.eq(&option_y.key),
                _ => false,
            })
            .collect_vec();
        for directive in merged_metas.iter_mut().rev().chain(directives.iter_mut()) {
            match &mut directive.data {
                Directive::Option(option) => option.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Open(open) => open.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Close(close) => close.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Commodity(commodity) => commodity.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Transaction(trx) => trx.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Balance(balance) => balance.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Note(_) => {}
                Directive::Document(document) => document.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Price(price) => price.handler(&mut ret_ledger, &directive.span).await?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                _ => {}
            }
        }

        ret_ledger.metas = merged_metas;
        ret_ledger.directives = directives;
        let mut operations = ret_ledger.operations().await;
        let errors = operations.errors().await?;
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
                    (DirectiveType::Balance, DirectiveType::Balance) => Ordering::Equal,
                    (DirectiveType::Balance, _) => Ordering::Less,
                    (_, DirectiveType::Balance) => Ordering::Greater,
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

    pub async fn is_transaction_balanced(&self, txn: &Transaction) -> ZhangResult<bool> {
        // 1. get the txn's inventory
        Ok(match txn.get_postings_inventory() {
            Ok(inventory) => {
                for (currency, amount) in inventory.currencies.iter() {
                    let mut operations = self.operations().await;
                    let commodity = operations.commodity(currency).await?;
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

    pub async fn reload(&mut self) -> ZhangResult<()> {
        let (entry, endpoint) = &mut self.entry;
        let transform_result = self.transformer.load(entry.clone(), endpoint.clone())?;
        let reload_ledger = Ledger::process(
            transform_result.directives,
            (entry.clone(), endpoint.clone()),
            self.database.clone(),
            transform_result.visited_files,
            self.transformer.clone(),
        )
        .await?;
        *self = reload_ledger;
        Ok(())
    }

    pub async fn operations(&self) -> Operations {
        let pool = self.connection().await;
        Operations { pool }
    }
}

#[cfg(test)]
mod test {
    use glob::Pattern;
    use std::option::Option::None;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tempfile::tempdir;

    use text_transformer::parse as parse_zhang;
    use zhang_ast::{Directive, SpanInfo, Spanned};

    use crate::ledger::Ledger;
    use crate::transform::{TransformResult, Transformer};
    use crate::ZhangResult;

    macro_rules! count {
        ($reason:expr, $times: expr, $sql:expr, $conn:expr) => {
            assert_eq!($times, sqlx::query($sql).fetch_all($conn).await.unwrap().len(), $reason)
        };
        ($reason:expr, $sql:expr, $conn:expr) => {
            assert_eq!(1, sqlx::query($sql).fetch_all($conn).await.unwrap().len(), $reason)
        };
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
        parse_zhang(content, None).expect("cannot parse zhang")
    }
    struct TestTransformer {}

    impl Transformer for TestTransformer {
        fn load(&self, _entry: PathBuf, _endpoint: String) -> ZhangResult<TransformResult> {
            todo!()
        }
    }
    async fn load_from_temp_str(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(&example, content).unwrap();
        Ledger::process(
            test_parse_zhang(content),
            (temp_dir.clone(), "example.zhang".to_string()),
            None,
            vec![Pattern::new(temp_dir.join("example.zhang").as_path().to_str().unwrap()).unwrap()],
            Arc::new(TestTransformer {}),
        )
        .await
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
        use crate::ledger::test::load_from_temp_str;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_price() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                    option "title" "Example Beancount file"
                    option "operating_currency" "USD"
                "#})
            .await;
            let mut operations = ledger.operations().await;

            assert_eq!("Example Beancount file", operations.option("title").await?.unwrap().value);
            assert_eq!("USD", operations.option("operating_currency").await?.unwrap().value);
            assert!(operations.option("operating_currency2").await?.is_none());
            Ok(())
        }
    }

    mod extract_info {
        use indoc::indoc;

        use crate::ledger::test::load_from_temp_str;

        #[tokio::test]
        async fn should_extract_account_open() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 open Assets:Hello CNY
                "#})
            .await;
            let mut conn = ledger.connection().await;
            count!(
                "should have account record",
                "select * from accounts where name = 'Assets:Hello' and status = 'Open' ",
                &mut conn
            );
            // todo test account's commodity
        }

        #[tokio::test]
        async fn should_extract_account_close() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 open Assets:Hello CNY
                "#})
            .await;
            let mut conn = ledger.connection().await;
            count!(
                "should have account record",
                "select * from accounts where name = 'Assets:Hello' and status = 'Open' ",
                &mut conn
            );
        }

        #[tokio::test]
        async fn should_mark_as_close_after_opening_account() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 open Assets:Hello CNY
                    1970-02-01 close Assets:Hello
                "#})
            .await;
            let mut conn = ledger.connection().await;
            count!(
                "should have account record",
                "select * from accounts where name = 'Assets:Hello' and status = 'Close'",
                &mut conn
            );
            count!(
                "should not have account record",
                0,
                "select * from accounts where name = 'Assets:Hello' and status = 'Open'",
                &mut conn
            );
        }

        #[tokio::test]
        async fn should_extract_commodities() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 commodity CNY
                    1970-02-01 commodity HKD
                "#})
            .await;
            let mut conn = ledger.connection().await;
            count!("should have 2 commodity", 2, "select * from commodities", &mut conn);
            count!("should have CNY record", "select * from commodities where name = 'CNY'", &mut conn);
            count!("should have HKD record", "select * from commodities where name = 'HKD'", &mut conn);
        }
    }

    mod price {
        use crate::ledger::test::load_from_temp_str;
        use bigdecimal::BigDecimal;
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_price() {
            let ledger = load_from_temp_str(indoc! {r#"
                    1970-01-01 commodity CNY
                    1970-01-01 commodity USD
                    1970-02-01 price USD 7 CNY
                "#})
            .await;

            let mut operations = ledger.operations().await;

            let option = operations
                .get_price(
                    NaiveDateTime::new(NaiveDate::from_ymd_opt(1970, 2, 1).unwrap(), NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
                    "USD",
                    "CNY",
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(BigDecimal::from(7), option.amount.0)
        }
    }

    mod commodity {
        use crate::ledger::test::load_from_temp_str;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 commodity CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(2, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[tokio::test]
        async fn should_not_get_non_exist_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 commodity CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("USD").await?;
            assert!(commodity.is_none());
            Ok(())
        }
    }

    mod account {
        use crate::ledger::test::load_from_temp_str;
        use indoc::indoc;

        #[tokio::test]
        async fn should_return_true_given_exists_account() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_temp_str(indoc! {r#"
                1970-01-01 open Assets:Bank
            "#})
            .await;

            let mut operations = ledger.operations().await;
            assert!(operations.exist_account("Assets:Bank").await?);
            assert!(!operations.exist_account("Assets:Bank2").await?);
            Ok(())
        }
    }
    // mod txn {
    //     use bigdecimal::BigDecimal;
    //     use indoc::indoc;
    //     use crate::ledger::test::load_from_temp_str;
    //
    //     #[tokio::test]
    //     async fn should_record_amount_into_inventory() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 1970-01-01 open Assets:From CNY
    //                 1970-01-01 open Expenses:To CNY
    //
    //                 2022-02-22 "Payee"
    //                   Assets:From -10 CNY
    //                   Expenses:To 10 CNY
    //             "#})
    //         .await;
    //
    //         assert_eq!(2, ledger.account_inventory.len());
    //         assert_eq!(
    //             &BigDecimal::from(-10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Assets:From")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //         assert_eq!(
    //             &BigDecimal::from(10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Expenses:To")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //     }
    //
    //     #[tokio::test]
    //     async fn should_record_amount_into_inventory_given_none_unit_posting_and_single_unit_posting() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 1970-01-01 open Assets:From CNY
    //                 1970-01-01 open Expenses:To CNY
    //
    //                 2022-02-22 "Payee"
    //                   Assets:From -10 CNY
    //                   Expenses:To
    //             "#})
    //         .await;
    //
    //         assert_eq!(2, ledger.account_inventory.len());
    //         assert_eq!(
    //             &BigDecimal::from(-10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Assets:From")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //         assert_eq!(
    //             &BigDecimal::from(10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Expenses:To")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //     }
    //
    //     #[tokio::test]
    //     async fn should_record_amount_into_inventory_given_none_unit_posting_and_more_unit_postings() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 1970-01-01 open Assets:From CNY
    //                 1970-01-01 open Expenses:To CNY
    //
    //                 2022-02-22 "Payee"
    //                   Assets:From -5 CNY
    //                   Assets:From -5 CNY
    //                   Expenses:To
    //             "#})
    //         .await;
    //
    //         assert_eq!(2, ledger.account_inventory.len());
    //         assert_eq!(
    //             &BigDecimal::from(-10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Assets:From")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //         assert_eq!(
    //             &BigDecimal::from(10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Expenses:To")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //     }
    //
    //     #[tokio::test]
    //     async fn should_record_amount_into_inventory_given_unit_postings_and_total_cost() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 1970-01-01 open Assets:From CNY
    //                 1970-01-01 open Expenses:To CNY
    //
    //                 2022-02-22 "Payee"
    //                   Assets:From -5 CNY
    //                   Assets:From -5 CNY
    //                   Expenses:To 1 BTC @@ 10 CNY
    //             "#})
    //         .await;
    //
    //         assert_eq!(2, ledger.account_inventory.len());
    //         assert_eq!(
    //             &BigDecimal::from(-10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Assets:From")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //         assert_eq!(
    //             &BigDecimal::from(1i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Expenses:To")
    //                 .unwrap()
    //                 .currencies
    //                 .get("BTC")
    //                 .unwrap()
    //                 .total
    //         );
    //     }
    //
    //     #[tokio::test]
    //     async fn should_record_amount_into_inventory_given_unit_postings_and_single_cost() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 1970-01-01 open Assets:From CNY
    //                 1970-01-01 open Expenses:To CNY2
    //
    //                 2022-02-22 "Payee"
    //                   Assets:From -5 CNY
    //                   Assets:From -5 CNY
    //                   Expenses:To 10 CNY2 @ 1 CNY
    //             "#})
    //         .await;
    //
    //         assert_eq!(2, ledger.account_inventory.len());
    //         assert_eq!(
    //             &BigDecimal::from(-10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Assets:From")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY")
    //                 .unwrap()
    //                 .total
    //         );
    //         assert_eq!(
    //             &BigDecimal::from(10i32),
    //             &ledger
    //                 .account_inventory
    //                 .get("Expenses:To")
    //                 .unwrap()
    //                 .currencies
    //                 .get("CNY2")
    //                 .unwrap()
    //                 .total
    //         );
    //     }
    // }

    mod daily_inventory {

        #[tokio::test]
        async fn should_record_daily_inventory() {
            // let ledger = load_from_temp_str(indoc! {r#"
            //         1970-01-01 open Assets:From CNY
            //         1970-01-01 open Expenses:To CNY
            //
            //         2022-02-22 "Payee"
            //           Assets:From -10 CNY
            //           Expenses:To
            //     "#})
            // .await
            // .unwrap();
            //
            // let account_inventory = ledger
            //     .daily_inventory
            //     .get_account_inventory(&NaiveDate::from_ymd(2022, 2, 22));
            // assert_eq!(
            //     &BigDecimal::from(-10i32),
            //     &account_inventory
            //         .get("Assets:From")
            //         .unwrap()
            //         .currencies
            //         .get("CNY")
            //         .unwrap()
            //         .total
            // );
            // assert_eq!(
            //     &BigDecimal::from(10i32),
            //     &account_inventory
            //         .get("Expenses:To")
            //         .unwrap()
            //         .currencies
            //         .get("CNY")
            //         .unwrap()
            //         .total
            // );
        }

        #[test]
        fn should_get_from_previous_day_given_day_is_not_in_data() {
            // let mut daily_inventory = DailyAccountInventory::default();
            // let mut map = HashMap::default();
            // map.insert(
            //     "AAAAA".to_string(),
            //     Inventory {
            //         currencies: Default::default(),
            //     },
            // );
            // daily_inventory.insert_account_inventory(NaiveDate::from_ymd(2022, 2, 22), map);
            //
            // let target_day_inventory = daily_inventory.get_account_inventory(&NaiveDate::from_ymd(2022, 3, 22));
            // assert_eq!(1, target_day_inventory.len());
            // assert!(target_day_inventory.contains_key("AAAAA"));
        }
    }
    //
    // mod option {
    //     use crate::core::ledger::Ledger;
    //     use indoc::indoc;
    //
    //     #[tokio::test]
    //     async fn should_read_to_option() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 option "title" "Example accounting book"
    //                 option "operating_currency" "CNY"
    //             "#})
    //         .await
    //         .unwrap();
    //         assert_eq!(ledger.option("title").unwrap(), "Example accounting book");
    //         assert_eq!(ledger.option("operating_currency").unwrap(), "CNY");
    //     }
    //
    //     #[tokio::test]
    //     async fn should_store_the_latest_one_given_same_name_option() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 option "title" "Example accounting book"
    //                 option "title" "Example accounting book 2"
    //             "#})
    //         .await
    //         .unwrap();
    //         assert_eq!(ledger.option("title").unwrap(), "Example accounting book 2");
    //     }
    // }
    //
    // mod default_behavior {
    //     use crate::ledger::Ledger;
    //     use indoc::indoc;
    //
    //     #[tokio::test]
    //     async fn should_generate_default_commodity_for_operating_commodity() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 option "operating_currency" "CNY"
    //             "#})
    //         .await
    //         .unwrap();
    //         let mut conn = ledger.connection().await;
    //         assert_eq!(ledger.options.operating_currency, "CNY");
    //
    //         count!(
    //             "should have commodity record for operating currency",
    //             "select * from commodities where name = 'CNY'",
    //             &mut conn
    //         )
    //     }
    //
    //     // todo(test): should update commodity info given options and commodity directive
    //     #[tokio::test]
    //     async fn should_update_commodity_info_given_operating_commodity_and_commodity_directive() {
    //         let ledger = load_from_temp_str(indoc! {r#"
    //                 option "operating_currency" "CNY"
    //                 1970-01-01 commodity CNY
    //                   precision: 3
    //             "#})
    //         .await
    //         .unwrap();
    //         let mut conn = ledger.connection().await;
    //         assert_eq!(ledger.options.operating_currency, "CNY");
    //
    //         count!(
    //             "should have commodity record for operating currency",
    //             "select * from commodities where name = 'CNY'",
    //             &mut conn
    //         );
    //         count!(
    //             "should update commodity info",
    //             "select * from commodities where name = 'CNY' and precision = 3",
    //             &mut conn
    //         )
    //     }
    // }
}
