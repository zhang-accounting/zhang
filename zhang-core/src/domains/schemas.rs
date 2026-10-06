use std::collections::HashMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use chrono_tz::Tz;
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use log::debug;
use serde::Serialize;
use strum::{AsRefStr, EnumString};
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Rounding, SpanInfo, Spanned};
use zhang_shared::prices::PriceMap;

use crate::utils::id::FromSpan;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct OptionDomain {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Serialize, AsRefStr, EnumString)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum AccountStatus {
    Open,
    Close,
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

impl ErrorDomain {
    /// the error `kind` of the directive at `span`, with what else the load knows of it in `metas`
    pub fn new(kind: ErrorKind, span: &SpanInfo, metas: HashMap<String, String>) -> ErrorDomain {
        debug!("insert a new error [{}] [span: {:?}] [meta:{:?}]", kind, span, metas);
        ErrorDomain {
            id: Uuid::from_span(span).to_string(),
            error_type: kind,
            span: Some(span.clone()),
            metas,
        }
    }
}

/// The price map of the `price` directives of `directives`, in their order, each at its date in the ledger's timezone
/// `timezone`: what the budget check and the query engine's valuation convert with.
pub fn price_map<'a>(directives: impl IntoIterator<Item = &'a Spanned<Directive>>, timezone: &Tz) -> PriceMap {
    PriceMap::from_points(directives.into_iter().filter_map(|directive| match &directive.data {
        Directive::Price(price) => Some((
            price.date.to_timezone_datetime(timezone).date_naive(),
            &price.currency,
            &price.amount.commodity,
            price.amount.number.clone(),
        )),
        _ => None,
    }))
}
