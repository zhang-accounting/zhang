use std::ops::Sub;
use std::sync::Arc;

use actix_web::get;
use actix_web::web::{Data, Query};
use tokio::sync::RwLock;

use zhang_core::ledger::Ledger;

use crate::request::BudgetListRequest;
use crate::response::{BudgetListItemResponse, ResponseWrapper};
use crate::ApiResult;

#[get("/api/budgets")]
pub async fn get_budget_list(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<BudgetListRequest>) -> ApiResult<Vec<BudgetListItemResponse>> {
    let interval = params.year * 100 + params.month;

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
