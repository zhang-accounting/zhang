use std::collections::HashMap;

use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Budget, BudgetAdd, BudgetClose, BudgetTransfer, Date, SpanInfo};

use crate::domains::schemas::PriceDomain;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::utils::hashmap::HashMapOfExt;
use crate::ZhangResult;

impl DirectiveProcess for Budget {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        if defined(ledger, &self.name).is_some() {
            ledger.operations().new_error(ErrorKind::DefineDuplicatedBudget, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let budget = DefinedBudget {
            commodity: self.commodity.clone(),
            close: None,
        };
        ledger
            .defined_budgets
            .as_mut()
            .expect("budget state is set during the store fold")
            .insert(self.name.clone(), budget);
        Ok(())
    }
}

impl DirectiveProcess for BudgetAdd {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(ledger, &self.name, span)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        keep_foreign_amount(ledger, &self.name, &self.amount, None, self.date.naive_date(), span, None);
        Ok(())
    }
}

impl DirectiveProcess for BudgetTransfer {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        Ok(budget_exists(ledger, &self.from, span)? && budget_exists(ledger, &self.to, span)?)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        for name in [&self.from, &self.to] {
            keep_foreign_amount(ledger, name, &self.amount, None, self.date.naive_date(), span, None);
        }
        Ok(())
    }
}

impl DirectiveProcess for BudgetClose {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(ledger, &self.name, span)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        // a budget closes with its first budget-close
        if let Some(budget) = ledger.defined_budgets.as_mut().and_then(|budgets| budgets.get_mut(&self.name)) {
            budget.close.get_or_insert_with(|| self.date.clone());
        }
        Ok(())
    }
}

/// What the store fold knows of a defined budget, solely to validate the stream. Budget figures
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
    ledger.defined_budgets.as_ref().expect("budget state is set during the store fold").get(name)
}

/// A directive using an undefined budget reports an error at its span.
fn budget_exists(ledger: &mut Ledger, name: &str, span: &SpanInfo) -> ZhangResult<bool> {
    if defined(ledger, name).is_some() {
        Ok(true)
    } else {
        ledger
            .operations()
            .new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::of("budget_name", name))?;
        Ok(false)
    }
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
/// as numbers of another commodity.
pub(crate) fn report_unconverted_amounts(ledger: &mut Ledger) -> ZhangResult<()> {
    let amounts = std::mem::take(&mut ledger.foreign_budget_amounts);
    if amounts.is_empty() {
        return Ok(());
    }
    let mut operations = ledger.operations();
    let prices = PriceDomain::price_map(&operations.read().prices);
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
        operations.new_error(ErrorKind::BudgetCommodityMismatch, &amount.span, metas)?;
    }
    Ok(())
}
