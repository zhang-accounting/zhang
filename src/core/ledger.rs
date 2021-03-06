use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{Commodity, Date, Include, Transaction};
use crate::core::models::{Directive, DirectiveType, ZhangString};
use crate::core::process::{DirectiveProcess, ProcessContext};
use crate::core::utils::inventory::{DailyAccountInventory, Inventory};
use crate::core::utils::multi_value_map::MultiValueMap;
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::utils::span::{SpanInfo, Spanned};
use crate::core::{AccountName, Currency};
use crate::error::{ZhangError, ZhangResult};
use crate::parse_zhang;
use crate::server::model::mutation::create_folder_if_not_exist;
use crate::target::ZhangTarget;
use async_graphql::Enum;
use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use itertools::{Either, Itertools};
use log::{debug, error, info};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::option::Option::None;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};

#[derive(Debug, Clone)]
pub enum DocumentType {
    AccountDocument {
        date: Date,
        account: Account,
        filename: String,
    },
    TransactionDocument {
        date: Date,
        trx: Transaction,
        filename: String,
    },
}

#[derive(Debug, PartialEq, Eq, Clone, Enum, Copy)]
pub enum AccountStatus {
    /// account is open
    Open,
    /// account is closed
    Close,
}
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub(crate) currencies: HashSet<Currency>,
    pub(crate) status: AccountStatus,
    pub(crate) meta: MultiValueMap<String, String>,
}
#[derive(Debug, Clone)]
pub struct CurrencyInfo {
    pub commodity: Commodity,
    pub prices: HashMap<NaiveDate, BigDecimal>,
}

#[derive(Clone, Debug)]
pub struct LedgerError {
    pub(crate) span: SpanInfo,
    pub(crate) error: LedgerErrorType,
}

#[derive(Clone, Debug)]
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
        }
    }
}

#[derive(Debug)]
pub struct Ledger {
    pub entry: Either<(PathBuf, String), String>,
    pub(crate) visited_files: Vec<PathBuf>,

    pub(crate) directives: Vec<Spanned<Directive>>,
    pub metas: Vec<Spanned<Directive>>,
    pub accounts: HashMap<AccountName, AccountInfo>,
    pub currencies: HashMap<Currency, CurrencyInfo>,
    pub account_inventory: HashMap<AccountName, Inventory>,
    pub daily_inventory: DailyAccountInventory,
    pub documents: Vec<DocumentType>,
    pub errors: Vec<LedgerError>,

    pub configs: HashMap<String, String>,
    pub prices: Arc<StdRwLock<DatedPriceGrip>>,
}

impl Ledger {
    pub fn load(entry: PathBuf, endpoint: String) -> ZhangResult<Ledger> {
        let main_endpoint = entry.join(&endpoint);
        let mut load_queue = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited = HashSet::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
            let path = load_entity.canonicalize()?;
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
            Either::Left((entry, endpoint)),
            visited.into_iter().collect_vec(),
        )
    }

    fn load_directive_from_file(entry: PathBuf) -> ZhangResult<Vec<Spanned<Directive>>> {
        let content = std::fs::read_to_string(&entry)?;
        parse_zhang(&content, entry).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn process(
        directives: Vec<Spanned<Directive>>, entry: Either<(PathBuf, String), String>, visited_files: Vec<PathBuf>,
    ) -> ZhangResult<Ledger> {
        let (mut meta_directives, dated_directive): (Vec<Spanned<Directive>>, Vec<Spanned<Directive>>) =
            directives.into_iter().partition(|it| it.datetime().is_none());
        let mut directives = Ledger::sort_directives_datetime(dated_directive);

        let arc_price = Arc::new(StdRwLock::new(DatedPriceGrip::default()));
        let mut ret_ledger = Self {
            entry,
            visited_files,
            directives: vec![],
            metas: vec![],
            accounts: HashMap::default(),
            currencies: HashMap::default(),
            account_inventory: HashMap::default(),
            daily_inventory: DailyAccountInventory::default(),
            documents: Vec::new(),
            errors: vec![],
            configs: HashMap::default(),
            prices: arc_price.clone(),
        };
        let mut context = ProcessContext {
            target_day: None,
            prices: arc_price,
        };
        for directive in meta_directives.iter_mut().chain(directives.iter_mut()) {
            match &mut directive.data {
                Directive::Option(option) => option.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Open(open) => open.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Close(close) => close.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Commodity(commodity) => commodity.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Transaction(trx) => trx.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Balance(balance) => balance.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Note(_) => {}
                Directive::Document(document) => document.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Price(price) => price.process(&mut ret_ledger, &mut context, &directive.span)?,
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                _ => {}
            }
        }
        if let Some(last_day) = context.target_day {
            ret_ledger
                .daily_inventory
                .insert_account_inventory(last_day, ret_ledger.account_inventory.clone());
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

    pub(crate) fn load_from_str(content: impl AsRef<str>) -> ZhangResult<Ledger> {
        let content = content.as_ref();
        let directives = parse_zhang(content, None).map_err(|it| ZhangError::PestError(it.to_string()))?;
        Ledger::process(directives, Either::Right(content.to_string()), vec![])
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

    pub fn reload(&mut self) -> ZhangResult<()> {
        let reload_ledger = match &mut self.entry {
            Either::Left((entry, endpoint)) => Ledger::load(entry.clone(), endpoint.clone()),
            Either::Right(raw_string) => Ledger::load_from_str(raw_string),
        }?;
        *self = reload_ledger;
        Ok(())
    }
    pub fn default_account_inventory(&self) -> Inventory {
        Inventory {
            inner: Default::default(),
            prices: self.prices.clone(),
        }
    }

    pub(crate) fn append_directives(&self, directives: Vec<Directive>, target_endpoint: impl Into<Option<String>>) {
        let (entry, endpoint) = match &self.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return;
            }
        };
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
        let mut directive_content = directives.into_iter().map(|it| it.to_target()).join("\n");
        directive_content.push('\n');
        let mut ledger_base_file = OpenOptions::new().append(true).create(true).open(&endpoint).unwrap();
        ledger_base_file.write_all(directive_content.as_bytes()).unwrap();
    }
}

#[cfg(test)]
mod test {
    use crate::core::models::Directive;
    use crate::core::utils::span::Spanned;
    use crate::parse_zhang;
    use std::option::Option::None;

    fn test_parse_zhang(content: &str) -> Vec<Spanned<Directive>> {
        parse_zhang(content, None).expect("cannot parse zhang")
    }

    mod sort_directive_datetime {
        use crate::core::ledger::test::test_parse_zhang;
        use crate::core::ledger::Ledger;
        use indoc::indoc;
        use itertools::Itertools;

        #[test]
        fn should_keep_order_given_two_none_datetime() {
            let original = test_parse_zhang(indoc! {r#"
                option "title" "Title"
                option "description" "Description"
            "#});
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                    option "title" "Title"
                    option "description" "Description"
                "#}),
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
    }
    mod extract_info {
        use crate::core::ledger::{AccountStatus, Ledger};
        use indoc::indoc;

        #[test]
        fn should_extract_account_open() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:Hello CNY
            "#})
            .unwrap();
            assert_eq!(1, ledger.accounts.len());
            let account_info = ledger.accounts.get("Assets:Hello").unwrap();
            assert_eq!(AccountStatus::Open, account_info.status);
            assert_eq!(1, account_info.currencies.len());
            assert!(account_info.currencies.contains("CNY"));
        }

        #[test]
        fn should_extract_account_close() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 close Assets:Hello
            "#})
            .unwrap();
            assert_eq!(1, ledger.accounts.len());
            let account_info = ledger.accounts.get("Assets:Hello").unwrap();
            assert_eq!(AccountStatus::Close, account_info.status);
            assert_eq!(0, account_info.currencies.len());
        }

        #[test]
        fn should_mark_as_close_after_opening_account() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:Hello CNY
                1970-02-01 close Assets:Hello
            "#})
            .unwrap();
            assert_eq!(1, ledger.accounts.len());
            let account_info = ledger.accounts.get("Assets:Hello").unwrap();
            assert_eq!(AccountStatus::Close, account_info.status);
            assert_eq!(1, account_info.currencies.len());
            assert!(account_info.currencies.contains("CNY"));
        }
        #[test]
        fn should_extract_commodities() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 commodity CNY
                1970-02-01 commodity HKD
            "#})
            .unwrap();
            assert_eq!(2, ledger.currencies.len());
            assert!(ledger.currencies.contains_key("CNY"));
            assert!(ledger.currencies.contains_key("HKD"));
        }
    }
    mod multiple_file {
        use crate::core::ledger::test::test_parse_zhang;
        use crate::core::ledger::Ledger;
        use indoc::indoc;
        use itertools::Itertools;
        use tempfile::tempdir;

        #[test]
        fn should_load_file_from_include_directive() {
            let temp_dir = tempdir().unwrap().into_path();
            let example = temp_dir.join("example.zhang");
            std::fs::write(
                &example,
                indoc! {r#"
                option "title" "Example"
                include "include.zhang"
            "#},
            )
            .unwrap();
            let include = temp_dir.join("include.zhang");
            std::fs::write(
                &include,
                indoc! {r#"
                    option "description" "Example Description"
                "#},
            )
            .unwrap();
            let ledger = Ledger::load(temp_dir, "example.zhang".to_string()).unwrap();
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                option "title" "Example"
                include "include.zhang"
                option "description" "Example Description"
            "#})
                .into_iter()
                .map(|it| it.data)
                .collect_vec(),
                ledger.metas.into_iter().map(|it| it.data).collect_vec()
            );
            assert_eq!(0, ledger.directives.len());
        }
    }

    mod account_inventory {
        use crate::core::amount::Amount;
        use crate::core::utils::inventory::Inventory;
        use crate::core::utils::price_grip::DatedPriceGrip;
        use bigdecimal::BigDecimal;
        use std::sync::{Arc, RwLock as StdRwLock};

        #[test]
        fn should_add_to_inner() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(StdRwLock::new(DatedPriceGrip::default())),
            };
            inventory.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            assert_eq!(1, inventory.inner.len());
            assert_eq!(&BigDecimal::from(1i32), inventory.inner.get("CNY").unwrap())
        }

        #[test]
        fn should_inventory_be_independent() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(StdRwLock::new(DatedPriceGrip::default())),
            };
            inventory.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            let new_inventory = inventory.pin();

            inventory.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            assert_eq!(1, inventory.inner.len());
            assert_eq!(&BigDecimal::from(2i32), inventory.inner.get("CNY").unwrap());

            assert_eq!(1, new_inventory.inner.len());
            assert_eq!(&BigDecimal::from(1i32), new_inventory.inner.get("CNY").unwrap())
        }
    }
    mod txn {
        use crate::core::ledger::Ledger;
        use bigdecimal::BigDecimal;
        use indoc::indoc;

        #[test]
        fn should_record_amount_into_inventory() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To 10 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.account_inventory.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .account_inventory
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .account_inventory
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_inventory_given_none_unit_posting_and_single_unit_posting() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To
            "#})
            .unwrap();

            assert_eq!(2, ledger.account_inventory.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .account_inventory
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .account_inventory
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_inventory_given_none_unit_posting_and_more_unit_postings() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To
            "#})
            .unwrap();

            assert_eq!(2, ledger.account_inventory.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .account_inventory
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .account_inventory
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_inventory_given_unit_postings_and_total_cost() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To 1 BTC @@ 10 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.account_inventory.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .account_inventory
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(1i32),
                ledger
                    .account_inventory
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("BTC")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_inventory_given_unit_postings_and_single_cost() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY2

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To 10 CNY2 @ 1 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.account_inventory.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .account_inventory
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .account_inventory
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY2")
                    .unwrap()
            );
        }
    }
    mod daily_inventory {
        use crate::core::ledger::Ledger;
        use crate::core::utils::inventory::{DailyAccountInventory, Inventory};
        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use indoc::indoc;
        use std::collections::HashMap;
        use std::sync::Arc;

        #[test]
        fn should_record_daily_inventory() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To
            "#})
            .unwrap();

            let account_inventory = ledger
                .daily_inventory
                .get_account_inventory(&NaiveDate::from_ymd(2022, 2, 22));
            assert_eq!(
                &BigDecimal::from(-10i32),
                account_inventory.get("Assets:From").unwrap().inner.get("CNY").unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                account_inventory.get("Expenses:To").unwrap().inner.get("CNY").unwrap()
            );
        }
        #[test]
        fn should_get_from_previous_day_given_day_is_not_in_data() {
            let mut daily_inventory = DailyAccountInventory::default();
            let mut map = HashMap::default();
            map.insert(
                "AAAAA".to_string(),
                Inventory {
                    inner: Default::default(),
                    prices: Arc::new(Default::default()),
                },
            );
            daily_inventory.insert_account_inventory(NaiveDate::from_ymd(2022, 2, 22), map);

            let target_day_inventory = daily_inventory.get_account_inventory(&NaiveDate::from_ymd(2022, 3, 22));
            assert_eq!(1, target_day_inventory.len());
            assert!(target_day_inventory.contains_key("AAAAA"));
        }
    }

    mod option {
        use crate::core::ledger::Ledger;
        use indoc::indoc;

        #[test]
        fn should_read_to_option() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                option "title" "Example accounting book"
                option "operating_currency" "CNY"
            "#})
            .unwrap();
            assert_eq!(ledger.option("title").unwrap(), "Example accounting book");
            assert_eq!(ledger.option("operating_currency").unwrap(), "CNY");
        }
        #[test]
        fn should_store_the_latest_one_given_same_name_option() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                option "title" "Example accounting book"
                option "title" "Example accounting book 2"
            "#})
            .unwrap();
            assert_eq!(ledger.option("title").unwrap(), "Example accounting book 2");
        }
    }
}
