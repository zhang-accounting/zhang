use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::models::{Flag, SingleTotalPrice, StringOrAccount, ZhangString};
use crate::core::utils::inventory::Inventory;
use crate::core::utils::multi_value_map::MultiValueMap;
use crate::core::AccountName;
use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveDateTime};
use itertools::Itertools;
use std::collections::HashSet;
use std::ops::Neg;
use std::sync::Arc;

pub type Meta = MultiValueMap<String, ZhangString>;

#[derive(Debug, PartialEq, Clone)]
pub enum Date {
    Date(NaiveDate),
    DateHour(NaiveDateTime),
    Datetime(NaiveDateTime),
}

impl Date {
    pub fn naive_datetime(&self) -> NaiveDateTime {
        match self {
            Date::Date(date) => date.and_hms(0, 0, 0),
            Date::DateHour(date_hour) => *date_hour,
            Date::Datetime(datetime) => *datetime,
        }
    }
    pub fn naive_date(&self) -> NaiveDate {
        match self {
            Date::Date(date) => *date,
            Date::DateHour(date_hour) => date_hour.date(),
            Date::Datetime(datetime) => datetime.date(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Open {
    pub date: Date,
    pub account: Account,
    pub commodities: Vec<String>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Close {
    pub date: Date,
    pub account: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Commodity {
    pub date: Date,
    pub currency: String,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub enum Balance {
    BalanceCheck(BalanceCheck),
    BalancePad(BalancePad),
}

#[derive(Debug, PartialEq, Clone)]
pub struct BalanceCheck {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    /// the amount of tolerance to use in the verification.
    pub tolerance: Option<BigDecimal>,
    /// None if the balance check succeeds. This value is set to
    /// an Amount instance if the balance fails, the amount of the difference.
    pub distance: Option<Amount>,
    pub current_amount: Option<Amount>,
    pub meta: Meta,
}
#[derive(Debug, PartialEq, Clone)]
pub struct BalancePad {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    /// the amount of tolerance to use in the verification.
    pub tolerance: Option<BigDecimal>,
    /// None if the balance check succeeds. This value is set to
    /// an Amount instance if the balance fails, the amount of the difference.
    pub diff_amount: Option<Amount>,
    pub pad: Account,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Posting {
    pub flag: Option<Flag>,
    pub account: Account,
    pub units: Option<Amount>,
    pub cost: Option<Amount>,
    pub cost_date: Option<Date>,
    pub price: Option<SingleTotalPrice>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Transaction {
    pub date: Date,
    pub flag: Option<Flag>,
    pub payee: Option<ZhangString>,
    pub narration: Option<ZhangString>,
    pub tags: HashSet<String>,
    pub links: HashSet<String>,
    pub postings: Vec<Posting>,
    pub meta: Meta,
}

impl Transaction {
    pub fn is_balance(&self) -> bool {
        let mut inventory = Inventory {
            inner: Default::default(),
            prices: Arc::new(Default::default()),
        };
        self.txn_postings().into_iter().for_each(|tx_posting| {
            let amount = tx_posting.units();
            inventory.add_amount(amount);
        });
        inventory.is_zero()
    }

    pub fn txn_postings(&self) -> Vec<TxnPosting> {
        self.postings
            .iter()
            .map(|posting| TxnPosting { txn: self, posting })
            .collect_vec()
    }
    pub fn has_account(&self, name: &AccountName) -> bool {
        self.postings.iter().any(|posting| posting.account.content.eq(name))
    }
}

#[derive(Debug, PartialEq)]
pub struct TxnPosting<'a> {
    pub(crate) txn: &'a Transaction,
    pub(crate) posting: &'a Posting,
}

impl<'a> TxnPosting<'a> {
    pub fn units(&self) -> Amount {
        if let Some(unit) = &self.posting.units {
            unit.clone()
        } else {
            let vec = self
                .txn
                .txn_postings()
                .iter()
                .filter(|it| it.posting.units.is_some())
                .map(|it| it.costs())
                .collect_vec();

            let mut others = HashSet::new();
            for x in &vec {
                others.insert(x.currency.clone());
            }

            if others.len() > 1 {
                // todo error
                Amount::new(BigDecimal::zero(), "CNY")
            } else {
                let currency = others.into_iter().take(1).next().unwrap();
                let number = vec.into_iter().map(|it| it.number).sum();
                Amount::new(number, currency).neg()
            }
        }
    }
    pub fn costs(&self) -> Amount {
        if let Some(cost) = &self.posting.cost {
            cost.clone()
        } else {
            // match (&self.posting.units, &self.posting.single_price) {
            //     (Some(unit), Some(price)) => Amount::new((&unit.number).mul(&price.number), price.currency.clone()),
            //     _ => self.units(),
            // }
            // todo
            self.units()
        }
    }

    pub fn account_name(&self) -> AccountName {
        self.posting.account.content.clone()
    }
}

#[derive(Debug, PartialEq)]
pub struct Note {
    pub date: Date,
    pub account: Account,
    pub comment: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Event {
    pub date: Date,

    pub event_type: ZhangString,
    pub description: ZhangString,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Query {
    pub date: Date,

    pub name: ZhangString,
    pub query_string: ZhangString,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Price {
    pub date: Date,

    pub currency: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Document {
    pub date: Date,

    pub account: Account,
    pub filename: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Custom {
    pub date: Date,

    pub custom_type: ZhangString,
    pub values: Vec<StringOrAccount>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub key: ZhangString,
    pub value: ZhangString,
}

#[derive(Debug, PartialEq)]
pub struct Plugin {
    pub module: ZhangString,
    pub value: Vec<ZhangString>,
}

#[derive(Debug, PartialEq)]
pub struct Include {
    pub file: ZhangString,
}

#[derive(Debug, PartialEq)]
pub struct Comment {
    pub content: String,
}

#[cfg(test)]
mod test {
    mod transaction {
        use crate::core::models::Directive;
        use crate::parse_zhang;
        use indoc::indoc;

        #[test]
        fn should_return_true_given_balanced_transaction() {
            let directive = parse_zhang(
                indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 CNY
                  Expenses:Some 100 CNY
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
        #[test]
        fn should_return_true_given_two_same_decimal() {
            let directive = parse_zhang(
                indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100.00 CNY
                  Expenses:Some 100 CNY
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
        #[test]
        fn should_return_true_given_multiple_posting() {
            let directive = parse_zhang(
                indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100.00 CNY
                  Expenses:Some 50 CNY
                  Expenses:Others 50 CNY
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
        #[test]
        fn should_return_false_given_two_diff_posting_amount() {
            let directive = parse_zhang(
                indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 CNY
                  Expenses:Some 90 CNY
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(!trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
        #[test]
        fn should_return_false_given_two_diff_currency() {
            let directive = parse_zhang(
                indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 CNY
                  Expenses:Some 100 CNY2
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(!trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
        #[test]
        #[ignore]
        fn should_return_true_given_day_price() {
            let directive = parse_zhang(
                indoc! {r#"
                2015-01-05 * "Investing 60% of cash in RGAGX"
                  Assets:US:Vanguard:RGAGX      4.088 RGAGX {88.07 USD, 2015-01-05}
                  Assets:US:Vanguard:Cash     -360.03 USD
            "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap();
            dbg!(&directive);
            match directive.data {
                Directive::Transaction(trx) => {
                    assert!(trx.is_balance());
                }
                _ => unreachable!(),
            }
        }
    }
}
