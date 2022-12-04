use crate::core::database::type_ext::big_decimal::ZhangBigDecimal;
use crate::core::Currency;
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct AccountResponse {
    pub name: String,
    pub status: String,
    pub commodities: HashMap<Currency, ZhangBigDecimal>,
}

#[derive(Serialize, FromRow)]
pub struct DocumentResponse {
    pub datetime: NaiveDateTime,
    pub filename: String,
    pub path: String,
    pub extension: Option<String>,
    pub account: Option<String>,
    pub trx_id: Option<String>,
}

#[derive(Serialize)]
pub struct StatisticFrameResponse {
    datetime: NaiveDateTime,
    amount: ZhangBigDecimal,
    commodity: String,
}

#[derive(Serialize)]
pub struct StatisticResponse {
    pub detail: HashMap<NaiveDate, HashMap<String, AmountResponse>>, // summaries:
}

#[derive(Serialize, FromRow)]
pub struct MetaResponse {
    key: String,
    value: String,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum JournalItemResponse {
    Transaction(JournalTransactionItemResponse),
    BalanceCheck(JournalBalanceCheckItemResponse),
    BalancePad(JournalBalancePadItemResponse),
}

impl JournalItemResponse {
    pub fn sequence(&self) -> i64 {
        match self {
            JournalItemResponse::Transaction(inner) => inner.sequence,
            JournalItemResponse::BalanceCheck(inner) => inner.sequence,
            JournalItemResponse::BalancePad(inner) => inner.sequence,
        }
    }
}

#[derive(Serialize)]
pub struct JournalTransactionItemResponse {
    pub id: String,
    pub sequence: i64,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub flag: String,
    pub is_balanced: bool,
    pub postings: Vec<JournalTransactionPostingResponse>,
    pub metas: Vec<MetaResponse>,
}
#[derive(Serialize)]
pub struct JournalTransactionPostingResponse {
    pub account: String,
    pub unit_number: Option<ZhangBigDecimal>,
    pub unit_commodity: Option<String>,
    pub cost_number: Option<ZhangBigDecimal>,
    pub cost_commodity: Option<String>,
    pub price_number: Option<ZhangBigDecimal>,
    pub price_commodity: Option<String>,
    pub inferred_unit_number: ZhangBigDecimal,
    pub inferred_unit_commodity: String,
    pub account_before_number: ZhangBigDecimal,
    pub account_before_commodity: String,
    pub account_after_number: ZhangBigDecimal,
    pub account_after_commodity: String,
}

#[derive(Serialize)]
pub struct JournalBalanceCheckItemResponse {
    pub id: String,
    pub sequence: i64,
}

#[derive(Serialize)]
pub struct JournalBalancePadItemResponse {
    pub id: String,
    pub sequence: i64,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>,
}

#[derive(Serialize)]
pub struct InfoForNewTransaction {
    pub payee: Vec<String>,
    pub account_name: Vec<String>,
}

#[derive(Serialize)]
pub struct AmountResponse {
    pub number: ZhangBigDecimal,
    pub commodity: String,
}
