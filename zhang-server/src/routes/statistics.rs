//! The report endpoints. Each runs the report's built-in queries ([`crate::report`]) under the
//! ledger's read lock, off the async workers.

use std::str::FromStr;

use axum::extract::{Path, Query, State};
use gotcha::api;
use zhang_ast::AccountType;

use crate::builtin::LedgerDateRange;
use crate::error::ServerError;
use crate::report::GraphLimits;
use crate::request::{StatisticGraphRequest, StatisticRequest};
use crate::response::{ResponseWrapper, StatisticGraphEntity, StatisticRankEntity, StatisticSummaryEntity};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::{report, ApiResult, ServerResult};

/// The net worth and the liabilities at the end of the range, and the income, the expenses and
/// the number of transactions of the range (built-in queries `report.net_worth`,
/// `report.liabilities`, `report.flows` and `report.transaction_count`).
///
/// `from` and `to` are ledger dates (`YYYY-MM-DD`), both inclusive.
#[api(group = "statistic")]
pub async fn get_statistic_summary(ledger: State<SharedLedger>, Query(params): Query<StatisticRequest>) -> ApiResult<StatisticSummaryEntity> {
    let summary = with_ledger(&ledger.0 .0, move |ledger| {
        let range = LedgerDateRange::from_query(&params.from, &params.to, &ledger.options.timezone)?;
        report::summary(ledger, &range)
    })
    .await?;
    ResponseWrapper::json(summary)
}

/// The net worth at the end of every day, week or month of the range, and what each account
/// type changed by in it, keyed by the bucket's first day (built-in queries
/// `report.net_worth_trend`, `report.net_worth` and `report.changes`).
///
/// `from` and `to` are ledger dates (`YYYY-MM-DD`), both inclusive.
#[api(group = "statistic")]
pub async fn get_statistic_graph(ledger: State<SharedLedger>, Query(params): Query<StatisticGraphRequest>) -> ApiResult<StatisticGraphEntity> {
    // the queries run under the ledger's read lock; the points are built after it is released
    let rows = with_ledger(&ledger.0 .0, move |ledger| {
        let range = LedgerDateRange::from_query(&params.from, &params.to, &ledger.options.timezone)?;
        report::graph_rows(ledger, &range, &params.interval, GraphLimits::server())
    })
    .await?;
    let graph = tokio::task::spawn_blocking(move || rows.build()).await??;
    ResponseWrapper::json(graph)
}

/// What every account of the type changed by in the range, and its ten largest postings by
/// value (built-in queries `report.account_totals` and `report.top_postings`).
///
/// `from` and `to` are ledger dates (`YYYY-MM-DD`), both inclusive.
#[api(group = "statistic")]
pub async fn get_statistic_rank_detail_by_account_type(
    ledger: State<SharedLedger>, paths: Path<(String,)>, Query(params): Query<StatisticRequest>,
) -> ApiResult<StatisticRankEntity> {
    let account_type = account_type(&paths.0 .0)?;
    let rank = with_ledger(&ledger.0 .0, move |ledger| {
        let range = LedgerDateRange::from_query(&params.from, &params.to, &ledger.options.timezone)?;
        report::rank(ledger, account_type, &range)
    })
    .await?;
    ResponseWrapper::json(rank)
}

/// The account type of the path, such as `Expenses`; anything else is a 400.
fn account_type(name: &str) -> ServerResult<AccountType> {
    AccountType::from_str(name).map_err(|_| {
        ServerError::InvalidInput(format!(
            "unknown account type {:?}: expected Assets, Liabilities, Equity, Income or Expenses",
            name
        ))
    })
}
