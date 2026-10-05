use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use gotcha::Schematic;
use serde::Deserialize;
use zhang_ast::amount::Amount;
use zhang_ast::Flag;

use crate::error::ServerError;
use crate::ServerResult;

#[derive(Schematic, Deserialize)]
#[serde(tag = "type")]
pub enum BatchAccountBalanceRequest {
    Check { account_name: String, amount: Amount },
    Pad { account_name: String, amount: Amount, pad: String },
}

#[derive(Schematic, Deserialize)]
#[serde(tag = "type")]
pub enum AccountBalanceRequest {
    Check { amount: Amount },
    Pad { amount: Amount, pad: String },
}

#[derive(Schematic, Deserialize)]
pub struct FileUpdateRequest {
    pub content: String,
    /// the `sha256` the file was served with (`GET /api/files/{path}`): the save is refused with 409, and writes
    /// nothing, when the file no longer has that content. Without it the file is overwritten as it is, as before
    pub expected_sha256: Option<String>,
}

/// The buckets of the report graph: calendar days, weeks starting on Monday, or months.
#[derive(Schematic, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatisticInterval {
    Day,
    Week,
    Month,
}

/// A report range. `from` and `to` are ledger dates, `YYYY-MM-DD`, both inclusive; an RFC 3339
/// instant is still accepted and read as its date in the ledger's timezone.
#[derive(Schematic, Deserialize)]
pub struct StatisticRequest {
    pub from: String,
    pub to: String,
}

/// A report range, as in [`StatisticRequest`], and the buckets of the graph.
#[derive(Schematic, Deserialize)]
pub struct StatisticGraphRequest {
    pub from: String,
    pub to: String,
    pub interval: StatisticInterval,
}

/// What the new-transaction form asks: the accounts open at `datetime`, the transaction's date and time as the form
/// submits it ([`LedgerDateTime`]); now when it is left out.
#[derive(Schematic, Deserialize, Debug, Default)]
pub struct NewTransactionInfoRequest {
    pub datetime: Option<LedgerDateTime>,
}

/// A date and time of the ledger in a request: its wall-clock time in the ledger's timezone, written without an offset, such
/// as `2024-01-02T07:00:00`, as every response gives it. An instant with an offset or `Z`, such as `2024-01-01T23:00:00Z`,
/// what requests took before, is still read: it is the wall-clock time it is in the ledger's timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerDateTime {
    /// a wall-clock time of the ledger's timezone
    WallClock(NaiveDateTime),
    /// an instant
    Instant(DateTime<FixedOffset>),
}

impl LedgerDateTime {
    /// the wall-clock time this is in the ledger's timezone `timezone`
    pub fn in_ledger(&self, timezone: &Tz) -> NaiveDateTime {
        match self {
            LedgerDateTime::WallClock(time) => *time,
            LedgerDateTime::Instant(instant) => instant.with_timezone(timezone).naive_local(),
        }
    }
}

impl From<DateTime<Utc>> for LedgerDateTime {
    fn from(instant: DateTime<Utc>) -> Self {
        LedgerDateTime::Instant(instant.fixed_offset())
    }
}

impl FromStr for LedgerDateTime {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
            return Ok(LedgerDateTime::Instant(instant));
        }
        ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
            .map(LedgerDateTime::WallClock)
            .ok_or_else(|| format!("invalid date and time {text:?}: write the ledger's wall-clock time as 2024-01-02T07:00:00"))
    }
}

impl<'de> Deserialize<'de> for LedgerDateTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Schematic for LedgerDateTime {
    fn name() -> &'static str {
        "LedgerDateTime"
    }
    fn required() -> bool {
        true
    }
    fn type_() -> &'static str {
        "string"
    }
    fn doc() -> Option<String> {
        Some(
            "the ledger's wall-clock time, in its timezone, without an offset, as responses give it: `2024-01-02T07:00:00`. An \
             instant with an offset or `Z` is read too, as the wall-clock time it is in the ledger's timezone"
                .to_string(),
        )
    }
}

#[derive(Schematic, Deserialize, Debug)]
pub struct JournalRequest {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub keyword: Option<String>,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
}

/// The page of an account's journal to return: `size` rows of page `page`, by the rule of every paged endpoint
/// ([`crate::journals::window`]). Without either, the whole journal.
#[derive(Debug, Default, Schematic, Deserialize)]
pub struct AccountJournalRequest {
    pub page: Option<u32>,
    pub size: Option<u32>,
}

impl AccountJournalRequest {
    /// The window of rows the request asks for; `None` for the whole journal. A page of 0, and a size of 0 or above
    /// [`crate::journals::MAX_PAGE_SIZE`], are a 400, as on every paged endpoint.
    pub fn window(&self) -> Result<Option<crate::account_queries::JournalWindow>, crate::error::ServerError> {
        if self.page.is_none() && self.size.is_none() {
            return Ok(None);
        }
        let (_, size, offset) = crate::journals::window(self.page, self.size)?;
        Ok(Some(crate::account_queries::JournalWindow {
            offset: offset.unsigned_abs(),
            size: u64::from(size),
        }))
    }
}

#[derive(Schematic, Deserialize)]
pub struct CreateTransactionRequest {
    /// when the transaction is: the ledger's wall-clock time, in its timezone, without an offset, as responses give it
    /// (`2024-01-02T07:00:00`). An instant with an offset or `Z` is read as the wall-clock time it is in the ledger's
    /// timezone
    pub datetime: LedgerDateTime,
    pub payee: String,
    pub flag: Option<FlagRequest>,
    pub narration: Option<String>,
    pub postings: Vec<CreateTransactionPostingRequest>,
    pub metas: Vec<MetaRequest>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

#[derive(Deserialize)]
pub enum FlagRequest {
    Okay,
    Warning,
    BalancePad,
    BalanceCheck,
    #[serde(untagged)]
    Custom(char),
}

impl From<FlagRequest> for Flag {
    fn from(req: FlagRequest) -> Self {
        match req {
            FlagRequest::Okay => Flag::Okay,
            FlagRequest::Warning => Flag::Warning,
            FlagRequest::BalancePad => Flag::BalancePad,
            FlagRequest::BalanceCheck => Flag::BalanceCheck,
            FlagRequest::Custom(c) => Flag::Custom(c.to_string()),
        }
    }
}

impl Schematic for FlagRequest {
    fn name() -> &'static str {
        "FlagRequest"
    }
    fn required() -> bool {
        true
    }
    fn type_() -> &'static str {
        "string"
    }
    fn doc() -> Option<String> {
        Some("The flag of the transaction".to_string())
    }
}

#[derive(Schematic, Deserialize)]
pub struct CreateTransactionPostingRequest {
    pub account: String,
    /// the units of the posting, `null` for the one posting whose units booking infers
    pub unit: Option<UnitRequest>,
    /// metadata of the posting, checked like the transaction's `metas`
    pub metas: Option<Vec<MetaRequest>>,
    /// the cost of the posting as the ledger writes it: `{150 USD}` per unit, `{{1500 USD}}` in total, `{}` for
    /// whatever lot booking finds, or `{150 USD, 2024-01-15, "lot"}` with the acquisition date and the label of
    /// the lot. In an update, a field left out keeps the cost of the posting it edits, `null` removes it
    #[serde(default, deserialize_with = "given")]
    pub cost: Option<Option<String>>,
    /// the price of the posting as the ledger writes it: `@ 6 USD` per unit or `@@ 60 USD` in total. In an update,
    /// a field left out keeps the price of the posting it edits, `null` removes it
    #[serde(default, deserialize_with = "given")]
    pub price: Option<Option<String>>,
    /// the comment at the end of the posting line, without the `;`. In an update, a field left out keeps the
    /// comment of the posting it edits, `null` removes it
    #[serde(default, deserialize_with = "given")]
    pub comment: Option<Option<String>>,
}

/// The units of a posting: an amount, or its text as the ledger writes it, read with the ledger's own grammar, such as
/// `-1,000.50 CNY` or `(10 + 2) / 4 USD`. A text with a number alone is in the ledger's operating currency.
#[derive(Schematic, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum UnitRequest {
    Amount(Amount),
    Text(String),
}

impl From<Amount> for UnitRequest {
    fn from(amount: Amount) -> Self {
        UnitRequest::Amount(amount)
    }
}

/// A field that may be left out of a request, be `null`, or carry a value, told apart: `None` when left out (with
/// `#[serde(default)]`), `Some(None)` for `null`, `Some(Some(value))` otherwise. An update takes a field left out as
/// "as it was", so a client that does not know the field never drops what it stands for.
fn given<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Schematic, Deserialize)]
pub struct MetaRequest {
    pub key: String,
    pub value: String,
}

#[derive(Schematic, Deserialize)]
pub struct BudgetListRequest {
    pub month: Option<u32>,
    pub year: Option<u32>,
}
impl BudgetListRequest {
    /// the old handlers' month, `year * 100 + month`, by default the server's current one
    #[cfg(test)]
    pub fn as_interval(&self) -> u32 {
        let time = chrono::Local::now();
        self.year.unwrap_or(time.year() as u32) * 100 + self.month.unwrap_or(time.month())
    }

    /// the first day of the requested month; the year and the month default to those of `today`
    pub fn month_or(&self, today: NaiveDate) -> ServerResult<NaiveDate> {
        BudgetListRequest::month_of(self.year.unwrap_or(today.year() as u32), self.month.unwrap_or(today.month()))
    }

    /// the first day of a month; a 400 if there is no such month
    pub fn month_of(year: u32, month: u32) -> ServerResult<NaiveDate> {
        i32::try_from(year)
            .ok()
            .and_then(|year| NaiveDate::from_ymd_opt(year, month, 1))
            .ok_or_else(|| ServerError::InvalidInput(format!("there is no month {} in the year {}", month, year)))
    }
}

#[derive(Schematic, Deserialize)]
pub struct BudgetIntervalDetailRequest {
    pub budget_name: String,
    pub year: u32,
    pub month: u32,
}

#[derive(Schematic, Deserialize)]
pub struct QueryRequest {
    /// the BQL query text
    pub query: String,
    /// also count the rows before `LIMIT` and `OFFSET` into the result's `total`, e.g. for the
    /// number of pages; `POST /api/query` only
    pub count_total: Option<bool>,
}

/// The value of a parameter of a built-in query, by its type: a boolean for `bool`, an integer
/// for `int`, a number or a string such as `"12.50"` for `decimal`, a string for `str`, a
/// string `YYYY-MM-DD` for `date` and a list of strings for `set`; `null` is NULL.
#[derive(Schematic, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum BuiltinParamValue {
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
    List(Vec<String>),
}

#[derive(Schematic, Deserialize)]
pub struct BuiltinQueryTextRequest {
    /// the value of every parameter of the query, by name (`from` for `:from`)
    pub params: HashMap<String, Option<BuiltinParamValue>>,
}

#[derive(Schematic, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Schematic, Deserialize)]
pub struct PasskeyRegisterStartRequest {
    /// the `ZHANG_PASSKEY` secret, needed when the caller has no session
    pub secret: Option<String>,
    /// the name of the new passkey
    pub name: Option<String>,
}

#[derive(Schematic, Deserialize)]
pub struct PasskeyRegisterFinishRequest {
    pub state_id: String,
    /// the name of the new passkey, overriding the one given when starting
    pub name: Option<String>,
    /// the `RegisterPublicKeyCredential` of `navigator.credentials.create`
    pub credential: serde_json::Value,
}

#[derive(Schematic, Deserialize)]
pub struct PasskeyLoginFinishRequest {
    pub state_id: String,
    /// the `PublicKeyCredential` of `navigator.credentials.get`
    pub credential: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::CreateTransactionPostingRequest;

    /// The cost, price and comment of a request posting left out, `null` or given are told apart (#473): an update
    /// keeps what is left out, removes what is `null` and writes what is given.
    #[test]
    fn posting_fields_left_out_null_and_given_are_told_apart() {
        let posting: CreateTransactionPostingRequest = serde_json::from_str(r#"{"account": "Assets:Stock", "unit": null}"#).unwrap();
        assert_eq!((posting.cost, posting.price, posting.comment), (None, None, None));

        let json = r#"{"account": "Assets:Stock", "unit": null, "cost": null, "price": "@ 6 USD", "comment": null}"#;
        let posting: CreateTransactionPostingRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            (posting.cost, posting.price, posting.comment),
            (Some(None), Some(Some("@ 6 USD".to_owned())), Some(None))
        );
    }
}
