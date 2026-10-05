//! Pure stream-fold helpers shared by the built-in stages.
//!
//! [`PadStage`](crate::pipeline::PadStage) and
//! [`BalanceCheckStage`](crate::pipeline::BalanceCheckStage) are two
//! independent folds over the same stream (beancount's pad/balance design); they
//! share no mutable state, only these helpers. Each fold sees exactly what
//! final validation will book, in stream order — the stream is sorted, so "every
//! transaction before this directive" is "every transaction up to its datetime".
//! Only transactions move a balance: a balance assertion never does.
//!
//! The stream reaches these stages booked ([`BookingStage`](crate::pipeline::BookingStage)
//! ran before the plugins), so [`UnitBalances`] sums the units of the booked postings and
//! reads the lots an account holds at cost off the legs that carry a cost. It books nothing
//! itself, except a transaction a stage left unbooked (the padding transactions it makes, a
//! plugin's posting without units), which it completes the way final validation will.
//! [`ActiveAccountsStage`](crate::pipeline::ActiveAccountsStage) folds the account
//! lifecycle ([`AccountStates`]) the same way.

use std::collections::{BTreeMap, HashMap};
use std::ops::{Add, AddAssign, Sub};
use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Commodity, Directive, Open, Posting, Rounding, Transaction};

use super::StageContext;
use crate::booking::{group_units, weighs_by_lots, written_groups, BookOutcome, Booker};
use crate::constants::{DEFAULT_BOOKING_METHOD, KEY_DEFAULT_BOOKING_METHOD, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING};
use crate::domains::schemas::{CommodityDomain, OptionDomain};
use crate::inventory::BookingMethod;
use crate::process::commodity::commodity_precision;

/// An option's resolved value; invalid options abort the load before the pipeline runs.
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
    /// account name -> lot (commodity, cost, acquisition date, label) -> units held in it, from the
    /// booked legs that carry a cost: the lots final validation ends with, sorted by name like
    /// `balances`
    at_cost: BTreeMap<String, HashMap<LotKey, BigDecimal>>,
    /// completes the one kind of transaction the stages leave unbooked and final validation accepts:
    /// a posting without units next to postings that weigh as written, which is what the pad
    /// stage's own padding transactions are (the stream is booked again after the plugins, so a
    /// plugin's output never gets here unbooked). It interpolates as final validation will; it holds no
    /// lots, and never needs them. Its errors are final validation's to report
    booker: Booker,
}

/// a lot a booked leg names: its commodity, and its per-unit cost (the number normalized, so
/// `10` and `10.0` are one lot, as they are one lot to the booker), acquisition date and label
/// as text
type LotKey = (String, String);

/// a booker set up like final validation's: the ledger's default booking method and `commodities`,
/// those defined before the stream starts (by the options)
pub(crate) fn booker(default_booking_method: BookingMethod, commodities: &[CommodityDomain]) -> Booker {
    let mut booker = Booker::new(default_booking_method);
    for commodity in commodities {
        booker.define_commodity(&commodity.name, commodity.precision, commodity.rounding);
    }
    booker
}

/// the booker of a stage, set up from its context
pub(crate) fn stage_booker(ctx: &StageContext) -> Booker {
    booker(default_booking_method(ctx.options), &ctx.commodities)
}

/// fold a `commodity` directive into `booker`: implicit postings in it are rounded at its precision,
/// as the store fold defines it from the same options
pub(crate) fn define_commodity(booker: &mut Booker, commodity: &Commodity, options: &[OptionDomain]) {
    let default_precision = option_value::<i32>(options, KEY_DEFAULT_COMMODITY_PRECISION);
    let default_rounding = option_value::<Rounding>(options, KEY_DEFAULT_ROUNDING);
    // an invalid `rounding` meta aborts the load in the store fold; nothing to define here
    if let Ok((precision, rounding)) = commodity_precision(commodity, default_precision, default_rounding) {
        booker.define_commodity(&commodity.currency, precision, rounding);
    }
}

impl UnitBalances {
    /// `commodities` are those defined before the stream starts (by the options)
    pub fn new(default_booking_method: BookingMethod, commodities: &[CommodityDomain]) -> Self {
        Self {
            balances: BTreeMap::new(),
            at_cost: BTreeMap::new(),
            booker: booker(default_booking_method, commodities),
        }
    }

    /// the balances of a stage, set up from its context
    pub fn for_stage(ctx: &StageContext) -> Self {
        Self::new(default_booking_method(ctx.options), &ctx.commodities)
    }

    /// fold an `open`: its booking method decides which lots later reductions book against
    pub fn apply_open(&mut self, open: &Open) {
        let _reported_by_validation = self.booker.apply_open(open);
    }

    /// fold a `commodity`: implicit postings in it are rounded at its precision, as final validation
    /// defines it from the same options
    pub fn apply_commodity(&mut self, commodity: &Commodity, options: &[OptionDomain]) {
        define_commodity(&mut self.booker, commodity, options);
    }

    /// fold a transaction: every posting adds its units. A transaction the booking stage
    /// booked is summed as it is. One left with a posting without units is completed first,
    /// as final validation will complete it, when nothing in it weighs by lots (the padding
    /// transactions); otherwise, and when its implicit posting cannot be interpolated, final validation
    /// rejects it, and it is skipped here too. Returns the units of each posting as written
    /// (the legs booking split from it summed), none for a transaction skipped
    pub fn apply_transaction(&mut self, txn: &Transaction) -> Vec<Amount> {
        // An unresolved explicit cost after booking is an unbookable transaction (E6/E9),
        // including one whose units were all written. Final validation rejects it too.
        if txn.postings.iter().any(weighs_by_lots) {
            return vec![];
        }
        let completed;
        let txn = if txn.postings.iter().all(|posting| posting.units.is_some()) {
            txn
        } else {
            // Complete a copy, such as the padding stage's implicit leg; the stream keeps
            // the postings the stage left. Unresolved costs were rejected above.
            let mut copy = txn.clone();
            let BookOutcome::Booked(_) = self.booker.book(&mut copy) else {
                return vec![];
            };
            completed = copy;
            &completed
        };
        written_groups(&txn.postings)
            .into_iter()
            .map(|group| {
                let legs = group.legs;
                let units = group_units(legs);
                self.add(&legs[0].account, &units);
                for leg in legs {
                    self.add_at_cost(leg);
                }
                units
            })
            .collect()
    }

    fn add(&mut self, account: &Account, amount: &Amount) {
        let commodities = self.balances.entry(account.name().to_owned()).or_default();
        let balance = commodities.entry(amount.commodity.clone()).or_insert_with(BigDecimal::zero);
        *balance = (&*balance).add(&amount.number);
    }

    /// a booked leg carrying a cost adds its units to the lot it names; a leg without a cost
    /// number (`{}` no lot covered) is held without cost, like final validation's default lot
    fn add_at_cost(&mut self, leg: &Posting) {
        let (Some(units), Some(cost)) = (&leg.units, leg.cost.as_ref().and_then(|cost| cost.base.as_ref())) else {
            return;
        };
        let spec = leg.cost.as_ref().expect("a cost number comes with a cost spec");
        let lot = format!(
            "{} {} {:?} {:?}",
            cost.number.normalized(),
            cost.commodity,
            spec.date.as_ref().map(|it| it.naive_date()),
            spec.label
        );
        let key = (units.commodity.clone(), lot);
        let lots = self.at_cost.entry(leg.account.name().to_owned()).or_default();
        lots.entry(key).or_insert_with(BigDecimal::zero).add_assign(&units.number);
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

    /// whether the account or one of its sub-accounts holds `commodity` at cost: a lot of it with
    /// a cost, which some booked legs filled and later ones did not empty
    pub fn holds_at_cost(&self, account: &Account, commodity: &str) -> bool {
        let name = account.name();
        let sub_accounts = self.at_cost.range(format!("{name}:")..format!("{name};"));
        self.at_cost
            .get_key_value(name)
            .into_iter()
            .chain(sub_accounts)
            .flat_map(|(_, lots)| lots)
            .any(|((lot_commodity, _), units)| lot_commodity == commodity && !units.is_zero())
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
/// `open` (re)opens an account, `close` only closes an account that exists.
/// It also keeps the commodities each account was opened with, which restrict what it may hold
#[derive(Default)]
pub struct AccountStates {
    accounts: HashMap<String, AccountState>,
    /// the commodities listed by the latest `open` of each account that lists any: the only
    /// commodities it may hold. An `open` without any, a reopening included, lifts the restriction
    allowed: HashMap<String, Vec<String>>,
}

impl AccountStates {
    /// fold an `open` / `close` directive; other directives are ignored
    pub fn apply(&mut self, directive: &Directive) {
        match directive {
            Directive::Open(open) => {
                let name = open.account.name().to_owned();
                if open.commodities.is_empty() {
                    self.allowed.remove(&name);
                } else {
                    self.allowed.insert(name.clone(), open.commodities.clone());
                }
                self.accounts.insert(name, AccountState::Open);
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

    /// `CommodityNotAllowed` if the account was opened with a list of commodities that does not include
    /// `commodity`, as in beancount: a posting, a balance assertion or a padding in it is invalid. `None` if the
    /// account lists none (it holds any commodity), or never was opened. Only the account itself is restricted,
    /// not its sub-accounts
    pub fn commodity_error(&self, account: &Account, commodity: &str) -> Option<ErrorKind> {
        let allowed = self.allowed.get(account.name())?;
        (!allowed.iter().any(|it| it == commodity)).then_some(ErrorKind::CommodityNotAllowed)
    }

    /// the account errors a directive referencing `accounts` raises, in the order
    /// the account stages report them: every missing account, then every closed one
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

    /// `content` booked as the booking stage books it, then folded
    fn fold_booked(content: &str) -> UnitBalances {
        let mut booker = crate::booking::Booker::new(BookingMethod::Fifo);
        let mut balances = UnitBalances::new(BookingMethod::Fifo, &[]);
        for mut directive in parse(content) {
            match &mut directive {
                Directive::Open(open) => {
                    booker.apply_open(open);
                    balances.apply_open(open);
                }
                Directive::Transaction(txn) => {
                    let _ = booker.book(txn);
                    assert!(txn.postings.iter().all(|it| it.units.is_some()), "the stage booked it");
                    balances.apply_transaction(txn);
                }
                _ => {}
            }
        }
        balances
    }

    #[test]
    fn should_sum_booked_legs_and_read_the_lots_off_them() {
        let balances = fold_booked(indoc! {r#"
            2023-01-01 * "buy two lots"
              Assets:Broker 3 AAPL {10 USD}
              Assets:Broker:Sub 2 AAPL {11 USD}
              Assets:Cash
            2023-01-02 * "sell across both, the legs split"
              Assets:Broker -3 AAPL {}
              Assets:Broker:Sub -1 AAPL {}
              Assets:Cash 45 USD
              Income:Gains
            2023-01-03 * "a short lot at another cost"
              Assets:Short -1 AAPL {12 USD}
              Assets:Cash 12 USD
            2023-01-04 * "nothing at cost"
              Assets:Plain 1 AAPL
              Assets:Cash -10 USD
        "#});

        assert_eq!(balances.balance(&account("Assets:Broker"), "AAPL"), BigDecimal::from(1));
        assert_eq!(balances.balance(&account("Assets:Cash"), "USD"), BigDecimal::from(-52 + 45 + 12 - 10));
        assert_eq!(balances.balance(&account("Income:Gains"), "USD"), BigDecimal::from(-4));
        // the first lot is sold out, the second (a sub-account's) still holds one share
        assert!(balances.holds_at_cost(&account("Assets:Broker"), "AAPL"));
        assert!(balances.holds_at_cost(&account("Assets:Broker:Sub"), "AAPL"));
        assert!(!balances.holds_at_cost(&account("Assets:Broker"), "USD"));
        // a lot sold short is held at cost too; units without a cost are not
        assert!(balances.holds_at_cost(&account("Assets:Short"), "AAPL"));
        assert!(!balances.holds_at_cost(&account("Assets:Plain"), "AAPL"));
        assert!(!balances.holds_at_cost(&account("Assets:Brokerage"), "AAPL"));
    }

    #[test]
    fn should_complete_a_transaction_a_stage_left_unbooked() {
        // the padding transaction the pad stage makes: an implicit leg, which final validation
        // interpolates; completed here the same way
        let balances = fold(indoc! {r#"
            2023-01-01 P "pad"
              Assets:A 7 USD
              Equity:Open
        "#});
        assert_eq!(balances.balance(&account("Assets:A"), "USD"), BigDecimal::from(7));
        assert_eq!(balances.balance(&account("Equity:Open"), "USD"), BigDecimal::from(-7));
    }

    #[test]
    fn should_skip_transactions_final_validation_rejects() {
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

    #[test]
    fn should_restrict_an_account_to_the_commodities_of_its_latest_open() {
        let mut states = AccountStates::default();
        let restricted = |states: &AccountStates, name: &str, commodity: &str| states.commodity_error(&account(name), commodity).is_some();
        for directive in parse(indoc! {r#"
            1970-01-01 open Assets:Bank USD, EUR
            1970-01-01 open Assets:Any
        "#})
        {
            states.apply(&directive);
        }
        assert!(!restricted(&states, "Assets:Bank", "USD"));
        assert!(!restricted(&states, "Assets:Bank", "EUR"));
        assert_eq!(states.commodity_error(&account("Assets:Bank"), "CNY"), Some(ErrorKind::CommodityNotAllowed));
        // an open without commodities, a sub-account and an account never opened are not restricted
        assert!(!restricted(&states, "Assets:Any", "CNY"));
        assert!(!restricted(&states, "Assets:Bank:Sub", "CNY"));
        assert!(!restricted(&states, "Assets:Missing", "CNY"));

        // opened again: the commodities of its latest open count, none lifting the restriction
        for directive in parse(indoc! {r#"
            1970-01-02 close Assets:Bank
            1970-01-03 open Assets:Bank
            1970-01-03 open Assets:Any CNY
        "#})
        {
            states.apply(&directive);
        }
        assert!(!restricted(&states, "Assets:Bank", "CNY"));
        assert!(restricted(&states, "Assets:Any", "USD"));
    }
}
