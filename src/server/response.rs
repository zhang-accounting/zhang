use std::collections::HashMap;
use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use crate::core::Currency;
use serde::Serialize;
use sqlx::FromRow;
use crate::core::database::type_ext::big_decimal::ZhangBigDecimal;

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
    commodity: String
}

#[derive(Serialize)]
pub struct StatisticResponse {
    total: Vec<StatisticFrameResponse>,
    income:Vec<StatisticFrameResponse>,
    expense: Vec<StatisticFrameResponse>,
    // summaries:
}

#[derive(Serialize, FromRow)]
pub struct MetaResponse {
    key: String,
    value: String
}



#[derive(Serialize)]
#[serde(tag = "type")]
pub enum JournalItemResponse {
    Transaction(JournalTransactionItemResponse),
    BalanceCheck(JournalBalanceCheckItemResponse),
    BalancePad(JournalBalancePadItemResponse),
}



#[derive(Serialize)]
pub struct JournalTransactionItemResponse {
    pub id: String,
    pub datetime: NaiveDateTime,
    pub payee:String,
    pub narration: Option<String>,
    pub tags:Vec<String>,
    pub links:Vec<String>,
    pub flag: String,
    pub is_balanced: bool,
    pub postings: Vec<JournalTransactionPostingResponse>,
    pub metas:Vec<MetaResponse>
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
}

#[derive(Serialize)]
pub struct JournalBalancePadItemResponse {
    pub id: String,
    pub datetime: NaiveDateTime,
    pub payee:String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>
}

#[derive(Serialize)]
pub struct InfoForNewTransaction {
    pub payee: Vec<String>,
    pub account_name:Vec<String>
}


#[derive(Serialize)]
pub struct AmountResponse {
    pub number: ZhangBigDecimal,
    pub commodity: String
}

