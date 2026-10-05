//! An independent computation of the budget figures, to check the budget endpoints against: it
//! reads the parsed directives of a ledger, not the store nor the query engine, and follows the
//! rules of the budget pages as the docs state them (#479 decision 8):
//!
//! - a budget exists from its first `budget` directive on; the directives of a budget that does
//!   not exist yet, and the postings before its definition, are not its own;
//! - activity adds up the postings of the budget's accounts, negated on Income, Liabilities and
//!   Equity accounts, each converted to the budget's commodity at its date; `budget-add` and
//!   `budget-transfer` amounts likewise. A posting is the activity of the budgets the account's
//!   `open` in effect at its date names with `budget:`, each once: an account closed and
//!   opened again with other budgets counts in those from its reopening on;
//! - a conversion uses the latest `price` on or before the date (the last of a day wins): the
//!   pair itself, else its inverse, else through the cost currency of a posting held at cost;
//!   an amount no price converts is left out;
//! - each month starts with what was available at the end of the previous one, and a budget is
//!   closed from the month of its first `budget-close`. It takes no activity after it: a
//!   posting dated after the day of a `budget-close` without a time, or after the time of one
//!   with a time, is not its own (#499).
//!
//! The one posting of a transaction written without units balances the others: its units are
//! the negated sum of their weights (units, times the cost or the price when there is one),
//! which must be in one commodity; otherwise the reference says it cannot compute.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Months, NaiveDate};
use zhang_ast::amount::Amount;
use zhang_ast::{AccountType, Date, Directive, SingleTotalPrice, Transaction};
use zhang_core::ledger::Ledger;

/// A budget's figures in a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Figures {
    pub assigned: BigDecimal,
    pub activity: BigDecimal,
    pub available: BigDecimal,
    pub closed: bool,
}

struct ReferenceBudget {
    commodity: String,
    /// the first day of the month of the definition
    first: NaiveDate,
    /// the first day of the month of the first `budget-close`
    closed_from: Option<NaiveDate>,
    /// the date of the first `budget-close`, as written
    close: Option<Date>,
    /// by first day of the month
    added: BTreeMap<NaiveDate, BigDecimal>,
    activity: BTreeMap<NaiveDate, BigDecimal>,
}

/// The budgets of a ledger, as the reference computes them.
pub(crate) struct Reference {
    budgets: HashMap<String, ReferenceBudget>,
    /// the postings of the budgets' accounts in ledger order, by account: the narration, the
    /// units, and the account's balance in their commodity after them
    postings: HashMap<String, Vec<ReferencePosting>>,
}

/// A posting of a budget's account, as the reference sees it.
pub(crate) struct ReferencePosting {
    pub date: NaiveDate,
    /// the date of its transaction, as written, with its time if any
    pub written: Date,
    /// the budgets of the account's `open` in effect at the posting's date
    pub budgets: BTreeSet<String>,
    pub account: String,
    pub narration: Option<String>,
    pub units: Amount,
    /// the account's balance in the commodity of `units` after the posting
    pub after: BigDecimal,
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first day")
}

/// Whether a transaction dated `date` comes after the `budget-close` dated `close`: after its
/// day without a time, after its time with one.
fn after_close(close: Option<&Date>, date: &Date) -> bool {
    let wall_clock = |date: &Date| match date {
        Date::Date(day) => day.and_hms_opt(0, 0, 0).expect("midnight exists"),
        Date::DateHour(datetime) | Date::Datetime(datetime) => *datetime,
    };
    match close {
        None => false,
        Some(Date::Date(day)) => date.naive_date() > *day,
        Some(close) => wall_clock(date) > wall_clock(close),
    }
}

/// The prices of the ledger: per (commodity, currency), the rate of each date, the last of a
/// day winning.
struct Prices(HashMap<(String, String), BTreeMap<NaiveDate, BigDecimal>>);

impl Prices {
    fn rate(&self, from: &str, to: &str, date: NaiveDate) -> Option<BigDecimal> {
        let at = |from: &str, to: &str| {
            self.0
                .get(&(from.to_owned(), to.to_owned()))
                .and_then(|rates| rates.range(..=date).next_back())
                .map(|(_, rate)| rate.clone())
        };
        at(from, to).or_else(|| at(to, from).filter(|rate| !rate.is_zero()).map(|rate| BigDecimal::from(1) / rate))
    }

    /// `number` of `currency`, held at a cost in `cost_currency` if any, in `target` at `date`.
    fn convert(&self, number: &BigDecimal, currency: &str, cost_currency: Option<&str>, target: &str, date: NaiveDate) -> Option<BigDecimal> {
        if currency == target {
            return Some(number.clone());
        }
        if let Some(rate) = self.rate(currency, target, date) {
            return Some(number * rate);
        }
        let via = cost_currency?;
        let first = self.rate(currency, via, date)?;
        let second = if via == target { BigDecimal::from(1) } else { self.rate(via, target, date)? };
        Some(number * first * second)
    }
}

/// The units of every posting of a transaction: as written, or for the one posting without
/// units, the negated sum of the weights of the others.
fn units(transaction: &Transaction) -> Result<Vec<Amount>, String> {
    let mut weights: BTreeMap<String, BigDecimal> = BTreeMap::new();
    for posting in transaction.postings.iter() {
        let Some(units) = &posting.units else { continue };
        let weight = match (&posting.cost, &posting.price) {
            (Some(cost), _) if cost.base.is_some() => {
                let base = cost.base.as_ref().expect("checked");
                let number = if cost.total {
                    base.number.clone() * units.number.sign_number()
                } else {
                    &base.number * &units.number
                };
                Amount::new(number, base.commodity.clone())
            }
            (_, Some(SingleTotalPrice::Single(price))) => Amount::new(&price.number * &units.number, price.commodity.clone()),
            (_, Some(SingleTotalPrice::Total(price))) => Amount::new(price.number.clone() * units.number.sign_number(), price.commodity.clone()),
            _ => units.clone(),
        };
        *weights.entry(weight.commodity).or_insert_with(BigDecimal::zero) += weight.number;
    }
    let inferred = || -> Result<Amount, String> {
        let weights = weights.iter().filter(|(_, number)| !number.is_zero()).collect::<Vec<_>>();
        match weights.as_slice() {
            [(commodity, number)] => Ok(Amount::new(-(*number).clone(), (*commodity).clone())),
            _ => Err(format!(
                "the units of a posting on {} are inferred from several commodities",
                transaction.date.naive_date()
            )),
        }
    };
    transaction
        .postings
        .iter()
        .map(|posting| match &posting.units {
            Some(units) => Ok(units.clone()),
            None => inferred(),
        })
        .collect()
}

trait SignNumber {
    /// 1 or -1, the sign of the number
    fn sign_number(&self) -> BigDecimal;
}

impl SignNumber for BigDecimal {
    fn sign_number(&self) -> BigDecimal {
        if self < &BigDecimal::zero() {
            BigDecimal::from(-1)
        } else {
            BigDecimal::from(1)
        }
    }
}

impl Reference {
    /// The reference of a ledger, or why it cannot compute one.
    pub(crate) fn of(ledger: &Ledger) -> Result<Reference, String> {
        let mut prices: HashMap<(String, String), BTreeMap<NaiveDate, BigDecimal>> = HashMap::new();
        for directive in &ledger.directives {
            if let Directive::Price(price) = &directive.data {
                prices
                    .entry((price.currency.clone(), price.amount.commodity.clone()))
                    .or_default()
                    .insert(price.date.naive_date(), price.amount.number.clone());
            }
        }
        let prices = Prices(prices);
        // account -> the budgets of its `open` in effect, as the directives are walked in order
        let mut owners: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut postings: HashMap<String, Vec<ReferencePosting>> = HashMap::new();
        let mut balances: HashMap<(String, String), BigDecimal> = HashMap::new();
        let mut budgets: HashMap<String, ReferenceBudget> = HashMap::new();
        let add = |budgets: &mut HashMap<String, ReferenceBudget>, name: &str, number: &BigDecimal, currency: &str, date: NaiveDate| {
            if let Some(budget) = budgets.get_mut(name) {
                if let Some(value) = prices.convert(number, currency, None, &budget.commodity, date) {
                    *budget.added.entry(first_of_month(date)).or_insert_with(BigDecimal::zero) += value;
                }
            }
        };
        for directive in &ledger.directives {
            match &directive.data {
                Directive::Open(open) => {
                    let names = open.meta.get_all("budget");
                    // a later open replaces the budgets of an earlier one
                    owners.insert(open.account.name().to_owned(), names.iter().map(|name| name.as_str().to_owned()).collect());
                }
                Directive::Budget(budget) => {
                    budgets.entry(budget.name.clone()).or_insert_with(|| ReferenceBudget {
                        commodity: budget.commodity.clone(),
                        first: first_of_month(budget.date.naive_date()),
                        closed_from: None,
                        close: None,
                        added: BTreeMap::new(),
                        activity: BTreeMap::new(),
                    });
                }
                Directive::BudgetAdd(it) => add(&mut budgets, &it.name, &it.amount.number, &it.amount.commodity, it.date.naive_date()),
                Directive::BudgetTransfer(it) => {
                    if budgets.contains_key(&it.from) && budgets.contains_key(&it.to) {
                        let date = it.date.naive_date();
                        add(&mut budgets, &it.from, &-it.amount.number.clone(), &it.amount.commodity, date);
                        add(&mut budgets, &it.to, &it.amount.number, &it.amount.commodity, date);
                    }
                }
                Directive::BudgetClose(it) => {
                    if let Some(budget) = budgets.get_mut(&it.name) {
                        if budget.close.is_none() {
                            budget.closed_from = Some(first_of_month(it.date.naive_date()));
                            budget.close = Some(it.date.clone());
                        }
                    }
                }
                Directive::Transaction(transaction) => {
                    let date = transaction.date.naive_date();
                    let mut units = None;
                    for (idx, posting) in transaction.postings.iter().enumerate() {
                        let Some(names) = owners.get(posting.account.name()).filter(|names| !names.is_empty()) else {
                            continue;
                        };
                        if units.is_none() {
                            units = Some(self::units(transaction)?);
                        }
                        let units = &units.as_ref().expect("computed")[idx];
                        let account = posting.account.name().to_owned();
                        let balance = balances.entry((account.clone(), units.commodity.clone())).or_insert_with(BigDecimal::zero);
                        *balance += &units.number;
                        postings.entry(account.clone()).or_default().push(ReferencePosting {
                            date,
                            written: transaction.date.clone(),
                            budgets: names.clone(),
                            account,
                            narration: transaction.narration.as_ref().map(|it| it.as_str().to_owned()).filter(|it| !it.is_empty()),
                            units: units.clone(),
                            after: balance.clone(),
                        });
                        // the defined budgets the posting counts in: none after its close
                        let names = names
                            .iter()
                            .filter(|name| budgets.get(*name).is_some_and(|budget| !after_close(budget.close.as_ref(), &transaction.date)))
                            .collect::<Vec<_>>();
                        let cost_currency = posting.cost.as_ref().and_then(|cost| cost.base.as_ref()).map(|cost| cost.commodity.as_str());
                        let negated = matches!(
                            posting.account.account_type,
                            AccountType::Income | AccountType::Liabilities | AccountType::Equity
                        );
                        for name in names {
                            let budget = budgets.get_mut(name).expect("defined");
                            if let Some(value) = prices.convert(&units.number, &units.commodity, cost_currency, &budget.commodity, date) {
                                let value = if negated { -value } else { value };
                                *budget.activity.entry(first_of_month(date)).or_insert_with(BigDecimal::zero) += value;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Reference { budgets, postings })
    }

    /// The postings of a budget in the month of `month`: those of the accounts whose `open` in
    /// effect at their date names it, but none after the budget's close.
    pub(crate) fn month_postings(&self, name: &str, month: NaiveDate) -> Vec<&ReferencePosting> {
        let month = first_of_month(month);
        let close = self.budgets.get(name).and_then(|budget| budget.close.as_ref());
        self.postings
            .values()
            .flatten()
            .filter(|posting| first_of_month(posting.date) == month && posting.budgets.contains(name) && !after_close(close, &posting.written))
            .collect()
    }

    /// The number of months of every budget from its first through the month of `month`: the
    /// rows `#budgets` generates for a query of the months up to it.
    pub(crate) fn months_until(&self, month: NaiveDate) -> u64 {
        let index = |date: NaiveDate| i64::from(date.year()) * 12 + i64::from(date.month0());
        self.budgets
            .values()
            .map(|budget| u64::try_from(index(month) - index(budget.first) + 1).unwrap_or(0))
            .sum()
    }

    /// The figures of a budget in the month of `month`, carried over from month to month; `None`
    /// for an unknown budget or a month before its first.
    pub(crate) fn figures(&self, name: &str, month: NaiveDate) -> Option<Figures> {
        let budget = self.budgets.get(name)?;
        let month = first_of_month(month);
        if month < budget.first {
            return None;
        }
        let mut available = BigDecimal::zero();
        let mut current = budget.first;
        loop {
            let added = budget.added.get(&current).cloned().unwrap_or_default();
            let activity = budget.activity.get(&current).cloned().unwrap_or_default();
            let assigned = &available + added;
            available = &assigned - &activity;
            if current == month {
                return Some(Figures {
                    assigned,
                    activity,
                    available,
                    closed: budget.closed_from.is_some_and(|from| from <= month),
                });
            }
            current = current.checked_add_months(Months::new(1))?;
        }
    }
}
