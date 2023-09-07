use std::ops::Deref;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};
#[cfg(feature = "sqlite")]
use sqlx::database::{HasArguments, HasValueRef};
#[cfg(feature = "sqlite")]
use sqlx::encode::IsNull;
#[cfg(feature = "sqlite")]
use sqlx::error::BoxDynError;
#[cfg(feature = "sqlite")]
use sqlx::sqlite::SqliteTypeInfo;
#[cfg(feature = "sqlite")]
use sqlx::{Database, Decode, Encode, Sqlite};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZhangBigDecimal(pub BigDecimal);

impl Deref for ZhangBigDecimal {
    type Target = BigDecimal;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "sqlite")]
impl<'r, DB: Database> Decode<'r, DB> for ZhangBigDecimal
where
    String: Decode<'r, DB>,
{
    fn decode(value: <DB as HasValueRef<'r>>::ValueRef) -> Result<Self, BoxDynError> {
        let value = <String as Decode<DB>>::decode(value)?;
        Ok(ZhangBigDecimal(BigDecimal::from_str(&value).unwrap()))
    }
}
#[cfg(feature = "sqlite")]
impl<'q, DB: Database> Encode<'q, DB> for ZhangBigDecimal
where
    String: Encode<'q, DB>,
{
    fn encode_by_ref(&self, buf: &mut <DB as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
        let string = self.0.to_string();
        <String as Encode<DB>>::encode(string, buf)
    }
}

#[cfg(feature = "sqlite")]
impl sqlx::Type<Sqlite> for ZhangBigDecimal {
    fn type_info() -> SqliteTypeInfo {
        <f64 as sqlx::Type<Sqlite>>::type_info()
    }
}
