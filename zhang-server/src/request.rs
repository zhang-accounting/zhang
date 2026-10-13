use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use gotcha::Schematic;
use serde::Deserialize;
use zhang_ast::amount::Amount;
use zhang_ast::{Date, Flag};

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

/// A report range, `from` and `to` as ledger dates, `YYYY-MM-DD`, both inclusive (an RFC 3339 instant is still
/// accepted and read as its date in the ledger's timezone), and the buckets of the graph.
#[derive(Schematic, Deserialize)]
pub struct StatisticGraphRequest {
    pub from: String,
    pub to: String,
    pub interval: StatisticInterval,
}

/// A date and time of the ledger in a request: its wall-clock time in the ledger's timezone, written without an offset, such
/// as `2024-01-02T07:00:00`, as every response gives it. An instant with an offset or `Z`, such as `2024-01-01T23:00:00Z`,
/// what requests took before, is still read: it is the wall-clock time it is in the ledger's timezone. A date alone,
/// `2024-01-02`, is the directive's date without a time: it is written so, as a hand-written entry is, and sorts at
/// midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerDateTime {
    /// a wall-clock time of the ledger's timezone
    WallClock(NaiveDateTime),
    /// an instant
    Instant(DateTime<FixedOffset>),
    /// a date without a time
    Date(NaiveDate),
}

impl LedgerDateTime {
    /// the wall-clock time this is in the ledger's timezone `timezone`; midnight for a date alone
    pub fn in_ledger(&self, timezone: &Tz) -> NaiveDateTime {
        match self {
            LedgerDateTime::WallClock(time) => *time,
            LedgerDateTime::Instant(instant) => instant.with_timezone(timezone).naive_local(),
            LedgerDateTime::Date(date) => date.and_hms_opt(0, 0, 0).expect("midnight exists"),
        }
    }

    /// the date of a directive this is, in the ledger's timezone `timezone`: a date alone stays one, written without
    /// a time
    pub fn ledger_date(&self, timezone: &Tz) -> Date {
        match self {
            LedgerDateTime::Date(date) => Date::Date(*date),
            _ => Date::Datetime(self.in_ledger(timezone)),
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
        if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            return Ok(LedgerDateTime::Date(date));
        }
        ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
            .map(LedgerDateTime::WallClock)
            .ok_or_else(|| format!("invalid date and time {text:?}: write the ledger's wall-clock time as 2024-01-02T07:00:00, or a date as 2024-01-02"))
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
             instant with an offset or `Z` is read too, as the wall-clock time it is in the ledger's timezone; a date alone, \
             `2024-01-02`, is written without a time"
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

/// An `open` of an account, as a plugin's write batch carries one (`zhang_write_preview`, kind `open`); written so a
/// future `POST /api/accounts` can take the same.
#[derive(Schematic, Deserialize)]
pub struct OpenAccountRequest {
    /// the date of the `open`, `2026-10-13`, or the ledger's wall-clock date and time as [`LedgerDateTime`] reads it
    pub date: String,
    pub account: String,
    /// the commodities the account may hold, as the `open` lists them; none for any commodity
    #[serde(default)]
    pub commodities: Vec<String>,
    /// the `booking_method` metadata of the `open`, such as `FIFO`
    pub booking_method: Option<String>,
    /// metadata of the `open`, checked like a transaction's `metas`
    #[serde(default)]
    pub metas: Vec<MetaRequest>,
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

impl From<BuiltinParamValue> for serde_json::Value {
    /// the value as the JSON it was read from, for the engine's `zhang_query::params::from_json`
    fn from(value: BuiltinParamValue) -> Self {
        match value {
            BuiltinParamValue::Bool(it) => serde_json::Value::Bool(it),
            BuiltinParamValue::Int(it) => serde_json::Value::from(it),
            BuiltinParamValue::Number(it) => serde_json::Value::from(it),
            BuiltinParamValue::Text(it) => serde_json::Value::String(it),
            BuiltinParamValue::List(items) => serde_json::Value::from(items),
        }
    }
}

#[derive(Schematic, Deserialize)]
pub struct BuiltinQueryTextRequest {
    /// the value of every parameter of the query, by name (`from` for `:from`)
    pub params: HashMap<String, Option<BuiltinParamValue>>,
}

/// `POST /api/query/builtins/{name}`: run a built-in query with its parameters bound.
#[derive(Schematic, Deserialize)]
pub struct BuiltinQueryRunRequest {
    /// the value of every parameter of the query, by name (`from` for `:from`)
    pub params: HashMap<String, Option<BuiltinParamValue>>,
    /// also count the rows before `LIMIT` and `OFFSET` into the result's `total`, as `POST /api/query` does
    pub count_total: Option<bool>,
}

/// One query of `POST /api/query/builtins`: the name and the body of `POST /api/query/builtins/{name}`.
#[derive(Schematic, Deserialize)]
pub struct BuiltinQueryBatchItem {
    /// the name of the built-in query, as `GET /api/query/builtins` lists it
    pub name: String,
    /// the value of every parameter of the query, by name (`from` for `:from`)
    pub params: HashMap<String, Option<BuiltinParamValue>>,
    /// also count the rows before `LIMIT` and `OFFSET` into this result's `total`
    pub count_total: Option<bool>,
}

#[derive(Schematic, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Schematic, Deserialize)]
pub struct AppCodeRequest {
    /// the app url to return to with the code, of the `zhang-app` scheme or one of `ZHANG_APP_RETURN_SCHEMES`
    pub return_to: String,
}

#[derive(Schematic, Deserialize)]
pub struct AppExchangeRequest {
    /// the one-time code of `POST /api/auth/app/code`
    pub code: String,
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
    use std::str::FromStr;

    use chrono::NaiveDate;
    use zhang_ast::Date;

    use super::{CreateTransactionPostingRequest, LedgerDateTime};

    /// A request's date and time is a wall-clock time, an instant, or a date alone, which is written without a time
    /// and sorts at midnight.
    #[test]
    fn a_date_alone_is_read_and_written_without_a_time() {
        let timezone = chrono_tz::Asia::Shanghai;
        let date = LedgerDateTime::from_str("2024-01-02").unwrap();
        assert_eq!(date, LedgerDateTime::Date(NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()));
        assert_eq!(date.in_ledger(&timezone).to_string(), "2024-01-02 00:00:00");
        assert_eq!(date.ledger_date(&timezone), Date::Date(NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()));
        let time = LedgerDateTime::from_str("2024-01-02T07:00:00").unwrap();
        assert_eq!(time.ledger_date(&timezone), Date::Datetime(time.in_ledger(&timezone)));
        assert_eq!(
            LedgerDateTime::from_str("2024-01-01T23:00:00Z").unwrap().in_ledger(&timezone).to_string(),
            "2024-01-02 07:00:00"
        );
        for text in ["20240102", "2024-01-02T07", "soon", ""] {
            assert!(LedgerDateTime::from_str(text).is_err(), "{text}");
        }
    }

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
