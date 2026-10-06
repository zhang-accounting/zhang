use std::collections::HashMap;

use log::trace;
use zhang_ast::error::ErrorKind;
use zhang_ast::{SpanInfo, Transaction};

use crate::booking::{group_units, written_groups};
use crate::ledger::Ledger;
use crate::utils::hashmap::HashMapOfExt;

/// check the budgets of the postings of `txn`, a transaction the load accepted
pub(crate) fn check_budgets(txn: &Transaction, ledger: &mut Ledger, span: &SpanInfo) {
    trace!("[fold] transaction: {:?}", txn);
    let datetime = txn.date.to_timezone_datetime(&ledger.options.timezone);

    // the budgets of each posting as written: the legs booking split from it are summed into it
    for group in written_groups(&txn.postings) {
        let legs = group.legs;
        let posting = &legs[0];
        let units = group_units(legs);
        // the cost currency of the booked lot, through which a budget may convert the units
        let via = posting.cost.as_ref().and_then(|cost| cost.base.as_ref()).map(|cost| cost.commodity.clone());

        // budget related: the posting belongs to the budgets of the account's `open` in effect
        // at its date and time as stored, the rule the query engine counts and lists it with
        // (`Ledger::account_budgets`). Like `budget-add`, activity on a budget the stream has
        // not defined (yet) is skipped, and so is activity after the budget's close. Each is
        // reported once per (account, budget), on the first transaction that loses activity,
        // instead of once per posting
        let budgets_name = ledger
            .account_budgets(posting.account.name(), datetime.naive_local())
            .cloned()
            .unwrap_or_default();
        for budget in budgets_name {
            let account_name = posting.account.name().to_owned();
            let Some(defined) = super::budget::defined(ledger, &budget) else {
                if ledger.reported_undefined_budgets.insert((account_name.clone(), budget.clone())) {
                    let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                    ledger.report(ErrorKind::BudgetDoesNotExist, span, metas);
                }
                continue;
            };
            if defined.close.as_ref().is_some_and(|close| close.close_precedes(datetime.naive_local())) {
                if ledger.reported_closed_budgets.insert((account_name.clone(), budget.clone())) {
                    let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                    ledger.report(ErrorKind::BudgetClosed, span, metas);
                }
                continue;
            }
            super::budget::keep_foreign_amount(ledger, &budget, &units, via.as_deref(), datetime.date_naive(), span, Some(&account_name));
        }
    }
}
