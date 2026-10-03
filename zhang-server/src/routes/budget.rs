//! The budget pages: every budget as of a month, one budget as of a month, and what happened to a
//! budget in a month. Each endpoint runs [built-in queries](crate::builtin) (`budgets.*`) over
//! `#budgets`, `#budget_events` and the postings, and maps their rows into the response.

use axum::extract::{Path, Query, State};
use bigdecimal::BigDecimal;
use chrono::{NaiveDate, Utc};
use gotcha::api;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_core::store::BudgetEventType;
use zhang_query::{Params, Value};

use crate::builtin::execute;
use crate::cells::{first_row, rows, Row};
use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
use crate::response::{BudgetEventEntity, BudgetInfoEntity, BudgetIntervalEventEntity, BudgetListItemEntity, ResponseWrapper};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// The first day of the month a budget page asks for: by default the current one in the
/// ledger's timezone, as `today()` reads it.
fn requested_month(ledger: &Ledger, params: &BudgetListRequest) -> ServerResult<NaiveDate> {
    params.month_or(Utc::now().with_timezone(&ledger.options.timezone).date_naive())
}

/// A budget's figures in a month.
struct MonthFigures {
    closed: bool,
    assigned: Amount,
    activity: Amount,
    available: Amount,
}

impl MonthFigures {
    /// The figures of `month` from a row of `budgets.month` or `budgets.budget_month`: those of
    /// the budget's last month up to `month`. A budget whose last month is before `month` carries
    /// over, as `#budgets` does from month to month: the month starts with what was available and
    /// spends nothing.
    fn of(row: &Row<'_>, month: NaiveDate) -> MonthFigures {
        let currency = row.str("currency").unwrap_or_default();
        let amount = |name: &str| row.amount(name).unwrap_or_else(|| Amount::zero(&currency));
        let available = amount("available");
        let (assigned, activity) = if row.date("last_month").is_some_and(|last| last < month) {
            (available.clone(), Amount::new(BigDecimal::from(0), &currency))
        } else {
            (amount("assigned"), amount("activity"))
        };
        MonthFigures {
            closed: row.bool("closed").unwrap_or(false),
            assigned,
            activity,
            available,
        }
    }

    /// The figures of a month before the budget's first one: nothing assigned or spent, not closed.
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
        let budgets = rows(&result)
            .map(|row| {
                let figures = MonthFigures::of(&row, month);
                BudgetListItemEntity {
                    name: row.str("name").unwrap_or_default(),
                    alias: row.str("alias"),
                    category: row.str("category"),
                    closed: figures.closed,
                    assigned_amount: figures.assigned,
                    activity_amount: figures.activity,
                    available_amount: figures.available,
                }
            })
            .collect_vec();
        Ok(budgets)
    })
    .await?;
    ResponseWrapper::json(budgets)
}

/// One budget as of a month, by default the current one in the ledger's timezone, with the
/// accounts whose postings are its activity, in name order. Before the budget's first month
/// nothing is assigned or spent.
#[api(group = "budget")]
pub async fn get_budget_info(ledger: State<SharedLedger>, paths: Path<(String,)>, params: Query<BudgetListRequest>) -> ApiResult<BudgetInfoEntity> {
    let (budget_name,) = paths.0;
    let budget = with_ledger(&ledger, move |ledger| {
        let month = requested_month(ledger, &params)?;
        let budget = execute(ledger, "budgets.budget", &Params::new().bind("name", budget_name.as_str()), false)?;
        let Some(budget) = first_row(&budget) else {
            return Ok(None);
        };
        let params = Params::new().bind("name", budget_name.as_str()).bind("month", month);
        let figures = execute(ledger, "budgets.budget_month", &params, false)?;
        let figures = match first_row(&figures) {
            Some(row) => MonthFigures::of(&row, month),
            None => MonthFigures::before_start(&budget.str("currency").unwrap_or_default()),
        };
        Ok(Some(BudgetInfoEntity {
            name: budget.str("name").unwrap_or(budget_name),
            alias: budget.str("alias"),
            category: budget.str("category"),
            closed: figures.closed,
            related_accounts: budget.set("accounts").unwrap_or_default().into_iter().collect_vec(),
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
/// `budget-transfer` directives put in, and the postings of its accounts, with their times in
/// the ledger's timezone.
#[api(group = "budget")]
pub async fn get_budget_interval_detail(ledger: State<SharedLedger>, paths: Path<BudgetIntervalDetailRequest>) -> ApiResult<Vec<BudgetIntervalEventEntity>> {
    let BudgetIntervalDetailRequest { budget_name, year, month } = paths.0;
    let month = BudgetListRequest::month_of(year, month)?;
    let detail = with_ledger(&ledger, move |ledger| {
        let budget = execute(ledger, "budgets.budget", &Params::new().bind("name", budget_name.as_str()), false)?;
        let Some(budget) = first_row(&budget) else {
            return Ok(None);
        };
        let accounts = budget.set("accounts").unwrap_or_default();

        let params = Params::new().bind("name", budget_name.as_str()).bind("month", month);
        let events = execute(ledger, "budgets.events", &params, false)?;
        let events = rows(&events)
            .map(|row| {
                let event_type = match row.str("type").as_deref() {
                    Some("assign") => BudgetEventType::AddAssignedAmount,
                    _ => BudgetEventType::Transfer,
                };
                BudgetIntervalEventEntity::BudgetEvent(BudgetEventEntity {
                    timestamp: row.int("timestamp").unwrap_or_default(),
                    amount: row.amount("amount").unwrap_or_else(|| Amount::zero("")),
                    event_type,
                })
            })
            .collect_vec();
        let params = Params::new().bind("accounts", Value::Set(accounts)).bind("month", month);
        let postings = execute(ledger, "budgets.postings", &params, false)?;
        let postings = rows(&postings)
            .map(|row| {
                let units = row.amount("units").unwrap_or_else(|| Amount::zero(""));
                BudgetIntervalEventEntity::Posting(AccountJournalDomain {
                    datetime: row.datetime("date", "time").unwrap_or_default(),
                    timestamp: row.int("timestamp").unwrap_or_default(),
                    account: row.str("account").unwrap_or_default(),
                    trx_id: row.str("id").unwrap_or_default(),
                    payee: row.str("payee"),
                    narration: row.str("narration").filter(|it| !it.is_empty()),
                    account_after: row.amount("balance").unwrap_or_else(|| Amount::zero(&units.commodity)),
                    inferred_unit: units,
                    asserted: None,
                    checked_balance: None,
                    passed: None,
                })
            })
            .collect_vec();
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

/// The handlers as they were before they ran built-in queries, kept to compare the two until they
/// are removed (#479).
#[cfg(test)]
pub(crate) mod legacy {
    use std::cmp::Reverse;
    use std::ops::Sub;

    use axum::extract::{Path, Query, State};
    use chrono::{Months, NaiveDate, NaiveTime, TimeDelta};
    use itertools::Itertools;
    use zhang_ast::amount::Amount;
    use zhang_ast::resolve_local_datetime;
    use zhang_core::store::BudgetIntervalDetail;

    use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
    use crate::response::{BudgetInfoEntity, BudgetIntervalEventEntity, BudgetListItemEntity, ResponseWrapper};
    use crate::state::SharedLedger;
    use crate::ApiResult;

    pub async fn get_budget_list(ledger: State<SharedLedger>, params: Query<BudgetListRequest>) -> ApiResult<Vec<BudgetListItemEntity>> {
        let interval = params.as_interval();

        let ledger = ledger.read().await;
        let operations = ledger.operations();

        let mut ret = vec![];
        for budget in operations.all_budgets()? {
            if let Some(interval_detail) = operations.budget_month_detail(&budget.name, interval)? {
                ret.push(BudgetListItemEntity {
                    name: budget.name,
                    alias: budget.alias,
                    category: budget.category,
                    closed: budget.closed,
                    available_amount: interval_detail.assigned_amount.sub(interval_detail.activity_amount.number.clone()),
                    assigned_amount: interval_detail.assigned_amount,
                    activity_amount: interval_detail.activity_amount,
                });
            }
        }
        ResponseWrapper::json(ret)
    }

    pub async fn get_budget_info(ledger: State<SharedLedger>, paths: Path<(String,)>, params: Query<BudgetListRequest>) -> ApiResult<BudgetInfoEntity> {
        let (budget_name,) = paths.0;
        let ledger = ledger.read().await;
        let operations = ledger.operations();

        let Some(budget) = operations.all_budgets()?.into_iter().find(|budget| budget.name.eq(&budget_name)) else {
            return ResponseWrapper::not_found();
        };
        let interval = params.as_interval();
        let interval_detail = operations.budget_month_detail(&budget.name, interval)?.unwrap_or(BudgetIntervalDetail {
            date: interval,
            events: vec![],
            assigned_amount: Amount::zero(&budget.commodity),
            activity_amount: Amount::zero(&budget.commodity),
        });
        let store = operations.store.read().unwrap();
        let related_accounts = store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq("AccountMeta"))
            .filter(|meta| meta.key.eq("budget"))
            .filter(|meta| meta.value.eq(&budget_name))
            .map(|meta| meta.type_identifier.clone())
            .collect_vec();
        ResponseWrapper::json(BudgetInfoEntity {
            name: budget.name,
            alias: budget.alias,
            category: budget.category,
            closed: budget.closed,
            related_accounts,
            available_amount: interval_detail.assigned_amount.sub(interval_detail.activity_amount.number.clone()),
            assigned_amount: interval_detail.assigned_amount,
            activity_amount: interval_detail.activity_amount,
        })
    }

    pub async fn get_budget_interval_detail(
        ledger: State<SharedLedger>, paths: Path<BudgetIntervalDetailRequest>,
    ) -> ApiResult<Vec<BudgetIntervalEventEntity>> {
        let BudgetIntervalDetailRequest { budget_name, year, month } = paths.0;
        let ledger = ledger.read().await;
        let operations = ledger.operations();

        if !operations.all_budgets()?.into_iter().any(|budget| budget.name.eq(&budget_name)) {
            return ResponseWrapper::not_found();
        };
        let timezone = &ledger.options.timezone;
        let first_day = NaiveDate::from_ymd_opt(year as i32, month, 1).unwrap();
        let month_beginning = resolve_local_datetime(timezone, &first_day.and_time(NaiveTime::MIN));
        let next_month_beginning = resolve_local_datetime(timezone, &(first_day + Months::new(1)).and_time(NaiveTime::MIN));
        let month_end = next_month_beginning - TimeDelta::nanoseconds(1);
        let interval = year * 100 + month;
        let budget_events = operations
            .budget_month_detail(&budget_name, interval)?
            .map(|interval| interval.events)
            .unwrap_or_default()
            .into_iter()
            .map(|event| BudgetIntervalEventEntity::BudgetEvent(event.into()))
            .collect_vec();

        let store = operations.store.read().unwrap();
        let related_accounts = store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq("AccountMeta"))
            .filter(|meta| meta.key.eq("budget"))
            .filter(|meta| meta.value.eq(&budget_name))
            .map(|meta| meta.type_identifier.clone())
            .collect_vec();
        let journals = operations
            .accounts_dated_journals(&related_accounts, month_beginning, month_end)?
            .into_iter()
            .map(BudgetIntervalEventEntity::Posting)
            .collect_vec();
        let mut ret = vec![];
        ret.extend(budget_events);
        ret.extend(journals);
        ret.sort_by_key(|a| Reverse(a.naive_datetime()));
        ResponseWrapper::json(ret)
    }
}
