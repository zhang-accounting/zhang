use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use sqlx::FromRow;
use zhang_ast::Currency;

#[derive(FromRow, Debug, Clone)]
pub struct OptionDomain {
    pub key: String,
    pub value: String,
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
