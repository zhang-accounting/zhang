use chrono::NaiveDate;
use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use sqlx::FromRow;
#[derive(FromRow)]
pub struct AccountDailyBalance {
    pub date: NaiveDate,
    pub account: String,
    pub balance_number: ZhangBigDecimal,
    pub balance_commodity: String,
}