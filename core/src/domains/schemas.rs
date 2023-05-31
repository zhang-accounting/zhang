use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashMap;
use strum::{AsRefStr, EnumString};
use zhang_ast::{Currency, SpanInfo};

macro_rules! text_enum {
    ($enum_type:tt) => {
        impl sqlx::Type<sqlx::sqlite::Sqlite> for $enum_type {
            fn type_info() -> <sqlx::Sqlite as sqlx::Database>::TypeInfo {
                <String as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
        }
        impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for $enum_type
        where
            &'r str: sqlx::Decode<'r, DB>,
        {
            fn decode(value: <DB as sqlx::database::HasValueRef<'r>>::ValueRef) -> Result<Self, sqlx::error::BoxDynError> {
                use std::str::FromStr;
                let value = <&str as sqlx::Decode<DB>>::decode(value)?;
                Ok($enum_type::from_str(value).unwrap())
            }
        }
        impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for $enum_type
        where
            String: sqlx::Encode<'q, DB>,
        {
            fn encode_by_ref(&self, buf: &mut <DB as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
                let enum_str: String = self.as_ref().to_owned();
                <String as sqlx::Encode<DB>>::encode_by_ref(&enum_str, buf)
            }
        }
    };
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, AsRefStr, EnumString)]
pub enum MetaType {
    AccountMeta,
    CommodityMeta,
    TransactionMeta,
}
text_enum! {MetaType}

#[derive(FromRow, Debug, Clone, Serialize)]
pub struct OptionDomain {
    pub key: String,
    pub value: String,
}
#[derive(FromRow, Debug, Clone)]
pub struct AccountDomain {
    pub date: NaiveDateTime,
    pub r#type: String,
    pub name: String,
    pub status: AccountStatus,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Serialize, AsRefStr, EnumString)]
pub enum AccountStatus {
    Open,
    Close,
}
text_enum! {AccountStatus}

#[derive(FromRow, Debug, Clone)]
pub struct AccountBalanceDomain {
    pub datetime: NaiveDateTime,
    pub account: String,
    pub account_status: AccountStatus,
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

#[derive(Debug, Clone, FromRow)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDomain {
    pub id: String,
    pub span: Option<SpanInfo>,
    pub error_type: ErrorType,
    pub metas: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, AsRefStr, EnumString)]
pub enum ErrorType {
    AccountBalanceCheckError,
    AccountDoesNotExist,
    AccountClosed,
    TransactionDoesNotBalance,
    CommodityDoesNotDefine,
    TransactionHasMultipleImplicitPosting,
    CloseNonZeroAccount,
}
text_enum! {ErrorType}
