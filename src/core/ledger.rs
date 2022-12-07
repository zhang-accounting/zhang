use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::option::Option::None;
use std::path::PathBuf;

use itertools::Itertools;
use log::{debug, error, info};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{ConnectOptions, SqliteConnection};

use crate::core::amount::Amount;
use crate::core::data::{Include, Transaction};
use crate::core::database::migrations::Migration;
use crate::core::models::{Directive, DirectiveType, ZhangString};
use crate::core::options::Options;
use crate::core::process::{DirectiveProcess, ProcessContext};
use crate::core::utils::span::{SpanInfo, Spanned};
use crate::error::{IoErrorIntoZhangError, ZhangError, ZhangResult};
use crate::parse_zhang;
use crate::server::route::create_folder_if_not_exist;
use crate::target::ZhangTarget;

#[derive(Clone, Debug)]
pub struct LedgerError {
    pub(crate) span: SpanInfo,
    pub(crate) error: LedgerErrorType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerErrorType {
    AccountBalanceCheckError {
        account_name: String,
        target: Amount,
        current: Amount,
        distance: Amount,
    },
    AccountDoesNotExist {
        account_name: String,
    },
    AccountClosed {
        account_name: String,
    },
    TransactionDoesNotBalance,
    CommodityDoesNotDefine {
        commodity_name: String,
    },
    TransactionHasMultipleImplicitPosting,
}

impl LedgerErrorType {
    pub fn message(&self) -> String {
        match self {
            LedgerErrorType::AccountBalanceCheckError {
                account_name,
                distance,
                target,
                current,
            } => format!(
                "account {} balance to {} {} with distance {} {}(current is {} {})",
                account_name,
                &target.number,
                &target.currency,
                &distance.number,
                &distance.currency,
                &current.number,
                &current.currency,
            ),
            LedgerErrorType::AccountDoesNotExist { account_name } => format!("account {} does not exist", account_name),
            LedgerErrorType::AccountClosed { account_name } => format!("account {} had been closed", account_name),
            LedgerErrorType::TransactionDoesNotBalance => "transaction does not balanced".to_string(),
            LedgerErrorType::CommodityDoesNotDefine { commodity_name } => {
                format!("commodity {commodity_name} does not define")
            }
            LedgerErrorType::TransactionHasMultipleImplicitPosting => {
                "transaction has more than one implicit posting".to_string()
            }
        }
    }
}

#[derive(Debug)]
pub struct Ledger {
    pub entry: (PathBuf, String),
    pub database: PathBuf,
    pub(crate) visited_files: Vec<PathBuf>,

    pub options: Options,
    pub errors: Vec<LedgerError>,
    pub configs: HashMap<String, String>,

    pub(crate) directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,
}

impl Ledger {
    pub async fn load(entry: PathBuf, endpoint: String) -> ZhangResult<Ledger> {
        let database = tempfile::tempdir().unwrap().into_path().join("data.db");
        Ledger::load_with_database(entry, endpoint, database).await
    }

    pub async fn load_with_database(entry: PathBuf, endpoint: String, database: PathBuf) -> ZhangResult<Ledger> {
        info!("cache store at {}", database.display());
        let entry = entry.canonicalize().with_path(&entry)?;
        let main_endpoint = entry.join(&endpoint);
        let main_endpoint = main_endpoint.canonicalize().with_path(&main_endpoint)?;
        let mut load_queue = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited = HashSet::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
            let path = load_entity.canonicalize().with_path(&load_entity)?;
            debug!("visited entry file: {}", path.to_str().unwrap());
            if visited.contains(&path) {
                continue;
            }
            let entity_directives = Ledger::load_directive_from_file(load_entity)?;
            entity_directives
                .iter()
                .filter(|it| it.directive_type() == DirectiveType::Include)
                .for_each(|it| match &it.data {
                    Directive::Include(include_directive) => {
                        let buf = PathBuf::from(include_directive.file.clone().to_plain_string());
                        let include_path = path.parent().map(|it| it.join(&buf)).unwrap_or(buf);
                        load_queue.push_back(include_path)
                    }
                    _ => {
                        unreachable!()
                    }
                });
            visited.insert(path);
            directives.extend(entity_directives)
        }
        Ledger::process(
            directives,
            (entry, endpoint),
            database,
            visited.into_iter().collect_vec(),
        )
        .await
    }

    fn load_directive_from_file(entry: PathBuf) -> ZhangResult<Vec<Spanned<Directive>>> {
        let content = std::fs::read_to_string(&entry).with_path(&entry)?;
        parse_zhang(&content, entry).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    pub(crate) async fn connection(&self) -> SqliteConnection {
        SqliteConnectOptions::default()
            .filename(&self.database)
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(false)
            .connect()
            .await
            .unwrap()
    }

    async fn process(
        directives: Vec<Spanned<Directive>>, entry: (PathBuf, String), database: PathBuf, visited_files: Vec<PathBuf>,
    ) -> ZhangResult<Ledger> {
        let mut connect = SqliteConnectOptions::default()
            .filename(&database)
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();

        Migration::init_database_if_missing(&mut connect).await?;

        let (mut meta_directives, dated_directive): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) =
            directives.into_iter().partition(|it| it.datetime().is_none());
        let mut directives = Ledger::sort_directives_datetime(dated_directive);

        let mut ret_ledger = Self {
            options: Options::default(),
            entry,
            database,
            visited_files,
            directives: vec![],
            metas: vec![],

            errors: vec![],
            configs: HashMap::default(),
        };
        let mut context = ProcessContext { connection: connect };
        let _operating_currency = ret_ledger.options.operating_currency.to_string();
        let _default_precision = ret_ledger.options.default_balance_tolerance_precision;
        let _default_rounding = ret_ledger.options.default_rounding;
        for directive in meta_directives.iter_mut().chain(directives.iter_mut()) {
            match &mut directive.data {
                Directive::Option(option) => option.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Open(open) => open.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Close(close) => close.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Commodity(commodity) => {
                    commodity
                        .process(&mut ret_ledger, &mut context, &directive.span)
                        .await?
                }
                Directive::Transaction(trx) => trx.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Balance(balance) => balance.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Note(_) => {}
                Directive::Document(document) => {
                    document.process(&mut ret_ledger, &mut context, &directive.span).await?
                }
                Directive::Price(price) => price.process(&mut ret_ledger, &mut context, &directive.span).await?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                _ => {}
            }
        }

        ret_ledger.metas = meta_directives;
        ret_ledger.directives = directives;
        if !ret_ledger.errors.is_empty() {
            error!("Ledger loaded with {} error", ret_ledger.errors.len());
        } else {
            info!("Ledger loaded");
        }
        Ok(ret_ledger)
    }

    pub async fn load_from_str(content: impl AsRef<str>) -> ZhangResult<Ledger> {
        let result = tempfile::tempdir().unwrap();
        let endpoint_path = result.path().join("main.zhang");
        std::fs::write(&endpoint_path, content.as_ref()).with_path(&endpoint_path)?;
        Ledger::load(result.into_path(), "main.zhang".to_string()).await
    }

    fn sort_directives_datetime(mut directives: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        directives.sort_by(|a, b| match (a.datetime(), b.datetime()) {
            (Some(a_datetime), Some(b_datetime)) => a_datetime.cmp(&b_datetime),
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

    pub fn option(&self, key: &str) -> Option<String> {
        self.configs.get(key).map(|it| it.to_string())
    }

    pub fn is_transaction_balanced(&self, txn: &Transaction) -> bool {
        // 1. get the txn's inventory
        match txn.get_postings_inventory() {
            Ok(inventory) => {
                for (_currency, _amount) in inventory.currencies.iter() {
                    // todo get currency info
                    // let currency_info = self.currencies.get(currency);
                    // let precision = currency_info
                    //     .and_then(|info| info.precision)
                    //     .unwrap_or(self.options.default_balance_tolerance_precision);
                    // let rounding = currency_info
                    //     .and_then(|info| info.rounding)
                    //     .unwrap_or(self.options.default_rounding);
                    // let decimal = amount.total.round_with(precision as i64, rounding.is_up());
                    // if !decimal.is_zero() {
                    //     return false;
                    // }
                }
                true
            }
            Err(_) => false,
        }
    }

    pub async fn reload(&mut self) -> ZhangResult<()> {
        let (entry, endpoint) = &mut self.entry;
        let reload_ledger = Ledger::load(entry.clone(), endpoint.clone()).await?;
        *self = reload_ledger;
        Ok(())
    }

    pub(crate) fn append_directives(&self, directives: Vec<Directive>, target_endpoint: impl Into<Option<String>>) {
        let (entry, endpoint) = &self.entry;
        let endpoint = entry.join(target_endpoint.into().unwrap_or_else(|| endpoint.clone()));

        create_folder_if_not_exist(&endpoint);

        if !self.visited_files.contains(&endpoint) {
            let path = match endpoint.strip_prefix(entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => endpoint.to_str().unwrap(),
            };
            self.append_directives(
                vec![Directive::Include(Include {
                    file: ZhangString::QuoteString(path.to_string()),
                })],
                None,
            );
        }
        let directive_content = format!("\n{}\n", directives.into_iter().map(|it| it.to_target()).join("\n"));
        let mut ledger_base_file = OpenOptions::new().append(true).create(true).open(&endpoint).unwrap();
        ledger_base_file.write_all(directive_content.as_bytes()).unwrap();
    }
}
//
// #[cfg(test)]
// mod test {
//     use crate::core::models::Directive;
//     use crate::core::utils::span::Spanned;
//     use crate::parse_zhang;
//     use std::option::Option::None;
//
//     fn test_parse_zhang(content: &str) -> Vec<Spanned<Directive>> {
//         parse_zhang(content, None).expect("cannot parse zhang")
//     }
//
//     mod sort_directive_datetime {
//         use crate::core::ledger::test::test_parse_zhang;
//         use crate::core::ledger::Ledger;
//         use indoc::indoc;
//         use itertools::Itertools;
//
//         #[test]
//         fn should_keep_order_given_two_none_datetime() {
//             let original = test_parse_zhang(indoc! {r#"
//                 option "title" "Title"
//                 option "description" "Description"
//             "#});
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     option "title" "Title"
//                     option "description" "Description"
//                 "#}),
//                 sorted
//             )
//         }
//
//         #[test]
//         fn should_keep_original_order_given_none_datetime_and_datetime() {
//             let original = test_parse_zhang(indoc! {r#"
//                 1970-01-01 open Assets:Hello
//                 option "description" "Description"
//             "#});
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     option "description" "Description"
//                 "#}),
//                 sorted
//             );
//             let original = test_parse_zhang(indoc! {r#"
//                     option "description" "Description"
//                     1970-01-01 open Assets:Hello
//                 "#});
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     option "description" "Description"
//                     1970-01-01 open Assets:Hello
//                 "#}),
//                 sorted
//             )
//         }
//
//         #[test]
//         fn should_order_by_datetime() {
//             let original = test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     1970-02-01 open Assets:Hello
//                 "#});
//
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     1970-02-01 open Assets:Hello
//                 "#})
//                 .into_iter()
//                 .map(|it| it.data)
//                 .collect_vec(),
//                 sorted.into_iter().map(|it| it.data).collect_vec()
//             );
//             let original = test_parse_zhang(indoc! {r#"
//                     1970-02-01 open Assets:Hello
//                     1970-01-01 open Assets:Hello
//                 "#});
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     1970-02-01 open Assets:Hello
//                 "#})
//                 .into_iter()
//                 .map(|it| it.data)
//                 .collect_vec(),
//                 sorted.into_iter().map(|it| it.data).collect_vec()
//             )
//         }
//
//         #[test]
//         fn should_sorted_between_none_datatime() {
//             let original = test_parse_zhang(indoc! {r#"
//                     option "1" "1"
//                     1970-03-01 open Assets:Hello
//                     1970-02-01 open Assets:Hello
//                     option "2" "2"
//                     1970-01-01 open Assets:Hello
//                 "#});
//
//             let sorted = Ledger::sort_directives_datetime(original);
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     option "1" "1"
//                     1970-02-01 open Assets:Hello
//                     1970-03-01 open Assets:Hello
//                     option "2" "2"
//                     1970-01-01 open Assets:Hello
//                 "#})
//                 .into_iter()
//                 .map(|it| it.data)
//                 .collect_vec(),
//                 sorted.into_iter().map(|it| it.data).collect_vec()
//             );
//         }
//
//         #[test]
//         fn should_keep_order_given_same_datetime() {
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     1970-01-01 close Assets:Hello
//                 "#}),
//                 Ledger::sort_directives_datetime(test_parse_zhang(indoc! {r#"
//                     1970-01-01 open Assets:Hello
//                     1970-01-01 close Assets:Hello
//                 "#}))
//             );
//         }
//     }
//
//     mod extract_info {
//         use crate::core::ledger::{Ledger};
//         use indoc::indoc;
//
//         #[tokio::test]
//         async fn should_extract_account_open() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:Hello CNY
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(1, ledger.accounts.len());
//             let account_info = ledger.accounts.get("Assets:Hello").unwrap();
//             assert_eq!(AccountStatus::Open, account_info.status);
//             assert_eq!(1, account_info.currencies.len());
//             assert!(account_info.currencies.contains("CNY"));
//         }
//
//         #[tokio::test]
//         async fn should_extract_account_close() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 close Assets:Hello
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(1, ledger.accounts.len());
//             let account_info = ledger.accounts.get("Assets:Hello").unwrap();
//             assert_eq!(AccountStatus::Close, account_info.status);
//             assert_eq!(0, account_info.currencies.len());
//         }
//
//         #[tokio::test]
//         async fn should_mark_as_close_after_opening_account() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:Hello CNY
//                 1970-02-01 close Assets:Hello
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(1, ledger.accounts.len());
//             let account_info = ledger.accounts.get("Assets:Hello").unwrap();
//             assert_eq!(AccountStatus::Close, account_info.status);
//             assert_eq!(1, account_info.currencies.len());
//             assert!(account_info.currencies.contains("CNY"));
//         }
//
//         #[tokio::test]
//         async fn should_extract_commodities() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 commodity CNY
//                 1970-02-01 commodity HKD
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(2, ledger.currencies.len());
//             assert!(ledger.currencies.contains_key("CNY"));
//             assert!(ledger.currencies.contains_key("HKD"));
//         }
//     }
//
//     mod multiple_file {
//         use crate::core::ledger::test::test_parse_zhang;
//         use crate::core::ledger::Ledger;
//         use indoc::indoc;
//         use itertools::Itertools;
//         use tempfile::tempdir;
//
//         #[tokio::test]
//         async fn should_load_file_from_include_directive() {
//             let temp_dir = tempdir().unwrap().into_path();
//             let example = temp_dir.join("example.zhang");
//             std::fs::write(
//                 &example,
//                 indoc! {r#"
//                 option "title" "Example"
//                 include "include.zhang"
//             "#},
//             )
//             .unwrap();
//             let include = temp_dir.join("include.zhang");
//             std::fs::write(
//                 &include,
//                 indoc! {r#"
//                     option "description" "Example Description"
//                 "#},
//             )
//             .unwrap();
//             let ledger = Ledger::load(temp_dir, "example.zhang".to_string()).await.unwrap();
//             assert_eq!(
//                 test_parse_zhang(indoc! {r#"
//                 option "title" "Example"
//                 include "include.zhang"
//                 option "description" "Example Description"
//             "#})
//                 .into_iter()
//                 .map(|it| it.data)
//                 .collect_vec(),
//                 ledger.metas.into_iter().map(|it| it.data).collect_vec()
//             );
//             assert_eq!(0, ledger.directives.len());
//         }
//     }
//
//     mod txn {
//         use crate::core::ledger::Ledger;
//         use bigdecimal::BigDecimal;
//         use indoc::indoc;
//
//         #[tokio::test]
//         async fn should_record_amount_into_inventory() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY
//
//                 2022-02-22 "Payee"
//                   Assets:From -10 CNY
//                   Expenses:To 10 CNY
//             "#})
//             .await
//             .unwrap();
//
//             assert_eq!(2, ledger.account_inventory.len());
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//         }
//
//         #[tokio::test]
//         async fn should_record_amount_into_inventory_given_none_unit_posting_and_single_unit_posting() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY
//
//                 2022-02-22 "Payee"
//                   Assets:From -10 CNY
//                   Expenses:To
//             "#})
//             .await
//             .unwrap();
//
//             assert_eq!(2, ledger.account_inventory.len());
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//         }
//
//         #[tokio::test]
//         async fn should_record_amount_into_inventory_given_none_unit_posting_and_more_unit_postings() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY
//
//                 2022-02-22 "Payee"
//                   Assets:From -5 CNY
//                   Assets:From -5 CNY
//                   Expenses:To
//             "#})
//             .await
//             .unwrap();
//
//             assert_eq!(2, ledger.account_inventory.len());
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//         }
//
//         #[tokio::test]
//         async fn should_record_amount_into_inventory_given_unit_postings_and_total_cost() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY
//
//                 2022-02-22 "Payee"
//                   Assets:From -5 CNY
//                   Assets:From -5 CNY
//                   Expenses:To 1 BTC @@ 10 CNY
//             "#})
//             .await
//             .unwrap();
//
//             assert_eq!(2, ledger.account_inventory.len());
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(1i32),
//                 &ledger
//                     .account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("BTC")
//                     .unwrap()
//                     .total
//             );
//         }
//
//         #[tokio::test]
//         async fn should_record_amount_into_inventory_given_unit_postings_and_single_cost() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY2
//
//                 2022-02-22 "Payee"
//                   Assets:From -5 CNY
//                   Assets:From -5 CNY
//                   Expenses:To 10 CNY2 @ 1 CNY
//             "#})
//             .await
//             .unwrap();
//
//             assert_eq!(2, ledger.account_inventory.len());
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(10i32),
//                 &ledger
//                     .account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("CNY2")
//                     .unwrap()
//                     .total
//             );
//         }
//     }
//
//     mod daily_inventory {
//         use crate::core::ledger::Ledger;
//         use bigdecimal::BigDecimal;
//         use chrono::NaiveDate;
//         use indoc::indoc;
//         use std::collections::HashMap;
//         use std::sync::Arc;
//
//         #[tokio::test]
//         async fn should_record_daily_inventory() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 1970-01-01 open Assets:From CNY
//                 1970-01-01 open Expenses:To CNY
//
//                 2022-02-22 "Payee"
//                   Assets:From -10 CNY
//                   Expenses:To
//             "#})
//             .await
//             .unwrap();
//
//             let account_inventory = ledger
//                 .daily_inventory
//                 .get_account_inventory(&NaiveDate::from_ymd(2022, 2, 22));
//             assert_eq!(
//                 &BigDecimal::from(-10i32),
//                 &account_inventory
//                     .get("Assets:From")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//             assert_eq!(
//                 &BigDecimal::from(10i32),
//                 &account_inventory
//                     .get("Expenses:To")
//                     .unwrap()
//                     .currencies
//                     .get("CNY")
//                     .unwrap()
//                     .total
//             );
//         }
//
//         #[test]
//         fn should_get_from_previous_day_given_day_is_not_in_data() {
//             let mut daily_inventory = DailyAccountInventory::default();
//             let mut map = HashMap::default();
//             map.insert(
//                 "AAAAA".to_string(),
//                 Inventory {
//                     currencies: Default::default(),
//                     prices: Arc::new(Default::default()),
//                 },
//             );
//             daily_inventory.insert_account_inventory(NaiveDate::from_ymd(2022, 2, 22), map);
//
//             let target_day_inventory = daily_inventory.get_account_inventory(&NaiveDate::from_ymd(2022, 3, 22));
//             assert_eq!(1, target_day_inventory.len());
//             assert!(target_day_inventory.contains_key("AAAAA"));
//         }
//     }
//
//     mod option {
//         use crate::core::ledger::Ledger;
//         use indoc::indoc;
//
//         #[tokio::test]
//         async fn should_read_to_option() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 option "title" "Example accounting book"
//                 option "operating_currency" "CNY"
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(ledger.option("title").unwrap(), "Example accounting book");
//             assert_eq!(ledger.option("operating_currency").unwrap(), "CNY");
//         }
//
//         #[tokio::test]
//         async fn should_store_the_latest_one_given_same_name_option() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 option "title" "Example accounting book"
//                 option "title" "Example accounting book 2"
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(ledger.option("title").unwrap(), "Example accounting book 2");
//         }
//     }
//
//     mod default_behavior {
//         use crate::core::ledger::Ledger;
//         use indoc::indoc;
//
//         #[tokio::test]
//         async fn should_generate_default_commodity_for_operating_commodity() {
//             let ledger = Ledger::load_from_str(indoc! {r#"
//                 option "operating_currency" "CNY"
//             "#})
//             .await
//             .unwrap();
//             assert_eq!(ledger.options.operating_currency, "CNY");
//             assert!(ledger.currencies.contains_key("CNY"));
//         }
//     }
// }
