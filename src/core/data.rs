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
use std::ops::{Div, Mul, Neg};
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
        // let mut inventory = Inventory {
        //     inner: Default::default(),
        //     prices: Arc::new(Default::default()),
        // };
        // self.txn_postings().into_iter().for_each(|tx_posting| {
        //     let amount = tx_posting.units();
        //     inventory.add_amount(amount);
        // });
        // inventory.is_zero()
        todo!()
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
    pub fn units(&self) -> Option<Amount> {
        self.posting.units.clone()

        // if let Some(unit) = &self.posting.units {
        //     unit.clone()
        // } else {
        //     let vec = self
        //         .txn
        //         .txn_postings()
        //         .iter()
        //         .filter(|it| it.posting.units.is_some())
        //         .map(|it| it.costs())
        //         .collect_vec();
        //
        //     let mut others = HashSet::new();
        //     for x in &vec {
        //         others.insert(x.currency.clone());
        //     }
        //
        //     if others.len() > 1 {
        //         // todo error
        //         Amount::new(BigDecimal::zero(), "CNY")
        //     } else {
        //         let currency = others.into_iter().take(1).next().unwrap();
        //         let number = vec.into_iter().map(|it| it.number).sum();
        //         Amount::new(number, currency).neg()
        //     }
        // }
    }
    /// if cost is not specified, and it can be indicated from price. e.g.
    /// `Assets:Card 1 CNY @ 10 AAA` then cost `10 AAA` can be indicated from single price`@ 10 AAA`
    pub fn costs(&self) -> Option<Amount> {
        self.posting.cost.clone().or_else(|| {
            self.posting.price.as_ref().map(|price| match price {
                SingleTotalPrice::Single(single_price) => single_price.clone(),
                SingleTotalPrice::Total(total_price) => Amount::new(
                    (&total_price.number).div(&self.posting.units.as_ref().unwrap().number),
                    total_price.currency.clone(),
                ),
            })
        })
    }
    pub fn trade_amount(&self) -> Option<Amount> {
        if let Some(unit) = self.posting.units.as_ref() {
            Some(match (self.posting.cost.as_ref(), self.posting.price.as_ref()) {
                (Some(cost), _) => Amount::new((&unit.number).mul(&cost.number), cost.currency.clone()),
                (None, Some(price)) => match price {
                    SingleTotalPrice::Single(single_price) => {
                        Amount::new((&unit.number).mul(&single_price.number), single_price.currency.clone())
                    }
                    SingleTotalPrice::Total(total_price) => total_price.clone(),
                },
                (None, None) => unit.clone(),
            })
        } else {
            None
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

        mod txn_posting {
            use crate::core::amount::Amount;
            use crate::core::data::{Date, Transaction};
            use crate::core::models::{Directive, SingleTotalPrice};
            use crate::parse_zhang;
            use bigdecimal::{BigDecimal, FromPrimitive};
            use chrono::NaiveDate;
            use indoc::indoc;

            fn get_first_posting(content: &str) -> Transaction {
                let directive = parse_zhang(content, None).unwrap().pop().unwrap();
                match directive.data {
                    Directive::Transaction(trx) => trx,
                    _ => unreachable!(),
                }
            }

            #[test]
            fn should_get_none_unit_given_auto_balanced_posting() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(None, posting.units());
                assert_eq!(None, posting.costs());
                assert_eq!(None, posting.trade_amount());
            }
            #[test]
            fn should_get_unit() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card 100 CNY
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
                assert_eq!(None, posting.costs());
                assert_eq!(
                    Some(Amount::new(BigDecimal::from(100i32), "CNY")),
                    posting.trade_amount()
                );
            }

            #[test]
            fn should_get_unit_given_single_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card 100 CNY @ 10 AAA
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
                assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
                assert_eq!(
                    Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
                    posting.trade_amount()
                );
            }

            #[test]
            fn should_get_unit_given_cost() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card 100 CNY { 10 AAA }
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
                assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
                assert_eq!(
                    Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
                    posting.trade_amount()
                );
            }

            #[test]
            fn should_get_unit_given_cost_and_single_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card 100 CNY { 10 AAA } @ 11 AAA
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
                assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
                assert_eq!(
                    Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
                    posting.trade_amount()
                );
            }
            #[test]
            fn should_get_unit_given_total_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card 100 CNY @@ 110000 AAA
                "#});
                let posting = trx.txn_postings().pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
                assert_eq!(Some(Amount::new(BigDecimal::from(1100i32), "AAA")), posting.costs());
                assert_eq!(
                    Some(Amount::new(BigDecimal::from(110000i32), "AAA")),
                    posting.trade_amount()
                );
            }
        }
    }
}
