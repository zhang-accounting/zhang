use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{Commodity, Date};
use crate::core::inventory::{AccountName, Currency};
use crate::core::models::Directive;
use crate::core::process::{DirectiveProcess, ProcessContext};
use crate::core::utils::multi_value_map::MultiValueMap;
use crate::error::{ZhangError, ZhangResult};
use crate::parse_zhang;
use async_graphql::Enum;
use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use itertools::{Either, Itertools};
use log::debug;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Add;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum DocumentType {
    AccountDocument {
        date: Date,
        account: Account,
        filename: String,
    },
    TransactionDocument {
        date: Date,
        // todo add transaction location info
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountSnapshot {
    #[serde(flatten)]
    pub(crate) inner: HashMap<Currency, BigDecimal>,
}

impl Add for &AccountSnapshot {
    type Output = AccountSnapshot;

    fn add(self, rhs: Self) -> Self::Output {
        let mut snapshot = AccountSnapshot::default();
        for (currency, amount) in &self.inner {
            snapshot.add_amount(Amount::new(amount.clone(), currency));
        }
        for (currency, amount) in &rhs.inner {
            snapshot.add_amount(Amount::new(amount.clone(), currency));
        }
        snapshot
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DailyAccountSnapshot {
    data: HashMap<NaiveDate, HashMap<AccountName, AccountSnapshot>>,
    date_index: BTreeSet<NaiveDate>,
}

impl DailyAccountSnapshot {
    pub(crate) fn insert_snapshot(
        &mut self,
        day: NaiveDate,
        snapshot: HashMap<AccountName, AccountSnapshot>,
    ) {
        self.data.insert(day, snapshot);
        self.date_index.insert(day);
    }
    pub(crate) fn get_snapshot_by_date(
        &self,
        day: &NaiveDate,
    ) -> HashMap<AccountName, AccountSnapshot> {
        let vec = self.date_index.iter().collect_vec();
        let target_day = match vec.binary_search(&day) {
            Ok(_) => day,
            Err(gt_index) => vec[gt_index - 1],
        };
        self.data.get(target_day).cloned().unwrap_or_default()
    }
}

impl AccountSnapshot {
    pub fn add_amount(&mut self, amount: Amount) {
        let decimal1 = BigDecimal::zero();
        let x = self.inner.get(&amount.currency).unwrap_or(&decimal1);
        let decimal = (x).add(&amount.number);
        self.inner.insert(amount.currency, decimal);
    }
    pub fn snapshot(&self) -> AccountSnapshot {
        self.clone()
    }
    pub fn pop(&mut self) -> Option<Amount> {
        self.inner
            .drain()
            .take(1)
            .next()
            .map(|(currency, number)| Amount::new(number, currency))
    }
    pub fn get(&self, currency: &Currency) -> BigDecimal {
        self.inner
            .get(currency)
            .cloned()
            .unwrap_or_else(BigDecimal::zero)
    }
}

#[derive(Clone, Debug)]
pub enum LedgerError {
    AccountBalanceCheckError {
        account_name: String,
        target: Amount,
        current: Amount,
        distance: Amount,
    },
    // AccountDoesNotExist {},
    // AccountClosed {},
    // TransactionDoesNotBalance {},
}

#[derive(Debug)]
pub struct Ledger {
    pub entry: Either<PathBuf, String>,
    pub(crate) visited_files: Vec<PathBuf>,

    pub(crate) directives: Vec<Directive>,
    pub metas: Vec<Directive>,
    pub accounts: HashMap<AccountName, AccountInfo>,
    pub currencies: HashMap<Currency, CurrencyInfo>,
    pub snapshot: HashMap<AccountName, AccountSnapshot>,
    pub daily_snapshot: DailyAccountSnapshot,
    pub documents: HashMap<String, DocumentType>,
    pub errors: Vec<LedgerError>,

    pub configs: HashMap<String, String>,
}

impl Ledger {
    pub fn load(entry: PathBuf) -> ZhangResult<Ledger> {
        let mut load_queue = VecDeque::new();
        load_queue.push_back(entry.clone());

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
                .filter(|it| matches!(it, Directive::Include(_)))
                .for_each(|it| match it {
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
            Either::Left(entry),
            visited.into_iter().collect_vec(),
        )
    }

    fn load_directive_from_file(entry: PathBuf) -> ZhangResult<Vec<Directive>> {
        let content = std::fs::read_to_string(entry)?;
        parse_zhang(&content).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn process(
        directives: Vec<Directive>,
        entry: Either<PathBuf, String>,
        visited_files: Vec<PathBuf>,
    ) -> ZhangResult<Ledger> {
        let (meta_directives, dated_directive): (Vec<Directive>, Vec<Directive>) = directives
            .into_iter()
            .partition(|it| it.datetime().is_none());
        let mut directives = Ledger::sort_directives_datetime(dated_directive);

        let mut ret_ledger = Self {
            entry,
            visited_files,
            directives: vec![],
            metas: meta_directives,
            accounts: HashMap::default(),
            currencies: HashMap::default(),
            snapshot: HashMap::default(),
            daily_snapshot: DailyAccountSnapshot::default(),
            documents: HashMap::default(),
            errors: vec![],
            configs: HashMap::default(),
        };
        let mut context = ProcessContext { target_day: None };
        for directive in &mut directives {
            match directive {
                Directive::Option(option) => option.process(&mut ret_ledger, &mut context)?,
                Directive::Open(open) => open.process(&mut ret_ledger, &mut context)?,
                Directive::Close(close) => close.process(&mut ret_ledger, &mut context)?,
                Directive::Commodity(commodity) => {
                    commodity.process(&mut ret_ledger, &mut context)?
                }
                Directive::Transaction(trx) => trx.process(&mut ret_ledger, &mut context)?,
                Directive::Balance(balance) => balance.process(&mut ret_ledger, &mut context)?,
                Directive::Note(_) => {}
                Directive::Document(document) => document.process(&mut ret_ledger, &mut context)?,
                Directive::Price(_) => {}
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                _ => {}
            }
        }
        if let Some(last_day) = context.target_day {
            ret_ledger
                .daily_snapshot
                .insert_snapshot(last_day, ret_ledger.snapshot.clone());
        }
        ret_ledger.directives = directives;
        Ok(ret_ledger)
    }

    pub fn load_from_str(content: impl AsRef<str>) -> ZhangResult<Ledger> {
        let content = content.as_ref();
        let directives =
            parse_zhang(content).map_err(|it| ZhangError::PestError(it.to_string()))?;
        Ledger::process(directives, Either::Right(content.to_string()), vec![])
    }

    fn sort_directives_datetime(mut directives: Vec<Directive>) -> Vec<Directive> {
        directives.sort_by(|a, b| match (a.datetime(), b.datetime()) {
            (Some(a_datetime), Some(b_datetime)) => a_datetime.cmp(&b_datetime),
            _ => Ordering::Greater,
        });
        directives
    }

    pub fn apply(mut self, applier: impl Fn(Directive) -> Directive) -> Self {
        let vec = self.directives.into_iter().map(applier).collect_vec();
        self.directives = vec;
        self
    }

    pub fn reload(&mut self) -> ZhangResult<()> {
        let reload_ledger = match &mut self.entry {
            Either::Left(path_buf) => Ledger::load(path_buf.clone()),
            Either::Right(raw_string) => Ledger::load_from_str(raw_string),
        }?;
        *self = reload_ledger;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::core::models::Directive;
    use crate::parse_zhang;

    fn test_parse_zhang(content: &str) -> Vec<Directive> {
        parse_zhang(content).expect("cannot parse zhang")
    }

    mod sort_directive_datetime {
        use crate::core::ledger::test::test_parse_zhang;
        use crate::core::ledger::Ledger;
        use indoc::indoc;

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
                "#}),
                sorted
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
                "#}),
                sorted
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
                "#}),
                sorted
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
            let ledger = Ledger::load(example).unwrap();
            assert_eq!(
                test_parse_zhang(indoc! {r#"
                option "title" "Example"
                include "include.zhang"
                option "description" "Example Description"
            "#}),
                ledger.metas
            );
            assert_eq!(0, ledger.directives.len());
        }
    }

    mod account_snapshot {
        use crate::core::amount::Amount;
        use crate::core::ledger::AccountSnapshot;
        use bigdecimal::BigDecimal;

        #[test]
        fn should_add_to_inner() {
            let mut snapshot = AccountSnapshot {
                inner: Default::default(),
            };
            snapshot.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            assert_eq!(1, snapshot.inner.len());
            assert_eq!(&BigDecimal::from(1i32), snapshot.inner.get("CNY").unwrap())
        }

        #[test]
        fn should_snapshot_be_independent() {
            let mut snapshot = AccountSnapshot {
                inner: Default::default(),
            };
            snapshot.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            let new_snapshot = snapshot.snapshot();

            snapshot.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));

            assert_eq!(1, snapshot.inner.len());
            assert_eq!(&BigDecimal::from(2i32), snapshot.inner.get("CNY").unwrap());

            assert_eq!(1, new_snapshot.inner.len());
            assert_eq!(
                &BigDecimal::from(1i32),
                new_snapshot.inner.get("CNY").unwrap()
            )
        }
    }
    mod txn {
        use crate::core::ledger::Ledger;
        use bigdecimal::BigDecimal;
        use indoc::indoc;

        #[test]
        fn should_record_amount_into_snapshot() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To 10 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.snapshot.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_snapshot_given_none_unit_posting_and_single_unit_posting() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To
            "#})
            .unwrap();

            assert_eq!(2, ledger.snapshot.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_snapshot_given_none_unit_posting_and_more_unit_postings() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To
            "#})
            .unwrap();

            assert_eq!(2, ledger.snapshot.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_snapshot_given_unit_postings_and_total_cost() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To 1 BTC @@ 10 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.snapshot.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(1i32),
                ledger
                    .snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("BTC")
                    .unwrap()
            );
        }

        #[test]
        fn should_record_amount_into_snapshot_given_unit_postings_and_single_cost() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY2

                2022-02-22 "Payee"
                  Assets:From -5 CNY
                  Assets:From -5 CNY
                  Expenses:To 10 CNY2 @ 1 CNY
            "#})
            .unwrap();

            assert_eq!(2, ledger.snapshot.len());
            assert_eq!(
                &BigDecimal::from(-10i32),
                ledger
                    .snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                ledger
                    .snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY2")
                    .unwrap()
            );
        }
    }
    mod daily_snapshot {
        use crate::core::ledger::{AccountSnapshot, DailyAccountSnapshot, Ledger};
        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use indoc::indoc;
        use std::collections::HashMap;

        #[test]
        fn should_record_daily_snapshot() {
            let ledger = Ledger::load_from_str(indoc! {r#"
                1970-01-01 open Assets:From CNY
                1970-01-01 open Expenses:To CNY

                2022-02-22 "Payee"
                  Assets:From -10 CNY
                  Expenses:To
            "#})
            .unwrap();

            assert_eq!(1, ledger.daily_snapshot.data.len());
            let account_snapshot = ledger
                .daily_snapshot
                .get_snapshot_by_date(&NaiveDate::from_ymd(2022, 2, 22));
            assert_eq!(
                &BigDecimal::from(-10i32),
                account_snapshot
                    .get("Assets:From")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(10i32),
                account_snapshot
                    .get("Expenses:To")
                    .unwrap()
                    .inner
                    .get("CNY")
                    .unwrap()
            );
        }
        #[test]
        fn should_get_from_previous_day_given_day_is_not_in_data() {
            let mut daily_snapshot = DailyAccountSnapshot::default();
            let mut map = HashMap::default();
            map.insert("AAAAA".to_string(), AccountSnapshot::default());
            daily_snapshot.insert_snapshot(NaiveDate::from_ymd(2022, 2, 22), map);

            let target_day_snapshot =
                daily_snapshot.get_snapshot_by_date(&NaiveDate::from_ymd(2022, 3, 22));
            assert_eq!(1, target_day_snapshot.len());
            assert!(target_day_snapshot.contains_key("AAAAA"));
        }
    }
}
