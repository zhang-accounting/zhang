use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use sqlx::FromRow;
use zhang_ast::Currency;
use serde::Serialize;

#[derive(FromRow, Debug, Clone)]
pub struct OptionDomain {
    pub key: String,
    pub value: String,
}

#[derive(FromRow, Debug, Clone)]
pub struct AccountBalanceDomain {
    pub date: NaiveDate,
    pub account: String,
    pub account_status: String,
    pub balance_number: ZhangBigDecimal,
    pub balance_commodity: String,
}

#[derive(FromRow, Debug, Clone)]
pub struct AccountDailyBalanceDomain {
    pub date: NaiveDate,
    pub account: String,
    pub balance_number: ZhangBigDecimal,
    pub balance_commodity: String,
}

#[derive(FromRow, Debug, Clone)]
pub struct PriceDomain {
    pub datetime: NaiveDateTime,
    pub commodity: Currency,
    pub amount: ZhangBigDecimal,
    pub target_commodity: Currency,
}

#[derive(FromRow, Debug, Clone)]
pub struct MetaDomain {
    pub meta_type: String,
    pub type_identifier: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct CommodityDomain {
    pub name: String,
    pub precision: i32,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub rounding: Option<String>,
}

#[derive(Debug, Clone,FromRow)]
pub struct TransactionInfoDomain {
    pub id: String,
    pub source_file: String,
    pub span_start: i64,
    pub span_end: i64,
}

#[derive(FromRow, Debug, Clone, Serialize)]
pub struct AccountJournalDomain {
    pub datetime: NaiveDateTime,
    pub account: String,
    pub trx_id: String,
    pub payee: String,
    pub narration: Option<String>,
    pub inferred_unit_number: ZhangBigDecimal,
    pub inferred_unit_commodity: String,
    pub account_after_number: ZhangBigDecimal,
    pub account_after_commodity: String,
}