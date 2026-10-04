use std::cmp::max;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
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

#[derive(Schematic, Deserialize)]
pub struct ReportRequest {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Schematic, Deserialize, Debug)]
pub struct JournalRequest {
    pub page: Option<u32>,
    pub size: Option<u32>,
    pub keyword: Option<String>,
    pub tags: Option<HashSet<String>>,
    pub links: Option<HashSet<String>>,
}
impl JournalRequest {
    pub fn page(&self) -> u32 {
        max(self.page.unwrap_or(1), 1)
    }
    pub fn offset(&self) -> u32 {
        let page = self.page();
        (page - 1) * self.limit()
    }
    pub fn limit(&self) -> u32 {
        self.size.unwrap_or(100)
    }
}

/// The page of an account's journal to return: `size` rows of page `page`, counting from 1, `size` at
/// most 1000. Without either, the whole journal.
#[derive(Debug, Default, Schematic, Deserialize)]
pub struct AccountJournalRequest {
    pub page: Option<u32>,
    pub size: Option<u32>,
}

impl AccountJournalRequest {
    /// The default `size` of a page, as in `GET /api/journals`.
    pub const DEFAULT_SIZE: u32 = 100;
    /// The largest `size` of a page, as in `GET /api/journals`.
    pub const MAX_SIZE: u32 = crate::journals::MAX_PAGE_SIZE;

    /// The window of rows the request asks for; `None` for the whole journal. A page or a size of 0,
    /// and a size above [`Self::MAX_SIZE`], are a 400.
    pub fn window(&self) -> Result<Option<crate::account_queries::JournalWindow>, crate::error::ServerError> {
        if self.page.is_none() && self.size.is_none() {
            return Ok(None);
        }
        let page = self.page.unwrap_or(1);
        let size = self.size.unwrap_or(Self::DEFAULT_SIZE);
        if page == 0 || size == 0 {
            return Err(crate::error::ServerError::InvalidInput(format!(
                "page and size count from 1, got page {page} and size {size}"
            )));
        }
        if size > Self::MAX_SIZE {
            return Err(crate::error::ServerError::InvalidInput(format!(
                "a page has at most {} rows, got size {size}",
                Self::MAX_SIZE
            )));
        }
        Ok(Some(crate::account_queries::JournalWindow {
            offset: u64::from(page - 1) * u64::from(size),
            size: u64::from(size),
        }))
    }
}

#[derive(Schematic, Deserialize)]
pub struct CreateTransactionRequest {
    pub datetime: DateTime<Utc>,
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
    pub unit: Option<Amount>,
    /// metadata of the posting, checked like the transaction's `metas`
    pub metas: Option<Vec<MetaRequest>>,
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
