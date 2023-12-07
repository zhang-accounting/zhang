use std::ops::Sub;
use std::sync::Arc;

use actix_web::get;
use actix_web::web::{Data, Path, Query};
use chrono::{Datelike, Local};
use itertools::Itertools;
use tokio::sync::RwLock;

use zhang_ast::amount::Amount;
use zhang_core::ledger::Ledger;
use zhang_core::store::BudgetIntervalDetail;

use crate::request::BudgetListRequest;
use crate::response::{BudgetInfoResponse, BudgetListItemResponse, ResponseWrapper};
use crate::ApiResult;

#[get("/api/budgets")]
pub async fn get_budget_list(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<BudgetListRequest>) -> ApiResult<Vec<BudgetListItemResponse>> {
    let interval = params.as_interval();

    let ledger = ledger.read().await;
    let operations = ledger.operations();

    let mut ret = vec![];
    for budget in operations.all_budgets()? {
        if let Some(interval_detail) = operations.budget_month_detail(&budget.name, interval)? {
            ret.push(BudgetListItemResponse {
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

#[get("/api/budgets/{budget_name}")]
pub async fn get_budget_info(ledger: Data<Arc<RwLock<Ledger>>>, paths: Path<(String,)>, params: Query<BudgetListRequest>) -> ApiResult<BudgetInfoResponse> {
    let (budget_name,) = paths.into_inner();
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    let Some(budget) = operations.all_budgets()?.into_iter().filter(|budget| budget.name.eq(&budget_name)).next() else {
        return ResponseWrapper::not_found();
    };
    let interval = params.as_interval();
    let interval_detail = operations.budget_month_detail(&budget.name, interval)?.unwrap_or(BudgetIntervalDetail {
        date: interval,
        assigned_amount: Amount::zero(&budget.commodity),
        activity_amount: Amount::zero(&budget.commodity),
    });
    let store = operations.store.read().unwrap();
    let related_accounts = store
        .metas
        .iter()
        .filter(|meta| meta.meta_type.eq("AccountMeta"))
        .filter(|meta| meta.key.eq("budget"))
        .map(|meta| meta.type_identifier.clone())
        .collect_vec();
    ResponseWrapper::json(BudgetInfoResponse {
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
