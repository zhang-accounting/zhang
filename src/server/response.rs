use std::collections::HashMap;
use bigdecimal::BigDecimal;
use crate::core::Currency;
use serde::Serialize;

#[derive(Serialize)]
pub struct AccountResponse {
    pub name: String,
    pub status: String,
    pub commodities: HashMap<Currency, BigDecimal>,
}