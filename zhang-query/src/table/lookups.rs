//! The ledger's account and commodity directives by name, for the functions that read them
//! from any row (`open_date()`, `close_date()`, `open_meta()`, `commodity_meta()`). Kept in
//! the cache of the ledger ([`super::LedgerCache::lookups`]), so they are indexed once per
//! loaded ledger. `#accounts` lists the accounts from here too.
//!
//! As beancount keeps them for beanquery: an account's earliest `open` and earliest `close`
//! (ties in ledger order), and a currency's last `commodity` directive.
//!
//! The metadata of the `open` in effect at a time (`open_meta_values()`, and its alias
//! `account_budgets()`) is not indexed here: the ledger's account lifecycle answers it
//! ([`Ledger::open_meta_values_at`]), read through [`Dataset::open_meta_values`] below.

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDateTime;
use indexmap::IndexMap;
use zhang_ast::{Commodity, Directive};
use zhang_core::ledger::Ledger;

use super::cache::Entries;
use super::Dataset;
use crate::functions::AccountDirectives;

/// Indexes into [`Ledger::directives`].
pub(crate) struct Lookups {
    /// the first `open` and first `close` of each account, in the order of the first of them
    accounts: IndexMap<String, (Option<usize>, Option<usize>)>,
    commodities: HashMap<String, usize>,
}

impl Lookups {
    /// Index the directives of `ledger`, walking the `#entries` rows (the ledger order).
    pub fn build(ledger: &Ledger, entries: &Entries) -> Self {
        let mut accounts: IndexMap<String, (Option<usize>, Option<usize>)> = IndexMap::new();
        let mut commodities = HashMap::new();
        for entry in &entries.rows {
            let idx = entry.directive as usize;
            match &ledger.directives[idx].data {
                Directive::Open(open) => {
                    accounts.entry(open.account.name().to_owned()).or_default().0.get_or_insert(idx);
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
        Lookups { accounts, commodities }
    }

    /// Every account with an `open` or a `close`, in the order of the first of them, with the indexes of its first
    /// `open` and its first `close`.
    pub fn accounts(&self) -> impl Iterator<Item = (&str, Option<usize>, Option<usize>)> {
        self.accounts.iter().map(|(name, (open, close))| (name.as_str(), *open, *close))
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

impl<'a> Dataset<'a> {
    /// The values of the metadata `key` of the `open` of `account` in effect at the wall-clock time `at`, for
    /// `open_meta_values()`: the ledger's account lifecycle ([`Ledger::open_meta_values_at`]). `None` before the
    /// account's first `open`.
    pub fn open_meta_values(&self, account: &str, key: &str, at: NaiveDateTime) -> Option<&'a BTreeSet<String>> {
        self.ledger.open_meta_values_at(account, key, at)
    }
}
