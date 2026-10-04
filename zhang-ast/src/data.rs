use std::collections::HashSet;

use bigdecimal::BigDecimal;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::amount::Amount;
use crate::models::*;
use crate::utils::multi_value_map::MultiValueMap;
use crate::utils::timezone::resolve_local_datetime;
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
    /// The instant this wall-clock date means in `timezone`. A date-only value means local
    /// midnight. Times that daylight saving makes ambiguous or skips are resolved by
    /// [`resolve_local_datetime`] instead of panicking.
    pub fn to_timezone_datetime(&self, timezone: &Tz) -> DateTime<Tz> {
        resolve_local_datetime(timezone, &self.naive_datetime())
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
/// `balance ... with pad`: pads its account from `pad` to exactly the asserted amount
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BalancePad {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,
    pub pad: Account,

    pub meta: Meta,
}

/// beancount's `pad`: pads `account` from `pad` to exactly the amount the next balance assertion of
/// `account` in each currency asserts. The padding transaction is dated on the pad
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Pad {
    pub date: Date,
    pub account: Account,
    /// the account the padding comes from
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
    /// Metadata of the posting: the metadata lines written under it, as opposed to the
    /// transaction's own [`Transaction::meta`].
    ///
    /// Which lines belong to a posting depends on the format: in beancount every metadata
    /// line after a posting belongs to it, in zhang only a line indented deeper than the
    /// posting line does.
    ///
    /// A missing field reads as empty, so a WASM plugin built against an older zhang-ast,
    /// whose postings have no `meta`, still exchanges directives with zhang. Such a plugin
    /// does not know the field, though, and writes back every directive it is given: every
    /// transaction that passes through it loses the metadata of its postings.
    #[serde(default)]
    pub meta: Meta,
    /// What the user wrote, when booking changed this posting (booking-split design, #423):
    /// filled in the units of a posting written without them, resolved its cost spec to the
    /// per-unit cost and acquisition date of a lot, or split a reduction into one posting per
    /// lot. The legs of a split are adjacent and share the `index` of the posting they came
    /// from. `None` for a posting booking left as written.
    ///
    /// Advisory: nothing reads it for balances, lots or errors. Journal rows, the edit form and
    /// the exporter ([`written_postings`]) show the posting as written when it is here. A
    /// missing field reads as `None`, and `None` is not serialized, so a posting booking left
    /// alone serializes exactly as it did before the field existed; a plugin built against an
    /// older zhang-ast drops it, which only changes how such rows look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<WrittenPosting>,
}
impl Posting {
    pub fn set_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }
}

/// The written form of a posting booking changed; see [`Posting::written`].
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct WrittenPosting {
    /// position of the written posting in its transaction; all the legs of a split share it
    pub index: usize,
    /// the units as written; `None` for a posting written without units, which booking
    /// interpolated
    pub units: Option<Amount>,
    /// the cost spec as written: `{}`, `{{1000 USD}}`, `{185 USD}` without a date, ...
    pub cost: Option<PostingCost>,
}

/// `postings` as written: the legs booking split from one posting, adjacent and sharing
/// [`WrittenPosting::index`], merged back into that posting, with its units and cost as written
/// and the metadata of its first leg. A posting without [`Posting::written`] is kept as it is.
pub fn written_postings(postings: Vec<Posting>) -> Vec<Posting> {
    let mut written: Vec<Posting> = Vec::with_capacity(postings.len());
    let mut last_index: Option<usize> = None;
    for mut posting in postings {
        let index = match posting.written.take() {
            Some(form) => {
                if last_index == Some(form.index) {
                    // another leg of the posting merged last
                    continue;
                }
                posting.units = form.units;
                posting.cost = form.cost;
                Some(form.index)
            }
            None => None,
        };
        written.push(posting);
        last_index = index;
    }
    written
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

    /// the postings as written ([`written_postings`])
    pub fn written_postings(&self) -> Vec<Posting> {
        written_postings(self.postings.clone())
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

/// A named query stored in the ledger: `2024-01-01 query "name" "SELECT ..."`.
///
/// The query text is kept verbatim; it is neither parsed nor validated when the
/// ledger is loaded.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
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
