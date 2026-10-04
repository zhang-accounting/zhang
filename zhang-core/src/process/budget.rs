use std::collections::HashMap;

use zhang_ast::error::ErrorKind;
use zhang_ast::{Budget, BudgetAdd, BudgetClose, BudgetTransfer, SpanInfo};

use crate::domains::Operations;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::BudgetEventType;
use crate::ZhangResult;

impl DirectiveProcess for Budget {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        if operations.contains_budget(&self.name) {
            operations.new_error(ErrorKind::DefineDuplicatedBudget, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.init_budget(
            &self.name,
            &self.commodity,
            self.date.to_timezone_datetime(&ledger.options.timezone),
            self.meta.get_one("alias").map(|it| it.as_str().to_owned()),
            self.meta.get_one("category").map(|it| it.as_str().to_owned()),
        )?;
        Ok(())
    }
}

impl DirectiveProcess for BudgetAdd {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(&mut ledger.operations(), &self.name, span)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.budget_add_assigned_amount(
            &self.name,
            self.date.to_timezone_datetime(&ledger.options.timezone),
            BudgetEventType::AddAssignedAmount,
            self.amount.clone(),
        )?;

        Ok(())
    }
}

impl DirectiveProcess for BudgetTransfer {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        Ok(budget_exists(&mut operations, &self.from, span)? && budget_exists(&mut operations, &self.to, span)?)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.budget_transfer(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.from,
            &self.to,
            self.amount.clone(),
        )?;
        Ok(())
    }
}

impl DirectiveProcess for BudgetClose {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(&mut ledger.operations(), &self.name, span)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        // todo: check if budget is empty

        operations.budget_close(&self.name, self.date.clone())?;
        Ok(())
    }
}

/// whether the budget `name` exists; a [`ErrorKind::BudgetDoesNotExist`] error is recorded at `span` when it does not
fn budget_exists(operations: &mut Operations, name: &str, span: &SpanInfo) -> ZhangResult<bool> {
    if operations.contains_budget(name) {
        Ok(true)
    } else {
        operations.new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
        Ok(false)
    }
}
