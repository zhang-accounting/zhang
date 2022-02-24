use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::inventory::AccountName;
use crate::core::ledger::AccountSnapshot;
use crate::core::models::{Flag, StringOrAccount, ZhangString};
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::ops::Mul;

pub type Meta = HashMap<String, ZhangString>;

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
pub struct Pad {
    pub date: Date,
    pub account: Account,
    pub source: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Balance {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    /// the amount of tolerance to use in the verification.
    pub tolerance: Option<BigDecimal>,
    /// None if the balance check succeeds. This value is set to
    /// an Amount instance if the balance fails, the amount of the difference.
    pub diff_amount: Option<Amount>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Posting {
    pub flag: Option<Flag>,
    pub account: Account,
    pub units: Option<Amount>,
    pub cost: Option<Amount>,
    pub price: Option<Amount>,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
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
        // todo: check if transaction is balanced
        true
    }

    pub fn txn_postings(&self) -> Vec<TxnPosting> {
        self.postings
            .iter()
            .map(|posting| TxnPosting { txn: self, posting })
            .collect_vec()
    }
}

#[derive(Debug, PartialEq)]
pub struct TxnPosting<'a> {
    txn: &'a Transaction,
    posting: &'a Posting,
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
            let mut snapshot = AccountSnapshot::default();
            for x in vec {
                snapshot.add_amount(x)
            }
            // get missing unit calculated from other postings
            snapshot.pop().unwrap().neg()
        }
    }
    pub fn costs(&self) -> Amount {
        if let Some(cost) = &self.posting.cost {
            cost.clone()
        } else {
            match (&self.posting.units, &self.posting.price) {
                (Some(unit), Some(price)) => {
                    Amount::new((&unit.number).mul(&price.number), price.currency.clone())
                }
                _ => self.units(),
            }
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
