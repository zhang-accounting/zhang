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