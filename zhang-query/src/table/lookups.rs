//! The ledger's account and commodity directives by name, for the functions that read them
//! from any row (`open_date()`, `close_date()`, `open_meta()`, `commodity_meta()`).
//!
//! As beancount keeps them for beanquery: an account's earliest `open` and earliest `close`
//! (ties in ledger order), and a currency's last `commodity` directive.

use std::collections::HashMap;

use zhang_ast::{Close, Commodity, Directive, Open};
use zhang_core::ledger::Ledger;

use super::directives::ledger_order;
use crate::functions::AccountDirectives;

pub(crate) struct Lookups<'a> {
    accounts: HashMap<&'a str, (Option<&'a Open>, Option<&'a Close>)>,
    commodities: HashMap<&'a str, &'a Commodity>,
}

impl<'a> Lookups<'a> {
    pub fn new(ledger: &'a Ledger) -> Self {
        let mut accounts: HashMap<&str, (Option<&Open>, Option<&Close>)> = HashMap::new();
        let mut commodities = HashMap::new();
        for directive in ledger_order(ledger) {
            match &directive.data {
                Directive::Open(open) => {
                    accounts.entry(open.account.name()).or_default().0.get_or_insert(open);
                }
                Directive::Close(close) => {
                    accounts.entry(close.account.name()).or_default().1.get_or_insert(close);
                }
                Directive::Commodity(commodity) => {
                    commodities.insert(commodity.currency.as_str(), commodity);
                }
                _ => {}
            }
        }
        Lookups { accounts, commodities }
    }

    /// The `open` and `close` directives of `account`; `None` when it has neither.
    pub fn account(&self, account: &str) -> Option<AccountDirectives<'a>> {
        self.accounts.get(account).map(|(open, close)| AccountDirectives { open: *open, close: *close })
    }

    /// The `commodity` directive of `currency`.
    pub fn commodity(&self, currency: &str) -> Option<&'a Commodity> {
        self.commodities.get(currency).copied()
    }
}
