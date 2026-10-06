//! The report's graph endpoint, which runs the report's built-in queries ([`crate::report`]) under
//! the ledger's read lock, off the async workers. The summary and the ranks are the pages' own
//! built-in queries (`report.*`).

use axum::extract::{Query, State};
use gotcha::api;

use crate::builtin::LedgerDateRange;
use crate::report::GraphLimits;
use crate::request::StatisticGraphRequest;
use crate::response::{ResponseWrapper, StatisticGraphEntity};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::{report, ApiResult};

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
