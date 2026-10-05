use std::collections::HashMap;
use std::path::PathBuf;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::Serialize;
use strum::{AsRefStr, EnumString};
use zhang_ast::error::ErrorKind;
use zhang_ast::{Currency, Rounding, SpanInfo};
use zhang_shared::prices::PriceMap;

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
    /// `Close` when the latest `open` or `close` of the account is a `close`, whatever its date. Whether the account is
    /// active at a given time is [`Ledger::account_status`](crate::ledger::Ledger::account_status)
    pub status: AccountStatus,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Serialize, AsRefStr, EnumString)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum AccountStatus {
    Open,
    Close,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PriceDomain {
    pub datetime: NaiveDateTime,
    pub commodity: Currency,
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize")]
    pub amount: BigDecimal,
    pub target_commodity: Currency,
}

impl PriceDomain {
    /// The price map of `prices`, the store's `price` directives in ledger order: what the budget check and the query
    /// engine's valuation convert with.
    pub fn price_map<'a>(prices: impl IntoIterator<Item = &'a PriceDomain>) -> PriceMap {
        PriceMap::from_points(
            prices
                .into_iter()
                .map(|it| (it.datetime.date(), &it.commodity, &it.target_commodity, it.amount.clone())),
        )
    }
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

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDomain {
    pub id: String,
    pub span: Option<SpanInfo>,
    pub error_type: ErrorKind,
    pub metas: HashMap<String, String>,
}
