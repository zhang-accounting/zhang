use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::models::{Flag, StringOrAccount, ZhangString};
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::{HashMap, HashSet};

pub type Meta = HashMap<String, ZhangString>;

#[derive(Debug, PartialEq)]
pub enum Date {
    Date(NaiveDate),
    Datetime(NaiveDateTime),
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

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
pub struct TxnPosting<'a> {
    txn: &'a Transaction,
    posting: &'a Posting,
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
