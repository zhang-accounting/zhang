use std::collections::HashMap;

use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Budget, BudgetAdd, BudgetClose, BudgetTransfer, Date, Directive, SpanInfo, Spanned};

use crate::domains::schemas::price_map;
use crate::ledger::Ledger;
use crate::utils::hashmap::HashMapOfExt;

/// define the budget of `budget`, unless one of its name is defined already
pub(crate) fn define(budget: &Budget, ledger: &mut Ledger, span: &SpanInfo) {
    if defined(ledger, &budget.name).is_some() {
        ledger.report(ErrorKind::DefineDuplicatedBudget, span, HashMap::default());
        return;
    }
    let defined = DefinedBudget {
        commodity: budget.commodity.clone(),
        close: None,
    };
    ledger
        .defined_budgets
        .as_mut()
        .expect("budget state is set during the fold")
        .insert(budget.name.clone(), defined);
}

/// check a `budget-add` of a defined budget
pub(crate) fn add(add: &BudgetAdd, ledger: &mut Ledger, span: &SpanInfo) {
    if budget_exists(ledger, &add.name, span) {
        keep_foreign_amount(ledger, &add.name, &add.amount, None, add.date.naive_date(), span, None);
    }
}

/// check a `budget-transfer` between defined budgets
pub(crate) fn transfer(transfer: &BudgetTransfer, ledger: &mut Ledger, span: &SpanInfo) {
    if budget_exists(ledger, &transfer.from, span) && budget_exists(ledger, &transfer.to, span) {
        for name in [&transfer.from, &transfer.to] {
            keep_foreign_amount(ledger, name, &transfer.amount, None, transfer.date.naive_date(), span, None);
        }
    }
}

/// close a defined budget: a budget closes with its first `budget-close`
pub(crate) fn close(close: &BudgetClose, ledger: &mut Ledger, span: &SpanInfo) {
    if !budget_exists(ledger, &close.name, span) {
        return;
    }
    if let Some(budget) = ledger.defined_budgets.as_mut().and_then(|budgets| budgets.get_mut(&close.name)) {
        budget.close.get_or_insert_with(|| close.date.clone());
    }
}

/// What the fold knows of a defined budget, solely to validate the stream. Budget figures
/// belong to the query engine.
#[derive(Debug)]
pub(crate) struct DefinedBudget {
    pub(crate) commodity: String,
    /// the date of its first `budget-close`: it takes no activity after it (see
    /// [`Date::close_precedes`])
    pub(crate) close: Option<Date>,
}

/// An amount of a budget in another commodity than the budget's. The query engine converts it
/// to the budget's commodity at its date; whether a price does is checked after the fold, once
/// every price is known.
#[derive(Debug)]
pub(crate) struct ForeignAmount {
    span: SpanInfo,
    date: NaiveDate,
    commodity: String,
    /// the cost currency of a posting held at cost, through which a conversion may go
    via: Option<String>,
    budget: String,
    budget_commodity: String,
    /// the account of a posting
    account: Option<String>,
}

/// The budget `name`, if the stream has defined it so far.
pub(super) fn defined<'a>(ledger: &'a Ledger, name: &str) -> Option<&'a DefinedBudget> {
    ledger.defined_budgets.as_ref().expect("budget state is set during the fold").get(name)
}

/// A directive using an undefined budget reports an error at its span.
fn budget_exists(ledger: &mut Ledger, name: &str, span: &SpanInfo) -> bool {
    let exists = defined(ledger, name).is_some();
    if !exists {
        ledger.report(ErrorKind::BudgetDoesNotExist, span, HashMap::of("budget_name", name));
    }
    exists
}

/// Keep `amount` of the budget `name` for [`report_unconverted_amounts`] when it is in another
/// commodity than the budget's: the units of a posting of `account`, held at a cost in `via` if
/// any, or the amount of a budget directive.
pub(super) fn keep_foreign_amount(
    ledger: &mut Ledger, name: &str, amount: &Amount, via: Option<&str>, date: NaiveDate, span: &SpanInfo, account: Option<&str>,
) {
    let Some(budget) = defined(ledger, name) else { return };
    if amount.commodity == budget.commodity || amount.is_zero() {
        return;
    }
    let foreign = ForeignAmount {
        span: span.clone(),
        date,
        commodity: amount.commodity.clone(),
        via: via.map(str::to_owned),
        budget: name.to_owned(),
        budget_commodity: budget.commodity.clone(),
        account: account.map(str::to_owned),
    };
    ledger.foreign_budget_amounts.push(foreign);
}

/// Report the amounts [`keep_foreign_amount`] kept that no price converts to their budget's
/// commodity at their date, as the query engine converts them
/// ([`zhang_shared::prices::PriceMap::conversion`]): the engine leaves them out of the budget instead of adding them
/// as numbers of another commodity. `directives` are those of the stream being folded, every price among them.
pub(crate) fn report_unconverted_amounts(ledger: &mut Ledger, directives: &[Spanned<Directive>]) {
    let amounts = std::mem::take(&mut ledger.foreign_budget_amounts);
    if amounts.is_empty() {
        return;
    }
    let prices = price_map(directives, &ledger.options.timezone);
    for amount in amounts {
        if prices
            .conversion(&amount.commodity, &amount.budget_commodity, amount.via.as_deref(), Some(amount.date))
            .is_some()
        {
            continue;
        }
        let mut metas = HashMap::from([
            ("budget_name".to_owned(), amount.budget),
            ("commodity".to_owned(), amount.commodity),
            ("budget_commodity".to_owned(), amount.budget_commodity),
        ]);
        if let Some(account) = amount.account {
            metas.insert("account_name".to_owned(), account);
        }
        ledger.report(ErrorKind::BudgetCommodityMismatch, &amount.span, metas);
    }
}
