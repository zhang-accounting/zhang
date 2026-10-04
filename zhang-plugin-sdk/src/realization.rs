//! Balances for only the accounts a plugin needs, derived from its booked directive stream.
//!
//! [`SparseRealization`] retains one balance per requested account, not the whole account tree or
//! the postings themselves. [`AccountScope::Exact`] counts an account's own postings;
//! [`AccountScope::Subtree`] also counts its descendants, on `:` boundaries. Overlapping targets
//! have independent balances, and duplicate targets are counted once.
//!
//! Quantities and cost amounts are summed exactly, separately by commodity. Cost means booked
//! units times the lot's resolved per-unit cost; without a cost it is the units themselves.
//! Posting prices (`@` / `@@`) and the advisory `Posting.written` never affect these sums.
//! Zero totals are removed, so an absent commodity means zero. This is not market valuation:
//! use [`PriceMap`](crate::prices::PriceMap) to convert the resulting amounts at market rates.
//!
//! Zhang books postings before ordinary plugins run. The helper never books or infers anything:
//! a matching posting without units, or with an unresolved/total cost, returns [`UnbookedPosting`].
//! The entire transaction then leaves the accumulator unchanged, so a validator can report the
//! problem and continue. Unrequested accounts and non-transaction directives are ignored.
//! In particular, assertions change no balance, and pads count only when they are transactions
//! in the stream. Plugins run before the built-in pad stage, so they do not see pads it has yet
//! to generate. Call [`SparseRealization::apply`] in stream order to inspect a running balance,
//! or [`SparseRealization::from_stream`] to sum the supplied stream in full.
//!
//! ```
//! use zhang_plugin_sdk::realization::{AccountScope, SparseRealization};
//!
//! let balances = SparseRealization::from_stream(&[], ["Assets:Bank"], AccountScope::Subtree)?;
//! assert!(balances.get("Assets:Bank").unwrap().units.is_empty());
//! assert!(balances.get("Assets:Unrequested").is_none());
//! # Ok::<(), zhang_plugin_sdk::realization::UnbookedPosting>(())
//! ```

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

use bigdecimal::{BigDecimal, Zero};
use serde::Serialize;
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Posting, Spanned};
use zhang_shared::decimal::mul;

/// Which postings count towards each requested account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountScope {
    /// Only postings on the requested account itself.
    Exact,
    /// The requested account and its descendants: `Assets:Bank:Cash` is under `Assets:Bank`,
    /// but `Assets:Banking` is not.
    Subtree,
}

/// A requested account's totals, with no zero entries. The maps are ordered by commodity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AccountBalance {
    /// Booked quantities, by their own commodity (for example, `5 AAPL`).
    pub units: BTreeMap<String, BigDecimal>,
    /// Booked cost amounts, by cost commodity; uncosted units keep their own commodity
    /// (for example, `550 USD` for the five shares, alongside `20 EUR` in cash).
    pub cost: BTreeMap<String, BigDecimal>,
}

/// A matching posting is not fully booked. Its index is its position in the transaction's
/// current postings, including split legs, not the advisory written index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnbookedPosting {
    pub account: String,
    pub posting_index: usize,
}

impl Display for UnbookedPosting {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "posting {} on {} has unresolved units or cost; SparseRealization requires booked postings",
            self.posting_index, self.account
        )
    }
}

impl std::error::Error for UnbookedPosting {}

/// An accumulator storing only the requested accounts. Descendant matching walks a posting's
/// ancestors instead of scanning all targets or materializing a tree of every account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseRealization {
    scope: AccountScope,
    accounts: BTreeMap<String, AccountBalance>,
}

impl SparseRealization {
    /// Start at zero for `accounts`. Duplicate names are kept once, and accounts without
    /// postings remain present with empty totals. No account is added by later postings.
    pub fn new(accounts: impl IntoIterator<Item = impl Into<String>>, scope: AccountScope) -> Self {
        Self {
            scope,
            accounts: accounts.into_iter().map(|account| (account.into(), AccountBalance::default())).collect(),
        }
    }

    /// Sum every transaction in `stream`, using its booked units and costs.
    pub fn from_stream(
        stream: &[Spanned<Directive>], accounts: impl IntoIterator<Item = impl Into<String>>, scope: AccountScope,
    ) -> Result<Self, UnbookedPosting> {
        let mut ret = Self::new(accounts, scope);
        for directive in stream {
            ret.apply(&directive.data)?;
        }
        Ok(ret)
    }

    /// Apply one directive. Non-transactions do nothing. If a matching posting is unbooked,
    /// return its index and account and leave **all** balances unchanged for this transaction.
    /// Unbooked postings on unrequested accounts are ignored.
    pub fn apply(&mut self, directive: &Directive) -> Result<(), UnbookedPosting> {
        let Directive::Transaction(txn) = directive else {
            return Ok(());
        };
        // Resolve every relevant amount before updating anything: a bad last posting must not
        // leave the earlier postings of the transaction counted. This is arithmetic, not booking.
        let amounts = txn
            .postings
            .iter()
            .enumerate()
            .filter(|(_, posting)| ancestors(posting.account.name(), self.scope).any(|account| self.accounts.contains_key(account)))
            .map(|(index, posting)| booked_amounts(posting, index).map(|(units, cost)| (posting.account.name(), units, cost)))
            .collect::<Result<Vec<_>, _>>()?;
        for (name, units, cost) in amounts {
            for account in ancestors(name, self.scope) {
                if let Some(balance) = self.accounts.get_mut(account) {
                    add(&mut balance.units, units);
                    add(&mut balance.cost, &cost);
                }
            }
        }
        Ok(())
    }

    /// A requested account's balance; `None` means it was not requested, not that it has zero.
    pub fn get(&self, account: &str) -> Option<&AccountBalance> {
        self.accounts.get(account)
    }

    /// The requested accounts and their balances, ordered by account name.
    pub fn accounts(&self) -> impl Iterator<Item = (&str, &AccountBalance)> {
        self.accounts.iter().map(|(account, balance)| (account.as_str(), balance))
    }
}

fn ancestors(name: &str, scope: AccountScope) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(name), |name| name.rsplit_once(':').map(|(parent, _)| parent)).take(if scope == AccountScope::Exact { 1 } else { usize::MAX })
}

fn booked_amounts(posting: &Posting, posting_index: usize) -> Result<(&Amount, Amount), UnbookedPosting> {
    let unbooked = || UnbookedPosting {
        account: posting.account.name().to_owned(),
        posting_index,
    };
    let units = posting.units.as_ref().ok_or_else(unbooked)?;
    let cost = match &posting.cost {
        None => units.clone(),
        Some(cost) => {
            let base = cost.base.as_ref().filter(|_| !cost.total && cost.date.is_some()).ok_or_else(unbooked)?;
            Amount::new(mul(&units.number, &base.number), &base.commodity)
        }
    };
    Ok((units, cost))
}

fn add(totals: &mut BTreeMap<String, BigDecimal>, amount: &Amount) {
    let total = totals.entry(amount.commodity.clone()).or_default();
    *total += &amount.number;
    if total.is_zero() {
        totals.remove(&amount.commodity);
    }
}
