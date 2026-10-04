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

/// a named query saved in the ledger by a `query` directive.
///
/// The text is stored verbatim: it is not validated at load time, so a ledger may
/// keep queries written for engine features that do not exist yet. Every `query`
/// directive is kept, including ones that share a name — the date tells them apart.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QueryDomain {
    pub date: NaiveDate,
    pub name: String,
    pub query: String,
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
    /// where the transaction is, with the text the ledger loaded there
    pub span: zhang_ast::SpanInfo,
}

/// the balance of an account with its sub-accounts, which a balance assertion on the account is checked against
#[derive(Debug, Clone, Default)]
pub struct BalanceWithSubAccounts {
    /// per currency, the sum of the postings of the account and all its sub-accounts
    pub balance: std::collections::BTreeMap<Currency, BigDecimal>,
    /// whether the account has sub-accounts
    pub has_sub_accounts: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct AccountJournalDomain {
    pub datetime: NaiveDateTime,
    pub timestamp: i64,
    /// the account of the posting, in an account's journal the account itself or one of its sub-accounts; the
    /// asserted account for a balance assertion
    pub account: String,
    /// the id of the transaction; for a balance assertion, its id
    pub trx_id: String,
    pub payee: Option<String>,
    pub narration: Option<String>,
    /// what the row adds to the account; zero for a balance assertion, which changes no balance
    pub inferred_unit: Amount,
    /// the balance after the row, in the row's currency: in an account's journal, the running balance of the account
    /// and its sub-accounts, and for a balance assertion the balance it was checked against
    pub account_after: Amount,
    /// for the row of a balance assertion: the asserted amount; null for a posting
    pub asserted: Option<Amount>,
    /// for the row of a balance assertion: the balance it was checked against, that of the account and
    /// all its sub-accounts; null for a posting
    pub checked_balance: Option<Amount>,
    /// for the row of a balance assertion: whether it held, within its tolerance; null for a posting
    pub passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDomain {
    pub id: String,
    pub span: Option<SpanInfo>,
    pub error_type: ErrorKind,
    pub metas: HashMap<String, String>,
}
