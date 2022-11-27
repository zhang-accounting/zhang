use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum AccountBalanceRequest {
    Check {
        amount: BigDecimal,
        commodity: String,
    },
    Pad {
        amount: BigDecimal,
        commodity: String,
        pad_account: String,
    },
}


#[derive(Deserialize)]
pub struct FileUpdateRequest {
    pub content: String
}

#[derive(Deserialize)]
pub enum StatisticInterval {
    Day,
    Week,
    Month
}


#[derive(Deserialize)]
pub struct StatisticRequest {
    from:NaiveDateTime,
    to:NaiveDateTime,
    interval: StatisticInterval
}

#[derive(Deserialize)]
pub struct JournalRequest {
    page: u32,
    size: u32,

}