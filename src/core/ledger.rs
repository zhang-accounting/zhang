use crate::core::amount::Amount;
use crate::core::data::Commodity;
use crate::core::inventory::{AccountName, Currency};
use crate::core::models::Directive;
use crate::error::{ZhangError, ZhangResult};
use crate::parse_zhang;
use bigdecimal::{BigDecimal, Zero};
use itertools::Itertools;
use log::{debug, error};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Add;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum AccountStatus {
    Open,
    Close,
}
#[derive(Debug)]
pub struct AccountInfo {
    currencies: HashSet<Currency>,
    status: AccountStatus,
}
#[derive(Debug)]
pub struct CurrencyInfo {
    pub commodity: Commodity,
}

#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    inner: HashMap<Currency, BigDecimal>,
}

impl AccountSnapshot {
    pub fn new() -> Self {
        Self {
            inner: HashMap::default(),
        }
    }
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
}

#[derive(Debug)]
pub struct Ledger {
    pub(crate) directives: Vec<Directive>,
    pub metas: Vec<Directive>,
    pub accounts: HashMap<AccountName, AccountInfo>,
    pub currencies: HashMap<Currency, CurrencyInfo>,
    pub snapshot: HashMap<AccountName, AccountSnapshot>,
}

impl Ledger {
    pub fn load(entry: PathBuf) -> ZhangResult<Ledger> {
        let mut load_queue = VecDeque::new();
        load_queue.push_back(entry);

        let mut visited = HashSet::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
            dbg!(&load_entity);
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
        Ledger::process(dbg!(directives))
    }

    fn load_directive_from_file(entry: PathBuf) -> ZhangResult<Vec<Directive>> {
        let content = std::fs::read_to_string(entry)?;
        parse_zhang(&content).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn process(directives: Vec<Directive>) -> ZhangResult<Ledger> {
        let (meta_directives, dated_directive): (Vec<Directive>, Vec<Directive>) = directives
            .into_iter()
            .partition(|it| it.datetime().is_none());
        let directives = Ledger::sort_directives_datetime(dated_directive);
        let mut accounts = HashMap::default();
        let mut currencies = HashMap::default();
        let mut snapshot: HashMap<AccountName, AccountSnapshot> = HashMap::default();
        for directive in &directives {
            match directive {
                Directive::Open(open) => {
                    let account_info = accounts
                        .entry(open.account.content.to_string())
                        .or_insert_with(|| AccountInfo {
                            currencies: Default::default(),
                            status: AccountStatus::Open,
                        });
                    account_info.status = AccountStatus::Open;
                    for currency in &open.commodities {
                        account_info.currencies.insert(currency.to_string());
                    }
                }
                Directive::Close(close) => {
                    let account_info = accounts
                        .entry(close.account.content.to_string())
                        .or_insert_with(|| AccountInfo {
                            currencies: Default::default(),
                            status: AccountStatus::Open,
                        });
                    account_info.status = AccountStatus::Close;
                }
                Directive::Commodity(commodity) => {
                    let _target_currency = currencies
                        .entry(commodity.currency.to_string())
                        .or_insert_with(|| CurrencyInfo {
                            commodity: commodity.clone(),
                        });
                }
                Directive::Transaction(trx) => {
                    if trx.is_balance() {
                        error!("trx is not balanced");
                    }
                    for txn_posting in trx.txn_postings() {
                        let target_account_snapshot = snapshot
                            .entry(txn_posting.account_name())
                            .or_insert_with(|| AccountSnapshot::new());
                        target_account_snapshot.add_amount(txn_posting.units());
                    }
                }
                Directive::Balance(_) => {}
                Directive::Pad(_) => {}
                Directive::Note(_) => {}
                Directive::Document(_) => {}
                Directive::Price(_) => {}
                Directive::Event(_) => {}
                Directive::Custom(_) => {}
                _ => {}
            }
        }
        Ok(Self {
            metas: meta_directives,
            directives,
            accounts,
            currencies,
            snapshot,
        })
    }

    pub fn load_from_str(content: impl AsRef<str>) -> ZhangResult<Ledger> {
        let content = content.as_ref();
        let directives =
            parse_zhang(content).map_err(|it| ZhangError::PestError(it.to_string()))?;
        Ledger::process(directives)
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
            let ledger = Ledger::load(dbg!(example)).unwrap();
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
        use crate::core::ledger::test::test_parse_zhang;
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
    }
}
