use std::collections::HashSet;
use std::ops::{Div, Mul, Neg};

use bigdecimal::{BigDecimal, One, Signed};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::error::ErrorKind;
use crate::models::*;
use crate::utils::inventory::{Inventory, LotMeta};
use crate::utils::multi_value_map::MultiValueMap;
use crate::Account;

pub type Meta = MultiValueMap<String, ZhangString>;

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Date {
    Date(NaiveDate),
    DateHour(NaiveDateTime),
    Datetime(NaiveDateTime),
}

impl Date {
    pub fn now(timezone: &Tz) -> Date {
        Date::Datetime(Utc::now().with_timezone(timezone).naive_local())
    }
    pub fn to_timezone_datetime(&self, timezone: &Tz) -> DateTime<Tz> {
        timezone.from_local_datetime(&self.naive_datetime()).unwrap()
    }
    pub(crate) fn naive_datetime(&self) -> NaiveDateTime {
        match self {
            Date::Date(date) => date.and_hms_opt(0, 0, 0).expect("cannot construct naive datetime from naive date"),
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
    pub fn as_budget_interval(&self) -> u32 {
        let date = self.naive_date();
        let year = date.year();
        let month = date.month();
        month + (year * 100) as u32
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Open {
    pub date: Date,
    pub account: Account,
    pub commodities: Vec<String>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Close {
    pub date: Date,
    pub account: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Commodity {
    pub date: Date,
    pub currency: String,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BalanceCheck {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    pub meta: Meta,
}
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BalancePad {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    pub pad: Account,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Posting {
    pub flag: Option<Flag>,
    pub account: Account,
    pub units: Option<Amount>,
    pub cost: Option<PostingCost>,
    pub price: Option<SingleTotalPrice>,
    pub comment: Option<String>,
}
impl Posting {
    pub fn set_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct PostingCost {
    pub base: Option<Amount>,
    pub date: Option<Date>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
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
            inventory.add_amount(amount);
        }
        // todo work with commodity precision
        Ok(inventory)
    }

    pub fn txn_postings(&self) -> Vec<TxnPosting> {
        self.postings.iter().map(|posting| TxnPosting { txn: self, posting }).collect_vec()
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

impl TxnPosting<'_> {
    pub fn units(&self) -> Option<Amount> {
        self.posting.units.clone()
    }

    /// if cost is not specified, and it can be indicated from price. e.g.
    /// `Assets:Card 1 CNY @ 10 AAA` then cost `10 AAA` can be indicated from single price`@ 10 AAA`
    pub fn costs(&self) -> Option<Amount> {
        self.posting.cost.as_ref().and_then(|it| it.base.clone()).or_else(|| {
            self.posting.price.as_ref().map(|price| match price {
                SingleTotalPrice::Single(single_price) => single_price.clone(),
                SingleTotalPrice::Total(total_price) => Amount::new(
                    (&total_price.number).div(self.posting.units.as_ref().map(|it| &it.number).unwrap_or(&BigDecimal::one())),
                    total_price.currency.clone(),
                ),
            })
        })
    }
    /// trade amount means the amount used for other postings to calculate balance
    /// 1. if `unit` is null, return null
    /// 2. if `unit` is present,
    ///    2.1 return `unit * cost`, if cost is present
    ///    2.2 return `unit * single_price`, if single price is present
    ///    2.3 return `total_price * unit.sign()`, if total price is present
    ///    2.4 return `unit`, if both cost and price are not present.
    pub fn trade_amount(&self) -> Option<Amount> {
        self.posting
            .units
            .as_ref()
            .map(|unit| match (self.posting.cost.as_ref(), self.posting.price.as_ref()) {
                (Some(PostingCost { base: Some(cost), date: _ }), _) => Amount::new((&unit.number).mul(&cost.number), cost.currency.clone()),
                (None, Some(price)) => match price {
                    SingleTotalPrice::Single(single_price) => Amount::new((&unit.number).mul(&single_price.number), single_price.currency.clone()),
                    SingleTotalPrice::Total(total_price) => {
                        if unit.number.is_negative() {
                            total_price.neg()
                        } else {
                            total_price.clone()
                        }
                    }
                },
                _ => unit.clone(),
            })
    }

    /// infer the trade amount based on other postings, if it's trade amount is null
    pub fn infer_trade_amount(&self) -> Result<Amount, ErrorKind> {
        self.trade_amount().map(Ok).unwrap_or_else(|| {
            // get other postings' trade amount
            let (trade_amount_postings, non_trade_amount_postings): (Vec<Option<Amount>>, Vec<Option<Amount>>) =
                self.txn.txn_postings().iter().map(|it| (it.trade_amount())).partition(|it| it.is_some());
            match non_trade_amount_postings.len() {
                0 => unreachable!("txn should not have zero posting"),
                1 => {
                    let mut inventory = Inventory {
                        currencies: Default::default(),
                    };
                    for trade_amount in trade_amount_postings.into_iter().flatten() {
                        inventory.add_amount(trade_amount);
                    }
                    match inventory.size() {
                        0 => Err(ErrorKind::TransactionCannotInferTradeAmount),
                        1 => {
                            if let Some(inventory_balance) = inventory.pop() {
                                Ok(inventory_balance.neg())
                            } else {
                                Err(ErrorKind::TransactionCannotInferTradeAmount)
                            }
                        }
                        _ => Err(ErrorKind::TransactionExplicitPostingHaveMultipleCommodity),
                    }
                }
                _ => Err(ErrorKind::TransactionHasMultipleImplicitPosting),
            }
        })
    }

    /// return meta of lots, using to generate lot's record
    pub fn lot_meta(&self) -> LotMeta {
        if let Some(unit) = &self.posting.units {
            LotMeta {
                txn_date: self.txn.date.naive_date(),

                cost: self.posting.cost.clone(),
                price: self.posting.price.clone().map(|price| match price {
                    SingleTotalPrice::Single(amount) => amount,

                    SingleTotalPrice::Total(amount) => amount.div(unit.number.clone()),
                }),
            }
        } else {
            LotMeta {
                txn_date: self.txn.date.naive_date(),

                cost: None,
                price: None,
            }
        }
    }

    pub fn account_name(&self) -> String {
        self.posting.account.content.clone()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Note {
    pub date: Date,
    pub account: Account,
    pub comment: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
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

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Price {
    pub date: Date,

    pub currency: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Document {
    pub date: Date,

    pub account: Account,
    pub filename: ZhangString,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Custom {
    pub date: Date,

    pub custom_type: ZhangString,
    pub values: Vec<StringOrAccount>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Options {
    pub key: ZhangString,
    pub value: ZhangString,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub module: ZhangString,
    pub value: Vec<ZhangString>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Include {
    pub file: ZhangString,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub content: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub date: Date,
    pub name: String,
    pub commodity: String,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BudgetAdd {
    pub date: Date,
    pub name: String,
    pub amount: Amount,

    pub meta: Meta,
}
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BudgetTransfer {
    pub date: Date,
    pub from: String,
    pub to: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BudgetClose {
    pub date: Date,
    pub name: String,

    pub meta: Meta,
}
