use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::models::{Flag, StringOrAccount};
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::{HashMap, HashSet};

pub type Meta = HashMap<String, String>;

#[derive(Debug, PartialEq)]
pub struct Open {
    pub date: NaiveDateTime,
    pub account: Account,
    pub commodities: Vec<String>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Close {
    pub date: NaiveDateTime,
    pub account: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Commodity {
    pub date: NaiveDateTime,
    pub currency: String,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Pad {
    pub date: NaiveDateTime,
    pub account: Account,
    pub source: Account,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Balance {
    pub date: NaiveDateTime,
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
    pub date: NaiveDateTime,
    pub flag: Option<Flag>,
    pub payee: Option<String>,
    pub narration: Option<String>,
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
    pub date: NaiveDateTime,
    pub account:Account,
    pub comment: String,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Event {
    pub date: NaiveDateTime,

    pub event_type: String,
    pub description: String,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Query {
    pub date: NaiveDateTime,

    pub name: String,
    pub query_string: String,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Price {
    pub date: NaiveDateTime,

    pub currency: String,
    pub amount: Amount,

    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Document {
    pub date: NaiveDateTime,

    pub account: Account,
    pub filename: String,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
    pub meta: Meta,
}

#[derive(Debug, PartialEq)]
pub struct Custom {
    pub date: NaiveDateTime,

    pub custom_type: String,
    pub values: Vec<StringOrAccount>,
    pub meta: Meta,
}
