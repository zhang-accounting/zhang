//! Pure stream-fold helpers shared by the built-in balance stages.
//!
//! [`PadStage`](crate::pipeline::PadStage) and
//! [`BalanceCheckStage`](crate::pipeline::BalanceCheckStage) are two
//! independent folds over the same stream (beancount's pad/balance design); they
//! share no mutable state, only these helpers. Each fold sees exactly what the
//! store fold will book, in stream order — the stream is sorted, so "every
//! transaction before this directive" is "every transaction up to its datetime".

use std::collections::HashMap;
use std::ops::{Add, Sub};

use bigdecimal::{BigDecimal, Zero};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Directive, Transaction};

use crate::inventory::TransactionInference;

/// running per-account, per-commodity unit sums of the transactions folded so far.
///
/// Balances are per *exact* account (not its subtree) — what a balance directive
/// asserts in zhang.
#[derive(Default)]
pub struct UnitBalances {
    /// account name -> commodity -> units
    balances: HashMap<String, HashMap<String, BigDecimal>>,
}

impl UnitBalances {
    /// book a transaction the way the store fold does: transactions the fold
    /// rejects (see [`TransactionInference::validation_inventory`]) are skipped,
    /// every other posting adds its units, or for an implicit posting the amount
    /// inferred from the other postings
    pub fn apply_transaction(&mut self, txn: &Transaction) {
        if txn.validation_inventory().is_err() {
            return;
        }
        let units: Result<Vec<Amount>, ErrorKind> = txn
            .txn_postings()
            .iter()
            .map(|posting| posting.units().map(Ok).unwrap_or_else(|| posting.infer_trade_amount()))
            .collect();
        // amounts the fold cannot infer abort the whole load there; nothing to book here
        let Ok(units) = units else {
            return;
        };
        for (posting, amount) in txn.postings.iter().zip(units) {
            self.add(&posting.account, &amount);
        }
    }

    pub fn add(&mut self, account: &Account, amount: &Amount) {
        let commodities = self.balances.entry(account.name().to_owned()).or_default();
        let balance = commodities.entry(amount.commodity.clone()).or_insert_with(BigDecimal::zero);
        *balance = (&*balance).add(&amount.number);
    }

    /// the account's current units of `commodity`; zero if it holds none
    pub fn balance(&self, account: &Account, commodity: &str) -> BigDecimal {
        self.balances
            .get(account.name())
            .and_then(|commodities| commodities.get(commodity))
            .cloned()
            .unwrap_or_else(BigDecimal::zero)
    }

    /// how far the account is from `target`: `target - current balance`
    pub fn distance(&self, account: &Account, target: &Amount) -> Amount {
        let current = self.balance(account, &target.commodity);
        Amount::new((&target.number).sub(&current), target.commodity.clone())
    }
}

/// whether a balance distance breaks the assertion: `|distance| > tolerance`,
/// tolerance defaulting to zero
pub fn exceeds_tolerance(distance: &BigDecimal, tolerance: Option<&BigDecimal>) -> bool {
    let zero = BigDecimal::zero();
    let tolerance = tolerance.unwrap_or(&zero);
    distance.abs() > *tolerance
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountState {
    Open,
    Closed,
}

/// account lifecycle folded from `open` / `close` directives, mirroring the store:
/// `open` (re)opens an account, `close` only closes an account that exists
#[derive(Default)]
pub struct AccountStates {
    accounts: HashMap<String, AccountState>,
}

impl AccountStates {
    /// fold an `open` / `close` directive; other directives are ignored
    pub fn apply(&mut self, directive: &Directive) {
        match directive {
            Directive::Open(open) => {
                self.accounts.insert(open.account.name().to_owned(), AccountState::Open);
            }
            Directive::Close(close) => {
                if let Some(state) = self.accounts.get_mut(close.account.name()) {
                    *state = AccountState::Closed;
                }
            }
            _ => {}
        }
    }

    pub fn exists(&self, account: &Account) -> bool {
        self.accounts.contains_key(account.name())
    }

    pub fn is_closed(&self, account: &Account) -> bool {
        self.accounts.get(account.name()) == Some(&AccountState::Closed)
    }

    /// the account errors a directive referencing `accounts` raises, in the order
    /// the store fold reports them: every missing account, then every closed one
    pub fn errors<'a>(&self, accounts: &[&'a Account]) -> Vec<(ErrorKind, &'a Account)> {
        let missing = accounts.iter().filter(|it| !self.exists(it)).map(|it| (ErrorKind::AccountDoesNotExist, *it));
        let closed = accounts.iter().filter(|it| self.is_closed(it)).map(|it| (ErrorKind::AccountClosed, *it));
        missing.chain(closed).collect()
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Account, Directive};

    use super::{exceeds_tolerance, AccountStates, UnitBalances};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;

    fn parse(content: &str) -> Vec<Directive> {
        ZhangDataType {}
            .transform(content.to_owned(), None)
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect()
    }

    fn account(name: &str) -> Account {
        Account::from_str(name).unwrap()
    }

    fn fold(content: &str) -> UnitBalances {
        let mut balances = UnitBalances::default();
        for directive in parse(content) {
            if let Directive::Transaction(txn) = directive {
                balances.apply_transaction(&txn);
            }
        }
        balances
    }

    #[test]
    fn should_book_inferred_amount_of_implicit_posting() {
        let balances = fold(indoc! {r#"
            2023-01-01 * "buy with price"
              Assets:Broker 2 AAPL @ 100 USD
              Assets:Cash
            2023-01-02 * "buy with cost"
              Assets:Broker 3 AAPL {10 USD}
              Assets:Cash
            2023-01-03 * "plain"
              Assets:Cash
              Equity:Open -10 CNY
        "#});

        assert_eq!(balances.balance(&account("Assets:Cash"), "USD"), BigDecimal::from(-230));
        assert_eq!(balances.balance(&account("Assets:Cash"), "CNY"), BigDecimal::from(10));
        assert_eq!(balances.balance(&account("Assets:Broker"), "AAPL"), BigDecimal::from(5));
    }

    #[test]
    fn should_skip_transactions_the_store_fold_rejects() {
        let balances = fold(indoc! {r#"
            2023-01-01 * "two implicit postings"
              Assets:A
              Assets:B
              Equity:Open 10 CNY
            2023-01-02 * "nothing to infer from"
              Assets:A 0 CNY
              Assets:B
            2023-01-03 * "explicit postings in multiple commodities"
              Assets:A 5 CNY
              Assets:A 5 USD
              Assets:B
            2023-01-04 * "unbalanced, but still booked"
              Assets:A 5 CNY
              Assets:B 3 CNY
        "#});

        assert_eq!(balances.balance(&account("Assets:A"), "CNY"), BigDecimal::from(5));
        assert_eq!(balances.balance(&account("Assets:A"), "USD"), BigDecimal::from(0));
        assert_eq!(balances.balance(&account("Assets:B"), "CNY"), BigDecimal::from(3));
        assert_eq!(balances.balance(&account("Equity:Open"), "CNY"), BigDecimal::from(0));
    }

    #[test]
    fn should_track_exact_account_not_subtree() {
        let balances = fold(indoc! {r#"
            2023-01-01 * ""
              Assets:A:Sub 10 CNY
              Equity:Open
        "#});
        assert_eq!(balances.balance(&account("Assets:A"), "CNY"), BigDecimal::from(0));
        assert_eq!(
            balances.distance(&account("Assets:A:Sub"), &Amount::new(BigDecimal::from(15), "CNY")),
            Amount::new(BigDecimal::from(5), "CNY")
        );
    }

    #[test]
    fn should_compare_distance_against_tolerance() {
        let tolerance = BigDecimal::from_str("0.01").unwrap();
        assert!(!exceeds_tolerance(&BigDecimal::from(0), None));
        assert!(exceeds_tolerance(&BigDecimal::from_str("-0.001").unwrap(), None));
        assert!(!exceeds_tolerance(&BigDecimal::from_str("0.01").unwrap(), Some(&tolerance)));
        assert!(!exceeds_tolerance(&BigDecimal::from_str("-0.01").unwrap(), Some(&tolerance)));
        assert!(exceeds_tolerance(&BigDecimal::from_str("-0.011").unwrap(), Some(&tolerance)));
    }

    #[test]
    fn should_follow_account_lifecycle() {
        let mut states = AccountStates::default();
        for directive in parse(indoc! {r#"
            1970-01-01 close Assets:NeverOpened
            1970-01-01 open Assets:A
            1970-01-01 open Assets:Reopened
            1970-01-02 close Assets:A
            1970-01-02 close Assets:Reopened
            1970-01-03 open Assets:Reopened
        "#})
        {
            states.apply(&directive);
        }

        assert!(!states.exists(&account("Assets:NeverOpened")));
        assert!(states.is_closed(&account("Assets:A")));
        assert!(states.exists(&account("Assets:Reopened")));
        assert!(!states.is_closed(&account("Assets:Reopened")));

        let missing = account("Assets:Missing");
        let closed = account("Assets:A");
        let errors = states.errors(&[&closed, &missing]);
        assert_eq!(
            errors.into_iter().map(|(kind, account)| (kind, account.name().to_owned())).collect::<Vec<_>>(),
            vec![
                (ErrorKind::AccountDoesNotExist, "Assets:Missing".to_owned()),
                (ErrorKind::AccountClosed, "Assets:A".to_owned()),
            ]
        );
    }
}
