use std::collections::HashSet;
use std::ops::{Div, Mul, Neg};

use crate::amount::Amount;
use crate::error::ErrorKind;
use crate::models::*;
use crate::utils::inventory::{AmountLotPair, Inventory, LotInfo};
use crate::utils::multi_value_map::MultiValueMap;
use crate::Account;
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use indexmap::IndexSet;
use itertools::Itertools;

pub type Meta = MultiValueMap<String, ZhangString>;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Date {
    Date(NaiveDate),
    DateHour(NaiveDateTime),
    Datetime(NaiveDateTime),
}

impl Date {
    pub fn naive_datetime(&self) -> NaiveDateTime {
        match self {
            Date::Date(date) => date.and_hms_opt(0, 0, 0).unwrap(),
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

#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    pub date: Date,
    pub account: Account,
    pub commodities: Vec<String>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Close {
    pub date: Date,
    pub account: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Commodity {
    pub date: Date,
    pub currency: String,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Balance {
    BalanceCheck(BalanceCheck),
    BalancePad(BalancePad),
}

#[derive(Debug, PartialEq, Eq, Clone)]
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
#[derive(Debug, PartialEq, Eq, Clone)]
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Posting {
    pub flag: Option<Flag>,
    pub account: Account,
    pub units: Option<Amount>,
    pub cost: Option<Amount>,
    pub cost_date: Option<Date>,
    pub price: Option<SingleTotalPrice>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Transaction {
    pub date: Date,
    pub flag: Option<Flag>,
    pub payee: Option<ZhangString>,
    pub narration: Option<ZhangString>,
    pub tags: IndexSet<String>,
    pub links: IndexSet<String>,
    pub postings: Vec<Posting>,
    pub meta: Meta,
}

impl Transaction {
    pub fn get_postings_inventory(&self) -> Result<Inventory, ErrorKind> {
        let mut inventory = Inventory {
            currencies: Default::default(),
        };
        for posting in self.txn_postings() {
            let amount = posting.infer_trade_amount()?;
            let lot_info = posting.lots().unwrap_or(LotInfo::Fifo);
            inventory.add_lot(amount, lot_info);
        }
        // todo work with commodity precision
        Ok(inventory)
    }

    pub fn txn_postings(&self) -> Vec<TxnPosting> {
        self.postings
            .iter()
            .map(|posting| TxnPosting { txn: self, posting })
            .collect_vec()
    }
    pub fn has_account(&self, name: &String) -> bool {
        self.postings.iter().any(|posting| posting.account.content.eq(name))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TxnPosting<'a> {
    pub txn: &'a Transaction,
    pub posting: &'a Posting,
}

impl<'a> TxnPosting<'a> {
    pub fn units(&self) -> Option<Amount> {
        self.posting.units.clone()
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
        self.posting
            .units
            .as_ref()
            .map(|unit| match (self.posting.cost.as_ref(), self.posting.price.as_ref()) {
                (Some(cost), _) => Amount::new((&unit.number).mul(&cost.number), cost.currency.clone()),
                (None, Some(price)) => match price {
                    SingleTotalPrice::Single(single_price) => {
                        Amount::new((&unit.number).mul(&single_price.number), single_price.currency.clone())
                    }
                    SingleTotalPrice::Total(total_price) => total_price.clone(),
                },
                (None, None) => unit.clone(),
            })
    }

    pub fn infer_trade_amount(&self) -> Result<Amount, ErrorKind> {
        self.trade_amount().map(Ok).unwrap_or_else(|| {
            let (trade_amount_postings, non_trade_amount_postings): (Vec<AmountLotPair>, Vec<AmountLotPair>) = self
                .txn
                .txn_postings()
                .iter()
                .map(|it| (it.trade_amount(), it.lots()))
                .partition(|it| it.0.is_some());

            match non_trade_amount_postings.len() {
                0 => unreachable!(),
                1 => {
                    let mut inventory = Inventory {
                        currencies: Default::default(),
                    };
                    for (trade_amount, lot) in trade_amount_postings {
                        if let Some(trade_amount) = trade_amount {
                            let info = lot.unwrap_or(LotInfo::Fifo);
                            inventory.add_lot(trade_amount, info);
                        }
                    }
                    if inventory.size() > 1 {
                        Err(ErrorKind::TransactionCannotInferTradeAmount)
                    } else {
                        Ok(inventory.pop().unwrap().neg())
                    }
                }
                _ => Err(ErrorKind::TransactionHasMultipleImplicitPosting),
            }
        })
    }
    pub fn lots(&self) -> Option<LotInfo> {
        if let Some(unit) = &self.posting.units {
            if let Some(cost) = &self.posting.cost {
                Some(LotInfo::Lot(cost.currency.clone(), cost.number.clone()))
            } else if let Some(price) = &self.posting.price {
                match price {
                    SingleTotalPrice::Single(amount) => {
                        Some(LotInfo::Lot(amount.currency.clone(), amount.number.clone()))
                    }
                    SingleTotalPrice::Total(amount) => Some(LotInfo::Lot(
                        amount.currency.clone(),
                        (&amount.number).div(&unit.number),
                    )),
                }
            } else {
                None
            }
        } else {
            // should be load account default
            None
        }
    }

    pub fn account_name(&self) -> String {
        self.posting.account.content.clone()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub date: Date,
    pub account: Account,
    pub comment: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    pub date: Date,

    pub event_type: ZhangString,
    pub description: ZhangString,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Query {
    pub date: Date,

    pub name: ZhangString,
    pub query_string: ZhangString,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Price {
    pub date: Date,

    pub currency: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Document {
    pub date: Date,

    pub account: Account,
    pub filename: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Custom {
    pub date: Date,

    pub custom_type: ZhangString,
    pub values: Vec<StringOrAccount>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub key: ZhangString,
    pub value: ZhangString,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plugin {
    pub module: ZhangString,
    pub value: Vec<ZhangString>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Include {
    pub file: ZhangString,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Comment {
    pub content: String,
}

#[cfg(test)]
mod test {
    // mod transaction {
    //     use indoc::indoc;
    //
    //     use crate::models::Directive;

    // todo(refact): move to core lib
    // #[tokio::test]
    // async fn should_return_true_given_balanced_transaction() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2022-06-02 "balanced transaction"
    //           Assets:Card -100 CNY
    //           Expenses:Some 100 CNY
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }

    // todo(refact): move to core lib
    // #[tokio::test]
    // async fn should_return_true_given_two_same_decimal() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2022-06-02 "balanced transaction"
    //           Assets:Card -100.00 CNY
    //           Expenses:Some 100 CNY
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }

    // todo(refact): move to core lib
    // #[tokio::test]
    // async fn should_return_true_given_multiple_posting() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2022-06-02 "balanced transaction"
    //           Assets:Card -100.00 CNY
    //           Expenses:Some 50 CNY
    //           Expenses:Others 50 CNY
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }
    // todo(refact): move to core lib
    // #[tokio::test]
    // async fn should_return_false_given_two_diff_posting_amount() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2022-06-02 "balanced transaction"
    //           Assets:Card -100 CNY
    //           Expenses:Some 90 CNY
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(!ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }
    // #[tokio::test]
    // async fn should_return_false_given_two_diff_currency() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2022-06-02 "balanced transaction"
    //           Assets:Card -100 CNY
    //           Expenses:Some 100 CNY2
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(!ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }
    // #[tokio::test]
    // async fn should_return_true_given_day_price() {
    //     let directive = parse_zhang(
    //         indoc! {r#"
    //         2015-01-05 * "Investing 60% of cash in RGAGX"
    //           Assets:US:Vanguard:RGAGX      4.088 RGAGX {88.07 USD, 2015-01-05}
    //           Assets:US:Vanguard:Cash     -360.03 USD
    //     "#},
    //         None,
    //     )
    //     .unwrap()
    //     .pop()
    //     .unwrap();
    //     let ledger = Ledger::load_from_str("").await.unwrap();
    //     match directive.data {
    //         Directive::Transaction(trx) => {
    //             assert!(ledger.is_transaction_balanced(&trx).await.unwrap());
    //         }
    //         _ => unreachable!(),
    //     }
    // }

    //     mod txn_posting {
    //         use bigdecimal::BigDecimal;
    //         use indoc::indoc;
    //         use crate::amount::Amount;
    //         use crate::data::Transaction;
    //         use crate::models::Directive;
    //         use crate::utils::inventory::LotInfo;
    //
    //         fn get_first_posting(content: &str) -> Transaction {
    //             let directive = parse_zhang(content, None).unwrap().pop().unwrap();
    //             match directive.data {
    //                 Directive::Transaction(trx) => trx,
    //                 _ => unreachable!(),
    //             }
    //         }
    //
    //         #[test]
    //         fn should_get_none_unit_given_auto_balanced_posting() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(None, posting.units());
    //             assert_eq!(None, posting.costs());
    //             assert_eq!(None, posting.trade_amount());
    //         }
    //         #[test]
    //         fn should_get_unit() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
    //             assert_eq!(None, posting.costs());
    //             assert_eq!(
    //                 Some(Amount::new(BigDecimal::from(100i32), "CNY")),
    //                 posting.trade_amount()
    //             );
    //         }
    //
    //         #[test]
    //         fn should_get_unit_given_single_price() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY @ 10 AAA
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
    //             assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
    //             assert_eq!(
    //                 Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
    //                 posting.trade_amount()
    //             );
    //         }
    //
    //         #[test]
    //         fn should_get_unit_given_cost() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY { 10 AAA }
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
    //             assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
    //             assert_eq!(
    //                 Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
    //                 posting.trade_amount()
    //             );
    //         }
    //
    //         #[test]
    //         fn should_get_unit_given_cost_and_single_price() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY { 10 AAA } @ 11 AAA
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
    //             assert_eq!(Some(Amount::new(BigDecimal::from(10i32), "AAA")), posting.costs());
    //             assert_eq!(
    //                 Some(Amount::new(BigDecimal::from(1000i32), "AAA")),
    //                 posting.trade_amount()
    //             );
    //         }
    //         #[test]
    //         fn should_get_unit_given_total_price() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY @@ 110000 AAA
    //             "#});
    //             let posting = trx.txn_postings().pop().unwrap();
    //             assert_eq!(Some(Amount::new(BigDecimal::from(100i32), "CNY")), posting.units());
    //             assert_eq!(Some(Amount::new(BigDecimal::from(1100i32), "AAA")), posting.costs());
    //             assert_eq!(
    //                 Some(Amount::new(BigDecimal::from(110000i32), "AAA")),
    //                 posting.trade_amount()
    //             );
    //         }
    //         #[test]
    //         fn should_get_infer_trade_amount() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 CNY
    //               Assets:Card2
    //             "#});
    //             let mut vec = trx.txn_postings();
    //             let posting = vec.remove(0);
    //             assert_eq!(
    //                 Ok(Amount::new(BigDecimal::from(100i32), "CNY")),
    //                 posting.infer_trade_amount(),
    //                 "Assets:Card 100 CNY"
    //             );
    //             let posting2 = vec.remove(0);
    //             assert_eq!(
    //                 Ok(Amount::new(BigDecimal::from(-100i32), "CNY")),
    //                 posting2.infer_trade_amount(),
    //                 "Assets:Card2"
    //             );
    //         }
    //
    //         #[test]
    //         fn should_get_lots_given_only_unit() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 USD
    //               Assets:Card2
    //             "#});
    //             let mut vec = trx.txn_postings();
    //             let posting = vec.remove(0);
    //             assert_eq!(None, posting.lots(), "Assets:Card 100 USD");
    //             let posting = vec.remove(0);
    //             assert_eq!(None, posting.lots(), "Assets:Card2");
    //         }
    //         #[test]
    //         fn should_get_lots_given_unit_and_cost() {
    //             let trx = get_first_posting(indoc! {r#"
    //             2022-06-02 "balanced transaction"
    //               Assets:Card 100 USD { 7 CNY }
    //               Assets:Card2
    //             "#});
    //             let mut vec = trx.txn_postings();
    //             let posting = vec.remove(0);
    //             assert_eq!(
    //                 Some(LotInfo::Lot("CNY".to_string(), BigDecimal::from(7i32))),
    //                 posting.lots(),
    //                 "Assets:Card 100 USD {{ 7 CNY }}"
    //             );
    //             let posting = vec.remove(0);
    //             assert_eq!(None, posting.lots(), "Assets:Card2");
    //         }
    //     }
    // }
}
