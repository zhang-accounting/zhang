//! The budget pages: every budget as of a month, one budget as of a month, and what happened to a
//! budget in a month. Each endpoint runs [built-in queries](crate::builtin) (`budgets.*`) over
//! `#budgets`, `#budget_events` and the postings, and maps their rows into the response.

use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use gotcha::api;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_core::ledger::Ledger;
use zhang_query::Params;

use crate::builtin::execute;
use crate::cells::{first_row, rows, Row};
use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
use crate::response::{
    AccountJournalEntity, BudgetEventEntity, BudgetEventType, BudgetInfoEntity, BudgetIntervalEventEntity, BudgetListItemEntity, ResponseWrapper,
};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// The first day of the month a budget page asks for: by default the current one in the
/// ledger's timezone, by the ledger's clock, as `today()` reads it.
fn requested_month(ledger: &Ledger, params: &BudgetListRequest) -> ServerResult<NaiveDate> {
    params.month_or(ledger.today())
}

/// A budget's figures in a month.
struct MonthFigures {
    closed: bool,
    assigned: Amount,
    activity: Amount,
    available: Amount,
}

impl MonthFigures {
    /// The figures of a row of `budgets.month` or `budgets.budget_month`, as the query gives them
    /// (it carries a budget over to a month after its last row).
    fn from_row(row: &Row<'_>) -> ServerResult<MonthFigures> {
        let currency = row.str("currency")?.unwrap_or_default();
        let amount = |name: &str| -> ServerResult<Amount> { Ok(row.amount(name)?.unwrap_or_else(|| Amount::zero(&currency))) };
        Ok(MonthFigures {
            closed: row.bool("closed")?.unwrap_or(false),
            assigned: amount("assigned")?,
            // a number in the budget's currency, `0` in a month the query carries the budget over to
            activity: Amount::new(row.decimal("activity")?.unwrap_or_default(), &currency),
            available: amount("available")?,
        })
    }

    /// The figures of a month before the budget's first one, which has no row: nothing assigned
    /// or spent, not closed.
    fn before_start(currency: &str) -> MonthFigures {
        MonthFigures {
            closed: false,
            assigned: Amount::zero(currency),
            activity: Amount::zero(currency),
            available: Amount::zero(currency),
        }
    }
}

/// Every budget as of a month, by default the current one in the ledger's timezone, ordered by
/// name. Budgets that start after the month are not listed.
///
/// The figures are those of `#budgets`: activity is converted to the budget's commodity at each
/// posting's date, and `closed` is whether the budget was closed in or before the month.
#[api(group = "budget")]
pub async fn get_budget_list(ledger: State<SharedLedger>, params: Query<BudgetListRequest>) -> ApiResult<Vec<BudgetListItemEntity>> {
    let budgets = with_ledger(&ledger, move |ledger| {
        let month = requested_month(ledger, &params)?;
        let result = execute(ledger, "budgets.month", &Params::new().bind("month", month), false)?;
        rows("budgets.month", &result)
            .map(|row| {
                let figures = MonthFigures::from_row(&row)?;
                Ok(BudgetListItemEntity {
                    name: row.str("name")?.unwrap_or_default(),
                    alias: row.str("alias")?,
                    category: row.str("category")?,
                    closed: figures.closed,
                    assigned_amount: figures.assigned,
                    activity_amount: figures.activity,
                    available_amount: figures.available,
                })
            })
            .collect()
    })
    .await?;
    ResponseWrapper::json(budgets)
}

/// One budget as of a month, by default the current one in the ledger's timezone, with the
/// accounts whose postings are its activity, in name order, and the date and time of its close.
/// Before the budget's first month nothing is assigned or spent.
#[api(group = "budget")]
pub async fn get_budget_info(ledger: State<SharedLedger>, paths: Path<(String,)>, params: Query<BudgetListRequest>) -> ApiResult<BudgetInfoEntity> {
    let (budget_name,) = paths.0;
    let budget = with_ledger(&ledger, move |ledger| {
        let month = requested_month(ledger, &params)?;
        let budget = execute(ledger, "budgets.budget", &Params::new().bind("name", budget_name.as_str()), false)?;
        let Some(budget) = first_row("budgets.budget", &budget) else {
            return Ok(None);
        };
        let params = Params::new().bind("name", budget_name.as_str()).bind("month", month);
        let figures = execute(ledger, "budgets.budget_month", &params, false)?;
        let figures = match first_row("budgets.budget_month", &figures) {
            Some(row) => MonthFigures::from_row(&row)?,
            None => MonthFigures::before_start(&budget.str("currency")?.unwrap_or_default()),
        };
        Ok(Some(BudgetInfoEntity {
            name: budget.str("name")?.unwrap_or(budget_name),
            alias: budget.str("alias")?,
            category: budget.str("category")?,
            closed: figures.closed,
            close: budget.date("close")?,
            close_time: budget.str("close_time")?,
            related_accounts: budget.set("accounts")?.unwrap_or_default().into_iter().collect_vec(),
            assigned_amount: figures.assigned,
            activity_amount: figures.activity,
            available_amount: figures.available,
        }))
    })
    .await?;
    match budget {
        Some(budget) => ResponseWrapper::json(budget),
        None => ResponseWrapper::not_found(),
    }
}

/// What happened to a budget in a month, newest first: what its `budget-add` and
/// `budget-transfer` directives put in, and the postings that count toward it (they add up to the
/// month's activity: none before its definition, after its close or in a commodity no price
/// converts), with their times in the ledger's timezone.
#[api(group = "budget")]
pub async fn get_budget_interval_detail(ledger: State<SharedLedger>, paths: Path<BudgetIntervalDetailRequest>) -> ApiResult<Vec<BudgetIntervalEventEntity>> {
    let BudgetIntervalDetailRequest { budget_name, year, month } = paths.0;
    let month = BudgetListRequest::month_of(year, month)?;
    let detail = with_ledger(&ledger, move |ledger| {
        // an unknown budget is a 404
        let budget = execute(ledger, "budgets.budget", &Params::new().bind("name", budget_name.as_str()), false)?;
        if first_row("budgets.budget", &budget).is_none() {
            return Ok(None);
        }
        let params = Params::new().bind("name", budget_name.as_str()).bind("month", month);
        let events = execute(ledger, "budgets.events", &params, false)?;
        let events = rows("budgets.events", &events)
            .map(|row| {
                let event_type = match row.str("type")?.as_deref() {
                    Some("assign") => BudgetEventType::AddAssignedAmount,
                    _ => BudgetEventType::Transfer,
                };
                Ok(BudgetIntervalEventEntity::BudgetEvent(BudgetEventEntity {
                    timestamp: row.int("timestamp")?.unwrap_or_default(),
                    amount: row.amount("amount")?.unwrap_or_else(|| Amount::zero("")),
                    event_type,
                }))
            })
            .collect::<ServerResult<Vec<_>>>()?;
        // the postings that count toward the budget, so they add up to the month's activity
        let postings = execute(ledger, "budgets.postings", &params, false)?;
        let postings = rows("budgets.postings", &postings)
            .map(|row| {
                let units = row.amount("units")?.unwrap_or_else(|| Amount::zero(""));
                Ok(BudgetIntervalEventEntity::Posting(AccountJournalEntity {
                    datetime: row.datetime("date", "time")?.unwrap_or_default(),
                    timestamp: row.int("timestamp")?.unwrap_or_default(),
                    account: row.str("account")?.unwrap_or_default(),
                    trx_id: row.str("id")?.unwrap_or_default(),
                    payee: row.str("payee")?,
                    narration: row.str("narration")?.filter(|it| !it.is_empty()),
                    account_after: row.amount("balance")?.unwrap_or_else(|| Amount::zero(&units.commodity)),
                    inferred_unit: units,
                    asserted: None,
                    checked_balance: None,
                    passed: None,
                }))
            })
            .collect::<ServerResult<Vec<_>>>()?;
        // both lists are newest first; of the same time, the budget's own entries come first
        Ok(Some(
            events
                .into_iter()
                .merge_by(postings, |event, posting| event.timestamp() >= posting.timestamp())
                .collect_vec(),
        ))
    })
    .await?;
    match detail {
        Some(detail) => ResponseWrapper::json(detail),
        None => ResponseWrapper::not_found(),
    }
}
