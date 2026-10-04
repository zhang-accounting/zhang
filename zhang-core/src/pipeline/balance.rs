//! Pure stream-fold helpers shared by the built-in stages.
//!
//! [`PadStage`](crate::pipeline::PadStage) and
//! [`BalanceCheckStage`](crate::pipeline::BalanceCheckStage) are two
//! independent folds over the same stream (beancount's pad/balance design); they
//! share no mutable state, only these helpers. Each fold sees exactly what the
//! store fold will book, in stream order — the stream is sorted, so "every
//! transaction before this directive" is "every transaction up to its datetime".
//! Only transactions move a balance: a balance assertion never does.
//! [`ActiveAccountsStage`](crate::pipeline::ActiveAccountsStage) folds the account
//! lifecycle ([`AccountStates`]) the same way.

use std::collections::{BTreeMap, HashMap};
use std::ops::{Add, Sub};
use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Commodity, Directive, Open, Rounding, Transaction};

use super::StageContext;
use crate::booking::{group_units, written_groups, BookOutcome, Booker};
use crate::constants::{DEFAULT_BOOKING_METHOD, KEY_DEFAULT_BOOKING_METHOD, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING};
use crate::domains::schemas::{CommodityDomain, OptionDomain};
use crate::inventory::BookingMethod;
use crate::process::commodity::commodity_precision;

/// an option's value; an invalid one aborts the load in the store fold
fn option_value<T: FromStr>(options: &[OptionDomain], key: &str) -> Option<T> {
    options.iter().find(|option| option.key == key).and_then(|option| option.value.parse().ok())
}

/// the ledger's default booking method, as the options handler resolved it into `options`
pub fn default_booking_method(options: &[OptionDomain]) -> BookingMethod {
    let fallback = DEFAULT_BOOKING_METHOD.parse().expect("the default booking method is valid");
    options
        .iter()
        .find(|option| option.key == KEY_DEFAULT_BOOKING_METHOD)
        .map(|option| BookingMethod::resolve(&option.value, fallback).0)
        .unwrap_or(fallback)
}

/// running per-account, per-commodity unit sums of the transactions folded so far.
///
/// The balance of an account is that of the account and all its sub-accounts, as in
/// beancount: what a balance directive asserts, and what a pad brings to its amount.
pub struct UnitBalances {
    /// account name -> commodity -> units of the postings of that account alone, sorted by
    /// name so that an account's sub-accounts are a range
    balances: BTreeMap<String, HashMap<String, BigDecimal>>,
    /// books every transaction like the store fold, so an implicit posting gets the amount the
    /// fold gives it, interpolated from the lots its transaction books against. Its errors are
    /// the fold's to report
    booker: Booker,
}

impl UnitBalances {
    /// `commodities` are those defined before the stream starts (by the options)
    pub fn new(default_booking_method: BookingMethod, commodities: &[CommodityDomain]) -> Self {
        let mut booker = Booker::new(default_booking_method);
        for commodity in commodities {
            booker.define_commodity(&commodity.name, commodity.precision, commodity.rounding);
        }
        Self {
            balances: BTreeMap::new(),
            booker,
        }
    }

    /// the balances of a stage, set up from its context
    pub fn for_stage(ctx: &StageContext) -> Self {
        Self::new(default_booking_method(ctx.options), &ctx.commodities)
    }

    /// fold an `open`: its booking method decides which lots later reductions book against
    pub fn apply_open(&mut self, open: &Open) {
        let _reported_by_the_fold = self.booker.apply_open(open);
    }

    /// fold a `commodity`: implicit postings in it are rounded at its precision, as the store fold
    /// defines it from the same options
    pub fn apply_commodity(&mut self, commodity: &Commodity, options: &[OptionDomain]) {
        let default_precision = option_value::<i32>(options, KEY_DEFAULT_COMMODITY_PRECISION);
        let default_rounding = option_value::<Rounding>(options, KEY_DEFAULT_ROUNDING);
        // an invalid `rounding` meta aborts the load in the store fold; nothing to define here
        if let Ok((precision, rounding)) = commodity_precision(commodity, default_precision, default_rounding) {
            self.booker.define_commodity(&commodity.currency, precision, rounding);
        }
    }

    /// book a transaction the way the store fold does: transactions the fold
    /// rejects (their implicit posting cannot be interpolated) are skipped,
    /// every other posting adds its units, or for an implicit posting the amount
    /// interpolated from the other postings. Returns the units of each posting, none
    /// for a transaction skipped
    pub fn apply_transaction(&mut self, txn: &Transaction) -> Vec<Amount> {
        // a rejected transaction does not reach the store; nothing to book here. A copy is
        // booked: the stream keeps the postings as written
        let mut booked = txn.clone();
        let BookOutcome::Booked(_) = self.booker.book(&mut booked) else {
            return vec![];
        };
        // the units of every posting as written: the legs booking split from it summed
        written_groups(&booked.postings)
            .into_iter()
            .map(|group| {
                let units = group_units(group.legs);
                self.add(&group.legs[0].account, &units);
                units
            })
            .collect()
    }

    fn add(&mut self, account: &Account, amount: &Amount) {
        let commodities = self.balances.entry(account.name().to_owned()).or_default();
        let balance = commodities.entry(amount.commodity.clone()).or_insert_with(BigDecimal::zero);
        *balance = (&*balance).add(&amount.number);
    }

    /// the current units of `commodity` of the account and all its sub-accounts; zero if they hold none
    pub fn balance(&self, account: &Account, commodity: &str) -> BigDecimal {
        let name = account.name();
        // `Assets:Bank:` up to `Assets:Bank;` (`;` follows `:`) holds exactly the sub-accounts of `Assets:Bank`
        let sub_accounts = self.balances.range(format!("{name}:")..format!("{name};"));
        self.balances
            .get_key_value(name)
            .into_iter()
            .chain(sub_accounts)
            .filter_map(|(_, commodities)| commodities.get(commodity))
            .fold(BigDecimal::zero(), |sum, units| sum + units)
    }

    /// whether the account or one of its sub-accounts holds `commodity` at cost, in a lot with a cost
    pub fn holds_at_cost(&self, account: &Account, commodity: &str) -> bool {
        self.booker.holds_at_cost(account.name(), commodity)
    }

    /// [`UnitBalances::balance`] as an amount
    pub fn amount(&self, account: &Account, commodity: &str) -> Amount {
        Amount::new(self.balance(account, commodity), commodity)
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
    /// closed by a `close` dated on this day
    Closed(NaiveDate),
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
                    *state = AccountState::Closed(close.date.naive_date());
                }
            }
            _ => {}
        }
    }

    pub fn exists(&self, account: &Account) -> bool {
        self.accounts.contains_key(account.name())
    }

    pub fn is_closed(&self, account: &Account) -> bool {
        matches!(self.accounts.get(account.name()), Some(AccountState::Closed(_)))
    }

    /// the error a reference made on `date` to the account raises, `None` if the account is active
    /// then: `AccountDoesNotExist` if no `open` of it was folded yet, `AccountClosed` if it was closed
    /// on an earlier day. The account stays active through the whole day of its `close`, as in
    /// beancount, which sorts `close` after every other entry of its day
    pub fn inactive_error(&self, account: &Account, date: NaiveDate) -> Option<ErrorKind> {
        match self.accounts.get(account.name()) {
            None => Some(ErrorKind::AccountDoesNotExist),
            Some(AccountState::Closed(closed)) if *closed < date => Some(ErrorKind::AccountClosed),
            Some(_) => None,
        }
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
    use crate::inventory::BookingMethod;

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
        let mut balances = UnitBalances::new(BookingMethod::Fifo, &[]);
        for directive in parse(content) {
            match directive {
                Directive::Open(open) => balances.apply_open(&open),
                Directive::Transaction(txn) => {
                    balances.apply_transaction(&txn);
                }
                _ => {}
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
            2023-01-02 * "nothing to infer from: balanced in two commodities"
              Assets:A 0 CNY
              Assets:A 0 USD
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
    fn should_sum_an_account_with_its_sub_accounts() {
        let balances = fold(indoc! {r#"
            2023-01-01 * ""
              Assets:A 1 CNY
              Assets:A:Sub 10 CNY
              Assets:A:Sub:Deep 100 CNY
              Assets:AB 1000 CNY
              Assets:A:Sub 5 USD
              Equity:Open -1111 CNY
              Equity:Open -5 USD
        "#});
        // `Assets:AB` is no sub-account of `Assets:A`
        assert_eq!(balances.balance(&account("Assets:A"), "CNY"), BigDecimal::from(111));
        assert_eq!(balances.balance(&account("Assets:A:Sub"), "CNY"), BigDecimal::from(110));
        assert_eq!(balances.balance(&account("Assets:A:Sub:Deep"), "CNY"), BigDecimal::from(100));
        assert_eq!(balances.balance(&account("Assets:A"), "USD"), BigDecimal::from(5));
        assert_eq!(balances.balance(&account("Assets"), "CNY"), BigDecimal::from(1111));
        assert_eq!(balances.balance(&account("Assets:Missing"), "CNY"), BigDecimal::from(0));
        assert_eq!(
            balances.distance(&account("Assets:A"), &Amount::new(BigDecimal::from(150), "CNY")),
            Amount::new(BigDecimal::from(39), "CNY")
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

        // active through the whole day of the close
        let day = |day: u32| chrono::NaiveDate::from_ymd_opt(1970, 1, day).unwrap();
        assert_eq!(states.inactive_error(&closed, day(2)), None);
        assert_eq!(states.inactive_error(&closed, day(3)), Some(ErrorKind::AccountClosed));
        assert_eq!(states.inactive_error(&missing, day(1)), Some(ErrorKind::AccountDoesNotExist));
        assert_eq!(states.inactive_error(&account("Assets:Reopened"), day(4)), None);
    }
}
