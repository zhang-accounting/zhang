use crate::account::Account;
use crate::amount::Amount;
use crate::models::Flag;
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::{HashMap, HashSet};

pub type Meta = HashMap<String, String>;

pub struct Open {
    date: NaiveDateTime,
    account: Account,
    commodities: Vec<String>,
    meta: Meta,
}

pub struct Close {
    date: NaiveDateTime,
    account: Account,
    meta: Meta,
}

pub struct Commodity {
    date: NaiveDateTime,
    currency: String,
    meta: Meta,
}

pub struct Pad {
    date: NaiveDateTime,
    account: Account,
    source: Account,
    meta: Meta,
}

pub struct Balance {
    date: NaiveDateTime,
    account: Account,
    amount: Amount,
    /// the amount of tolerance to use in the verification.
    tolerance: Option<BigDecimal>,
    /// None if the balance check succeeds. This value is set to
    /// an Amount instance if the balance fails, the amount of the difference.
    diff_amount: Option<Amount>,
    meta: Meta,
}

#[derive(Debug)]
pub struct Posting {
    pub flag: Option<Flag>,
    pub account: Account,
    pub units: Amount,
    pub cost: Option<Amount>,
    pub price: Option<Amount>,

    pub meta: Meta,
}

#[derive(Debug)]
pub struct Transaction {
    pub date: NaiveDateTime,
    pub flag: Option<Flag>,
    pub payee: Option<String>,
    pub narration: String,
    pub tags: HashSet<String>,
    pub links: HashSet<String>,
    pub postings: Vec<Posting>,
    pub meta: Meta,
}

pub struct TxnPosting<'a> {
    txn: &'a Transaction,
    posting: &'a Posting,
}

pub struct Note {
    date: NaiveDateTime,
    account:Account,
    comment: String,
    tags: Option<HashSet<String>>,
    links: Option<HashSet<String>>,

    meta: Meta,
}

pub struct Event {
    date: NaiveDateTime,

    event_type: String,
    description: String,

    meta: Meta,
}

pub struct Query {
    date: NaiveDateTime,

    name: String,
    query_string: String,

    meta: Meta,
}

pub struct Price {
    date: NaiveDateTime,

    currency: String,
    amount: Amount,

    meta: Meta,
}

pub struct Document {
    date: NaiveDateTime,

    account: Account,
    filename: String,
    tags: Option<HashSet<String>>,
    links: Option<HashSet<String>>,
    meta: Meta,
}

pub struct Custom {
    date: NaiveDateTime,

    account: Account,
    custom_type: String,
    values: Vec<String>,
    meta: Meta,
}
