use std::cmp::Reverse;
use std::ops::Sub;

use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use gotcha::api;
use itertools::Itertools;
use now::DateTimeNow;
use zhang_ast::amount::Amount;
use zhang_core::store::BudgetIntervalDetail;

use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
use crate::response::{BudgetInfoEntity, BudgetIntervalEventEntity, BudgetListItemEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::ApiResult;

#[api(group = "budget")]
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
                available_amount: interval_detail.assigned_amount.sub(interval_detail.activity_amount.number.clone()).into(),
                assigned_amount: interval_detail.assigned_amount.into(),
                activity_amount: interval_detail.activity_amount.into(),
            });
        }
    }
    ResponseWrapper::json(ret)
}

#[api(group = "budget")]
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
        available_amount: interval_detail.assigned_amount.sub(interval_detail.activity_amount.number.clone()).into(),
        assigned_amount: interval_detail.assigned_amount.into(),
        activity_amount: interval_detail.activity_amount.into(),
    })
}

#[api(group = "budget")]
pub async fn get_budget_interval_detail(ledger: State<SharedLedger>, paths: Path<BudgetIntervalDetailRequest>) -> ApiResult<Vec<BudgetIntervalEventEntity>> {
    let BudgetIntervalDetailRequest { budget_name, year, month } = paths.0;
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    if !operations.all_budgets()?.into_iter().any(|budget| budget.name.eq(&budget_name)) {
        return ResponseWrapper::not_found();
    };
    let date = NaiveDate::from_ymd_opt(year as i32, month, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
    let datetime = date.and_local_timezone(ledger.options.timezone).unwrap();
    let month_beginning = datetime.beginning_of_month();
    let month_end = datetime.end_of_month();
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
        .map(|journal| BudgetIntervalEventEntity::Posting(journal.into()))
        .collect_vec();
    let mut ret = vec![];
    ret.extend(budget_events);
    ret.extend(journals);
    ret.sort_by_key(|a| Reverse(a.naive_datetime()));
    ResponseWrapper::json(ret)
}
