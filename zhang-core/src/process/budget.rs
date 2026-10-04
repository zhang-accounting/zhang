use std::collections::HashMap;

use zhang_ast::error::ErrorKind;
use zhang_ast::{Budget, BudgetAdd, BudgetClose, BudgetTransfer, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::ZhangResult;

impl DirectiveProcess for Budget {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        if is_defined(ledger, &self.name) {
            ledger.operations().new_error(ErrorKind::DefineDuplicatedBudget, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        ledger
            .defined_budgets
            .as_mut()
            .expect("budget state is set during the store fold")
            .insert(self.name.clone());
        Ok(())
    }
}

impl DirectiveProcess for BudgetAdd {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(ledger, &self.name, span)
    }

    fn process(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        Ok(())
    }
}

impl DirectiveProcess for BudgetTransfer {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        Ok(budget_exists(ledger, &self.from, span)? && budget_exists(ledger, &self.to, span)?)
    }

    fn process(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        Ok(())
    }
}

impl DirectiveProcess for BudgetClose {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        budget_exists(ledger, &self.name, span)
    }

    fn process(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        Ok(())
    }
}

/// Only names are needed to validate the stream. Budget calculations belong to the query engine.
pub(super) fn is_defined(ledger: &Ledger, name: &str) -> bool {
    ledger
        .defined_budgets
        .as_ref()
        .expect("budget state is set during the store fold")
        .contains(name)
}

/// A directive using an undefined budget reports an error at its span.
fn budget_exists(ledger: &mut Ledger, name: &str, span: &SpanInfo) -> ZhangResult<bool> {
    if is_defined(ledger, name) {
        Ok(true)
    } else {
        ledger.operations().new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
        Ok(false)
    }
}
