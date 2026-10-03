//! The ledger's account and commodity directives by name, for the functions that read them
//! from any row (`open_date()`, `close_date()`, `open_meta()`, `commodity_meta()`). Kept in
//! the cache of the ledger ([`super::LedgerCache::lookups`]), so they are indexed once per
//! loaded ledger.
//!
//! As beancount keeps them for beanquery: an account's earliest `open` and earliest `close`
//! (ties in ledger order), and a currency's last `commodity` directive.
//!
//! They also keep the budgets of every `open` (its `budget` metadata), for the budget tables and
//! the `budgets` column: an account closed and opened again with other budgets counts in the
//! budgets of the `open` in effect at each posting's date.

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use zhang_ast::{Commodity, Directive};
use zhang_core::ledger::Ledger;

use super::cache::Entries;
use crate::functions::AccountDirectives;

/// Indexes into [`Ledger::directives`].
pub(crate) struct Lookups {
    accounts: HashMap<String, (Option<usize>, Option<usize>)>,
    commodities: HashMap<String, usize>,
    /// every `open` of an account, in ledger order: its date and the budgets its `budget`
    /// metadata names (each once)
    opens: HashMap<String, Vec<(NaiveDate, BTreeSet<String>)>>,
}

impl Lookups {
    /// Index the directives of `ledger`, walking the `#entries` rows (the ledger order).
    pub fn build(ledger: &Ledger, entries: &Entries) -> Self {
        let mut accounts: HashMap<String, (Option<usize>, Option<usize>)> = HashMap::new();
        let mut commodities = HashMap::new();
        let mut opens: HashMap<String, Vec<(NaiveDate, BTreeSet<String>)>> = HashMap::new();
        for entry in &entries.rows {
            let idx = entry.directive as usize;
            match &ledger.directives[idx].data {
                Directive::Open(open) => {
                    accounts.entry(open.account.name().to_owned()).or_default().0.get_or_insert(idx);
                    let budgets = open.meta.get_all("budget").into_iter().map(|value| value.as_str().to_owned()).collect();
                    opens.entry(open.account.name().to_owned()).or_default().push((open.date.naive_date(), budgets));
                }
                Directive::Close(close) => {
                    accounts.entry(close.account.name().to_owned()).or_default().1.get_or_insert(idx);
                }
                Directive::Commodity(commodity) => {
                    commodities.insert(commodity.currency.clone(), idx);
                }
                _ => {}
            }
        }
        Lookups { accounts, commodities, opens }
    }

    /// The budgets a posting of `account` dated `date` counts in: those of the account's latest
    /// `open` on or before the date. Empty before the account's first `open`.
    pub fn budgets_at(&self, account: &str, date: NaiveDate) -> Option<&BTreeSet<String>> {
        let opens = self.opens.get(account)?;
        opens.iter().rev().find(|(opened, _)| *opened <= date).map(|(_, budgets)| budgets)
    }

    /// The accounts of every budget: those an `open` names it in, at any time.
    pub fn budget_accounts(&self) -> HashMap<String, BTreeSet<String>> {
        let mut accounts: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (account, opens) in &self.opens {
            for budget in opens.iter().flat_map(|(_, budgets)| budgets) {
                accounts.entry(budget.clone()).or_default().insert(account.clone());
            }
        }
        accounts
    }

    /// The `open` and `close` directives of `account`; `None` when it has neither.
    pub fn account<'a>(&self, ledger: &'a Ledger, account: &str) -> Option<AccountDirectives<'a>> {
        let (open, close) = self.accounts.get(account)?;
        let open = open.and_then(|idx| match &ledger.directives[idx].data {
            Directive::Open(open) => Some(open),
            _ => None,
        });
        let close = close.and_then(|idx| match &ledger.directives[idx].data {
            Directive::Close(close) => Some(close),
            _ => None,
        });
        Some(AccountDirectives { open, close })
    }

    /// The `commodity` directive of `currency`.
    pub fn commodity<'a>(&self, ledger: &'a Ledger, currency: &str) -> Option<&'a Commodity> {
        match &ledger.directives[*self.commodities.get(currency)?].data {
            Directive::Commodity(commodity) => Some(commodity),
            _ => None,
        }
    }
}
