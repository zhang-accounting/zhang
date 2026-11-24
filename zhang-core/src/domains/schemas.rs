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
    /// the error `kind` of the directive at `span`, with what else the load knows of it in `metas`. Its id is that of
    /// the first error of its kind there; [`identify_errors`] numbers the others once the load reported them all
    pub fn new(kind: ErrorKind, span: &SpanInfo, metas: HashMap<String, String>) -> ErrorDomain {
        debug!("insert a new error [{}] [span: {:?}] [meta:{:?}]", kind, span, metas);
        ErrorDomain {
            id: Uuid::of_error(span, &kind.to_string(), 0).to_string(),
            error_type: kind,
            span: Some(span.clone()),
            metas,
        }
    }
}

/// Give each of `errors`, the errors of a load in the order it reported them, its id: the `n`-th error of a kind at a
/// directive gets the `n`-th id of that kind there ([`FromSpan::of_error`]). No two errors of a load share an id, two
/// errors of one directive included (a plugin directive whose hosts and timeout were both reduced), and an error keeps
/// its id from one load of the same files to the next. An error of a transaction names the transaction in its
/// `txn_id` meta.
pub fn identify_errors(errors: &mut [ErrorDomain]) {
    let mut reported: HashMap<(Option<PathBuf>, usize, String), usize> = HashMap::new();
    for error in errors {
        let Some(span) = &error.span else { continue };
        let kind = error.error_type.to_string();
        let n = reported.entry((span.filename.clone(), span.start, kind.clone())).or_default();
        error.id = Uuid::of_error(span, &kind, *n).to_string();
        *n += 1;
    }
}

/// The price map of the `price` directives of `directives`, in their order, each at its date in the ledger's timezone
/// `timezone`: what the query engine's valuation, and `zhang_convert` for analyzers, convert with.
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
