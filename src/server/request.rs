use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use serde::Deserialize;
use std::cmp::{max, min};

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
    pub content: String,
}

#[derive(Deserialize)]
pub enum StatisticInterval {
    Day,
    Week,
    Month,
}

#[derive(Deserialize)]
pub struct StatisticRequest {
    from: NaiveDateTime,
    to: NaiveDateTime,
    interval: StatisticInterval,
}

#[derive(Deserialize)]
pub struct JournalRequest {
    page: Option<u32>,
    size: Option<u32>,
}
impl JournalRequest {
    pub fn offset(&self) -> u32 {
        let page = max(dbg!(self.page).unwrap_or(1), 1);
        dbg!((page - 1) * self.limit())
    }
    pub fn limit(&self) -> u32 {
        self.size.unwrap_or(100)
    }
}
