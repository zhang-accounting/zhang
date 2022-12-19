use std::ops::Deref;

use bigdecimal::{BigDecimal, FromPrimitive};
use serde::{Deserialize, Serialize};
use sqlx::database::{HasArguments, HasValueRef};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::SqliteTypeInfo;
use sqlx::{Database, Decode, Encode, Sqlite};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZhangBigDecimal(pub BigDecimal);

impl Deref for ZhangBigDecimal {
    type Target = BigDecimal;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'r, DB: Database> Decode<'r, DB> for ZhangBigDecimal
where
    f64: Decode<'r, DB>,
{
    fn decode(value: <DB as HasValueRef<'r>>::ValueRef) -> Result<Self, BoxDynError> {
        let value = <f64 as Decode<DB>>::decode(value)?;
        Ok(ZhangBigDecimal(BigDecimal::from_f64(value).unwrap()))
    }
}
impl<'q, DB: Database> Encode<'q, DB> for ZhangBigDecimal
where
    String: Encode<'q, DB>,
{
    fn encode_by_ref(&self, buf: &mut <DB as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
        let string = self.0.to_string();
        <String as Encode<DB>>::encode(string, buf)
    }
}

impl sqlx::Type<Sqlite> for ZhangBigDecimal {
    fn type_info() -> SqliteTypeInfo {
        <f64 as sqlx::Type<Sqlite>>::type_info()
    }
}
