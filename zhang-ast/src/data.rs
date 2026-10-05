use std::collections::{HashMap, HashSet};

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
    /// Whether the wall-clock time `at` comes after a close dated `self`, such as a `budget-close`: a
    /// close with only a date lasts through its whole day, and one with a time ends at that time.
    pub fn close_precedes(&self, at: NaiveDateTime) -> bool {
        match self {
            Date::Date(date) => at.date() > *date,
            Date::DateHour(datetime) | Date::Datetime(datetime) => at > *datetime,
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
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize_option")]
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

/// A group of adjacent booked postings standing for one posting as written: the legs booking
/// split a reduction into, one per lot, or a single posting. [`written_groups`] yields them; the
/// store's rows, the exporter and the query engine count postings by these groups.
pub struct WrittenGroup<'a> {
    /// the legs, adjacent in the transaction; never empty
    pub legs: &'a [Posting],
    /// the written form the legs restore to, when they are all of their written posting: they
    /// share its index and account, and no other run of adjacent legs carries that index. `None`
    /// for a posting booking left as written, and for a leg a stage moved away from the others,
    /// or gave an index another posting carries (a duplicated posting keeping its `written`):
    /// restoring the written form would double the posting or lose one, so such legs stand on
    /// their own, as booked, and nothing is lost
    pub written: Option<&'a WrittenPosting>,
}

/// `postings` grouped by the posting they were written as. A complete written posting (see
/// [`WrittenGroup::written`]) is one group with all its legs; every other posting is a group of
/// its own.
pub fn written_groups(postings: &[Posting]) -> Vec<WrittenGroup<'_>> {
    // the runs of adjacent legs sharing an index and an account, counted per index
    let mut runs: HashMap<usize, usize> = HashMap::new();
    let mut last_run: Option<(usize, &Account)> = None;
    for posting in postings {
        match &posting.written {
            Some(form) => {
                let run = (form.index, &posting.account);
                if last_run != Some(run) {
                    *runs.entry(form.index).or_default() += 1;
                }
                last_run = Some(run);
            }
            None => last_run = None,
        }
    }

    let mut groups = Vec::with_capacity(postings.len());
    let mut start = 0;
    while start < postings.len() {
        let posting = &postings[start];
        match &posting.written {
            Some(form) if runs.get(&form.index) == Some(&1) => {
                let mut end = start + 1;
                while end < postings.len()
                    && postings[end].written.as_ref().is_some_and(|it| it.index == form.index)
                    && postings[end].account == posting.account
                {
                    end += 1;
                }
                groups.push(WrittenGroup {
                    legs: &postings[start..end],
                    written: Some(form),
                });
                start = end;
            }
            _ => {
                groups.push(WrittenGroup {
                    legs: &postings[start..=start],
                    written: None,
                });
                start += 1;
            }
        }
    }
    groups
}

/// The units of a group of booked legs ([`written_groups`]): their sum, in the commodity of the
/// posting they were written as. Every leg of a booked group has units.
pub fn group_units(legs: &[Posting]) -> Amount {
    booked_group_units(legs).expect("a booked posting has units")
}

/// The units of a group of legs ([`written_groups`]) when every leg is booked: their sum, in the
/// commodity of the first. `None` when a leg has no units, as in a transaction the ledger could not
/// book (its implicit posting is never interpolated), which a reader of the ledger's directives
/// may meet where [`group_units`] would panic.
pub fn booked_group_units(legs: &[Posting]) -> Option<Amount> {
    let commodity = legs.first()?.units.as_ref()?.commodity.clone();
    let number: Option<BigDecimal> = legs.iter().map(|leg| leg.units.as_ref().map(|it| &it.number)).sum();
    Some(Amount::new(number?, commodity))
}

/// `postings` as written: each complete group of [`written_groups`] merged back into the posting
/// it was written as, with its units and cost as written and the flag, price, comment and
/// metadata of its first leg (booking copies the written posting's onto every leg, so none is
/// lost); every other posting as it is, its `written` dropped.
pub fn written_postings(postings: Vec<Posting>) -> Vec<Posting> {
    written_groups(&postings)
        .into_iter()
        .map(|group| {
            let mut posting = group.legs[0].clone();
            if let Some(form) = group.written {
                posting.units = form.units.clone();
                posting.cost = form.cost.clone();
            }
            posting.written = None;
            posting
        })
        .collect()
}

/// The cost spec of a posting, `{ ... }` or `{{ ... }}`: its components come in any order, separated
/// by commas, as in beancount. Each one is a criterion for the lot the posting books against, and
/// the missing ones are wildcards: `{150 USD}`, `{2024-01-15}`, `{"lot"}`, `{150 USD, 2024-01-15, "lot"}`
/// and `{}` are all cost specs.
#[derive(Debug, PartialEq, Eq, Clone, Default, Serialize, Deserialize)]
pub struct PostingCost {
    /// the cost number with its commodity: per unit (`{150 USD}`), or in total when [`total`](Self::total)
    pub base: Option<Amount>,
    pub date: Option<Date>,
    /// lot label from a `{ ..., "label" }` cost spec
    pub label: Option<String>,
    /// true when written as `{{ }}` (total cost) rather than `{ }` (per-unit)
    pub total: bool,
    /// the total part `T` of a compound cost `{P # T USD}`, in the commodity of [`base`](Self::base),
    /// which is the per-unit part `P`: the lot costs `P + T / |units|` per unit, as in beancount, so
    /// `10 HOOL {100 # 5 USD}` books a lot at `100.5 USD`. `None` for every other spec. A missing field
    /// reads as `None`, and `None` is not serialized: a posting without one serializes as before
    /// the field existed
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "zhang_shared::decimal::plain::serialize_option"
    )]
    pub compound_total: Option<BigDecimal>,
    /// true when the spec carries beancount's merge-cost marker `*`, as in `{*}`. Cost merging is
    /// not supported: booking reports it and books the spec as if the marker were not there, as
    /// beancount does. A missing field reads as `false`, and `false` is not serialized
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge: bool,
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
