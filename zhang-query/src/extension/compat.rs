//! The compatibility allowlist: wording the budget system has always had, kept verbatim for `zhang:budget` alone
//! (RFC #698 §3.7, §9.3). Any other provider gets the generic wording.

use std::fmt::Write as _;

use chrono::NaiveDate;
use zhang_ast::{Directive, Spanned};
use zhang_core::ledger::Ledger;

use super::series::{EndBy, Extent, SeriesSpec};
use super::snapshot::Cause;
use crate::catalog::TableRef;
use crate::table::{date_of, entry_type, ledger_file, LedgerCache};

/// The provider the legacy budget names and wording are granted to.
pub(crate) const BUDGET: &str = "zhang:budget";

/// The message of a series too long for the result: `total` periods in all, `longest` the partition with the most.
pub(crate) fn too_long(ledger: &Ledger, table: &TableRef, provider: &str, spec: &SeriesSpec, total: u64, longest: &Extent<'_>) -> String {
    if provider == BUDGET && table.name() == "budgets" {
        return budget_months(ledger, spec, total, longest);
    }
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    // the partition by its key, as its first fact has it
    let mut key = String::new();
    if let Some(fact) = longest.partition.facts.first() {
        for (n, column) in spec.partition.iter().enumerate() {
            let name = table.columns().get(*column).map_or("", |it| it.name.as_ref());
            let cell = fact.cells.get(*column).map(ToString::to_string).unwrap_or_default();
            let _ = write!(key, "{} {} = {}", if n == 0 { "" } else { "," }, name, cell);
        }
    }
    let cause = match longest.end_by {
        EndBy::Query => " (the last month the query keeps)".to_owned(),
        EndBy::CurrentPeriod => match entry(ledger, longest.partition.first_cause) {
            Some(directive) => format!(
                ", the current month; check the date of the {} it starts with{}",
                entry_type(&directive.data),
                file(ledger, directive)
            ),
            None => ", the current month".to_owned(),
        },
        EndBy::Entry(cause) => entry(ledger, cause)
            .and_then(|directive| {
                let date = date_of(&directive.data)?;
                Some(format!(
                    " because of the {} dated {}{}; check that date",
                    entry_type(&directive.data),
                    date,
                    file(ledger, directive)
                ))
            })
            .unwrap_or_default(),
    };
    format!(
        "the #{} table would generate too many rows ({} months in all, more than the result size limit): the series of{} runs from {} until {}{}",
        table.name(),
        total,
        key,
        month(longest.first),
        month(longest.end),
        cause
    )
}

/// The `#budgets` wording: the budget with the longest series (by its partition column, `name`), and the directive
/// whose date sets its end.
fn budget_months(ledger: &Ledger, spec: &SeriesSpec, total: u64, longest: &Extent<'_>) -> String {
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    let name = longest
        .partition
        .facts
        .first()
        .zip(spec.partition.first())
        .and_then(|(fact, column)| fact.cells.get(*column))
        .and_then(|cell| cell.as_str())
        .unwrap_or_default();
    let cause = match longest.end_by {
        EndBy::Query => " (the last month the query keeps)".to_owned(),
        // the current month ends it: the series is long because it starts early
        EndBy::CurrentPeriod => {
            let definition = entry(ledger, longest.partition.first_cause);
            format!(
                ", the current month; check the date of its budget directive{}",
                definition.map(|it| file(ledger, it)).unwrap_or_default()
            )
        }
        EndBy::Entry(cause) => entry(ledger, cause)
            .and_then(|directive| {
                let kind = match &directive.data {
                    Directive::Transaction(_) => "a transaction",
                    Directive::Budget(_) => "its budget directive",
                    Directive::BudgetAdd(_) => "a budget-add",
                    Directive::BudgetTransfer(_) => "a budget-transfer",
                    Directive::BudgetClose(_) => "a budget-close",
                    _ => return None,
                };
                let date = date_of(&directive.data)?;
                Some(format!(" because of {} dated {}{}; check that date", kind, date, file(ledger, directive)))
            })
            .unwrap_or_default(),
    };
    format!(
        "the #budgets table would generate too many rows ({} months in all, more than the result size limit): \
         budget '{}' runs from {} until {}{}",
        total,
        name,
        month(longest.first),
        month(longest.end),
        cause
    )
}

/// The directive of the entry `cause`.
fn entry(ledger: &Ledger, cause: Option<Cause>) -> Option<&Spanned<Directive>> {
    let entries = LedgerCache::of(ledger).entries(ledger);
    let row = entries.rows.get(cause?.seq as usize)?;
    ledger.directives.get(row.directive as usize)
}

/// ` (file)`, the file of `directive` within the ledger, or nothing.
fn file(ledger: &Ledger, directive: &Spanned<Directive>) -> String {
    directive
        .span
        .filename
        .as_deref()
        .map(|path| format!(" ({})", ledger_file(ledger, path).display()))
        .unwrap_or_default()
}
