use std::collections::HashMap;
use std::path::PathBuf;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::Serialize;
use strum::{AsRefStr, EnumString};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Currency, Rounding, SpanInfo};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, AsRefStr, EnumString)]
pub enum MetaType {
    AccountMeta,
    CommodityMeta,
    TransactionMeta,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct OptionDomain {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AccountDomain {
    pub date: NaiveDateTime,
    pub r#type: String,
    pub name: String,
    pub status: AccountStatus,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Serialize, AsRefStr, EnumString)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum AccountStatus {
    Open,
    Close,
}

#[derive(Debug, Clone)]
pub struct AccountBalanceDomain {
    pub datetime: NaiveDateTime,
    pub account: String,
    pub account_status: AccountStatus,
    pub balance: Amount,
}

#[derive(Debug, Clone)]
pub struct AccountDailyBalanceDomain {
    pub date: NaiveDate,
    pub account: String,
    pub balance: Amount,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PriceDomain {
    pub datetime: NaiveDateTime,
    pub commodity: Currency,
    pub amount: BigDecimal,
    pub target_commodity: Currency,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetaDomain {
    pub meta_type: String,
    pub type_identifier: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommodityDomain {
    pub name: String,
    pub precision: i32,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub rounding: Rounding,
}

#[derive(Debug, Clone)]
pub struct TransactionInfoDomain {
    pub id: String,
    pub source_file: PathBuf,
    pub span_start: usize,
    pub span_end: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct AccountJournalDomain {
    pub datetime: NaiveDateTime,
    pub timestamp: i64,
    pub account: String,
    pub trx_id: String,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub inferred_unit: Amount,
    pub account_after: Amount,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDomain {
    pub id: String,
    pub span: Option<SpanInfo>,
    pub error_type: ErrorKind,
    pub metas: HashMap<String, String>,
}
