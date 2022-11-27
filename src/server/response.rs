use std::collections::HashMap;
use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use crate::core::Currency;
use serde::Serialize;
use sqlx::FromRow;

#[derive(Serialize)]
pub struct AccountResponse {
    pub name: String,
    pub status: String,
    pub commodities: HashMap<Currency, BigDecimal>,
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
    amount: BigDecimal,
    commodity: String
}

#[derive(Serialize)]
pub struct StatisticResponse {
    total: Vec<StatisticFrameResponse>,
    income:Vec<StatisticFrameResponse>,
    expense: Vec<StatisticFrameResponse>,
    // summaries:
}


#[derive(Serialize)]
pub enum JournalItemResponse {
    Transaction(JournalTransactionItemResponse),
    BalanceCheck(JournalBalanceCheckItemResponse),
    BalancePad(JournalBalancePadItemResponse),
}



#[derive(Serialize)]
pub struct JournalTransactionItemResponse {
    datetime: NaiveDateTime,
    payee:String,
    narration: Option<String>,
    tags:Vec<String>,
    links:Vec<String>,
    type_: String,
    is_balanced: bool,
    // todo postings
}

#[derive(Serialize)]
pub struct JournalBalanceCheckItemResponse {
}

#[derive(Serialize)]
pub struct JournalBalancePadItemResponse {
}
