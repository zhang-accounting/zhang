use std::collections::HashMap;
use std::path::PathBuf;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use strum::{AsRefStr, EnumString};
use zhang_ast::{Currency, SpanInfo};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, AsRefStr, EnumString)]
pub enum MetaType {
    AccountMeta,
    CommodityMeta,
    TransactionMeta,
}

#[derive(Debug, Clone, Serialize)]
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
pub enum AccountStatus {
    Open,
    Close,
}

#[derive(Debug, Clone)]
pub struct AccountBalanceDomain {
    pub datetime: NaiveDateTime,
    pub account: String,
    pub account_status: AccountStatus,
    // todo: combine number and commodity
    pub balance_number: BigDecimal,
    pub balance_commodity: String,
}

#[derive(Debug, Clone)]
pub struct AccountDailyBalanceDomain {
    pub date: NaiveDate,
    pub account: String,
    pub balance_number: BigDecimal,
    pub balance_commodity: String,
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
    pub rounding: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TransactionInfoDomain {
    pub id: String,
    pub source_file: PathBuf,
    pub span_start: usize,
    pub span_end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountJournalDomain {
    pub datetime: NaiveDateTime,
    pub account: String,
    pub trx_id: String,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub inferred_unit_number: BigDecimal,
    pub inferred_unit_commodity: String,
    pub account_after_number: BigDecimal,
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

    BudgetDoesNotExist,
}
