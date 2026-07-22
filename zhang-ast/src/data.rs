use std::collections::HashSet;

use bigdecimal::BigDecimal;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::models::*;
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
    /// optional absolute tolerance for the assertion (beancount `~` syntax)
    pub tolerance: Option<BigDecimal>,
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

#[derive(Debug, PartialEq, Eq, Clone, Default, Serialize, Deserialize)]
pub struct PostingCost {
    pub base: Option<Amount>,
    pub date: Option<Date>,
    /// lot label from a `{ ..., "label" }` cost spec
    pub label: Option<String>,
    /// true when written as `{{ }}` (total cost) rather than `{ }` (per-unit)
    pub total: bool,
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
    pub fn has_account(&self, name: &String) -> bool {
        self.postings.iter().any(|posting| posting.account.content.eq(name))
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
