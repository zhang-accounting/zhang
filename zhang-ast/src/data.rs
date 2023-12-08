use std::collections::HashSet;
use std::ops::{Div, Mul, Neg};

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use itertools::Itertools;

use crate::amount::Amount;
use crate::error::ErrorKind;
use crate::models::*;
use crate::utils::inventory::{AmountLotPair, Inventory, LotInfo};
use crate::utils::multi_value_map::MultiValueMap;
use crate::Account;

pub type Meta = MultiValueMap<String, ZhangString>;

#[derive(Debug, PartialEq, Eq, Clone)]
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
        dbg!(timezone).from_local_datetime(dbg!(&self.naive_datetime())).unwrap()
    }
    pub(crate) fn naive_datetime(&self) -> NaiveDateTime {
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
    pub fn as_budget_interval(&self) -> u32 {
        let date = self.naive_date();
        let year = date.year();
        let month = date.month();
        month + (year * 100) as u32
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BalanceCheck {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    pub meta: Meta,
}
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BalancePad {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
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
                    SingleTotalPrice::Single(single_price) => Amount::new((&unit.number).mul(&single_price.number), single_price.currency.clone()),
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
                    SingleTotalPrice::Single(amount) => Some(LotInfo::Lot(amount.currency.clone(), amount.number.clone())),
                    SingleTotalPrice::Total(amount) => Some(LotInfo::Lot(amount.currency.clone(), (&amount.number).div(&unit.number))),
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

#[derive(Debug, PartialEq, Eq)]
pub struct Budget {
    pub date: Date,
    pub name: String,
    pub commodity: String,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BudgetAdd {
    pub date: Date,
    pub name: String,
    pub amount: Amount,

    pub meta: Meta,
}
#[derive(Debug, PartialEq, Eq)]
pub struct BudgetTransfer {
    pub date: Date,
    pub from: String,
    pub to: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BudgetClose {
    pub date: Date,
    pub name: String,

    pub meta: Meta,
}
