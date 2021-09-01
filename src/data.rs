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

pub struct Posting {
    flag: Option<Flag>,
    account: Account,
    units: Amount,
    cost: Amount,
    price: Amount,

    meta: Meta,
}

pub struct Transaction {
    date: NaiveDateTime,
    flag: Option<Flag>,
    payee: Option<String>,
    narration: String,
    tags: HashSet<String>,
    links: HashSet<String>,
    postings: Vec<Posting>,
    meta: Meta,
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
