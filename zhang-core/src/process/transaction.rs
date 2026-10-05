use std::collections::HashMap;
use std::sync::atomic::Ordering;

use itertools::Itertools;
use log::trace;
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Flag, SpanInfo, Transaction};

use crate::booking::{group_units, written_groups};
use crate::domains::schemas::MetaType;
use crate::ledger::Ledger;
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::ZhangResult;

/// Store the transaction `txn`, the directive with index `directive` in the stream being folded, which becomes
/// [`Ledger::directives`]: the store records that index, so a reader finds the directive of every stored transaction
/// even where several share a span (the paddings of a `pad`, the copies a plugin emits).
pub(crate) fn fold(txn: &Transaction, ledger: &mut Ledger, span: &SpanInfo, directive: usize) -> ZhangResult<()> {
    trace!("[fold] transaction: {:?}", txn);
    // a stage may synthesize several transactions at one place: the paddings of a `pad` serving several currencies
    let id = ledger.operations().unused_id(Uuid::from_span(span));
    // Final validation already booked the stream and rejected unbookable transactions. Bind
    // its errors to this candidate ID, which rejected transactions do not reserve in the store.
    if !ledger.take_validated_transaction(span, id) {
        return Ok(());
    }
    let mut operations = ledger.operations();

    let sequence = ledger.trx_counter.fetch_add(1, Ordering::Relaxed);
    let datetime = txn.date.to_timezone_datetime(&ledger.options.timezone);
    operations.insert_transaction(
        &id,
        sequence,
        directive,
        datetime,
        txn.flag.clone().unwrap_or(Flag::Okay),
        txn.payee.as_ref().map(|it| it.as_str()),
        txn.narration.as_ref().map(|it| it.as_str()),
        txn.tags.iter().cloned().collect_vec(),
        txn.links.iter().cloned().collect_vec(),
        span,
    )?;

    // one row per posting as written: the legs booking split from it, adjacent and sharing
    // `written.index`, are summed into it, so the rows, their ids and the balances are those
    // of the written postings. A split a stage broke apart (`written_groups`) is one row per
    // leg, as booked, like the exporter shows it
    for (posting_idx, group) in written_groups(&txn.postings).into_iter().enumerate() {
        let legs = group.legs;
        let posting = &legs[0];
        let inferred_amount = group_units(legs);
        let (unit, cost) = match group.written {
            Some(written) => (written.units.clone(), written.cost.clone()),
            None => (posting.units.clone(), posting.cost.clone()),
        };
        // the cost currency of the booked lot, through which a budget may convert the units
        let via = posting.cost.as_ref().and_then(|cost| cost.base.as_ref()).map(|cost| cost.commodity.clone());
        let units = inferred_amount.clone();
        operations.insert_transaction_posting(
            &id,
            posting_idx,
            posting.flag.clone(),
            posting.account.name(),
            unit,
            cost,
            inferred_amount,
            posting.meta.clone(),
        )?;

        // budget related: like `budget-add`, activity on a budget the stream has not defined
        // (yet) is skipped, and so is activity after the budget's close. Each is reported
        // once per (account, budget), on the first transaction that loses activity, instead
        // of once per posting
        let budgets_name = ledger.open_budgets.get(posting.account.name()).cloned().unwrap_or_default();
        for budget in budgets_name {
            let account_name = posting.account.name().to_owned();
            let Some(defined) = super::budget::defined(ledger, &budget) else {
                if ledger.reported_undefined_budgets.insert((account_name.clone(), budget.clone())) {
                    let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                    operations.new_error(ErrorKind::BudgetDoesNotExist, span, metas)?;
                }
                continue;
            };
            if defined.close.as_ref().is_some_and(|close| close.close_precedes(datetime.naive_local())) {
                if ledger.reported_closed_budgets.insert((account_name.clone(), budget.clone())) {
                    let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                    operations.new_error(ErrorKind::BudgetClosed, span, metas)?;
                }
                continue;
            }
            super::budget::keep_foreign_amount(ledger, &budget, &units, via.as_deref(), datetime.date_naive(), span, Some(&account_name));
        }
    }
    // the documents its `document` metadata names are the query engine's (`#documents`)
    operations.insert_meta(MetaType::TransactionMeta, id.to_string(), txn.meta.clone())?;
    Ok(())
}
