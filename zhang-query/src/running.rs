//! The running totals behind the `balance` column ([`RunningState`]).
//!
//! The state always keeps the running inventory of the rows added so far. When the optimizer
//! turned `units(balance)` or `cost(balance)` into running sums ([`Running::Units`],
//! [`Running::Cost`]), the state also keeps, per currency, the terms those functions add up,
//! so that reading them costs O(currencies) instead of O(open lots):
//!
//! - `units(balance)` adds up the number of every open lot, per units currency;
//! - `cost(balance)` adds up `units × cost` of every open lot (its units when it has no
//!   cost), per cost currency.
//!
//! Both are what `Inventory::reduce` computes, which adds the terms in lot order and drops an
//! entry whenever its partial sum is zero. When the terms of a currency are all non-zero and
//! of one sign, no partial sum is zero, and the result is their sum written with the largest
//! scale among them, exactly as the sequential additions write it. Otherwise (mixed signs, or
//! a zero term) the currency is reduced from the lots as before. Both give the same decimal,
//! scale included.

use std::collections::BTreeMap;

use bigdecimal::{BigDecimal, Signed, Zero};

use crate::compiler::Running;
use crate::table::{self, Row};
use crate::value::{Inventory, Position};
use crate::Amount;

/// The terms of one currency of a running sum.
#[derive(Default)]
struct Terms {
    /// the exact sum of the terms (its scale is irrelevant)
    total: BigDecimal,
    /// how many terms have each scale
    scales: BTreeMap<i64, usize>,
    positive: usize,
    negative: usize,
    zero: usize,
}

impl Terms {
    fn add(&mut self, term: &BigDecimal) {
        self.total += term;
        *self.scales.entry(term.fractional_digit_count()).or_default() += 1;
        *self.sign_count(term) += 1;
    }

    fn remove(&mut self, term: &BigDecimal) {
        self.total -= term;
        let scale = term.fractional_digit_count();
        if let Some(count) = self.scales.get_mut(&scale) {
            *count -= 1;
            if *count == 0 {
                self.scales.remove(&scale);
            }
        }
        *self.sign_count(term) -= 1;
    }

    fn sign_count(&mut self, term: &BigDecimal) -> &mut usize {
        if term.is_zero() {
            &mut self.zero
        } else if term.is_positive() {
            &mut self.positive
        } else {
            &mut self.negative
        }
    }

    fn is_empty(&self) -> bool {
        self.positive + self.negative + self.zero == 0
    }

    /// The sum as the sequential additions write it, when no partial sum can be zero.
    fn sum(&self) -> Option<BigDecimal> {
        let one_sign = self.zero == 0 && (self.positive == 0 || self.negative == 0);
        let scale = *self.scales.keys().next_back()?;
        one_sign.then(|| self.total.with_scale(scale))
    }
}

/// One running sum of a linear function of the positions, per result currency.
#[derive(Default)]
struct LinearSum {
    currencies: BTreeMap<String, Terms>,
}

impl LinearSum {
    fn change(&mut self, before: Option<Amount>, after: Option<Amount>) {
        if let Some(term) = before {
            if let Some(terms) = self.currencies.get_mut(&term.commodity) {
                terms.remove(&term.number);
                if terms.is_empty() {
                    self.currencies.remove(&term.commodity);
                }
            }
        }
        if let Some(term) = after {
            self.currencies.entry(term.commodity).or_default().add(&term.number);
        }
    }

    /// `f` reduced over the lots of `balance`, as `Inventory::reduce(f)` computes it.
    fn value(&self, balance: &Inventory, f: impl Fn(&Position) -> Amount) -> Inventory {
        let mut result = Inventory::new();
        for (currency, terms) in &self.currencies {
            match terms.sum() {
                Some(sum) => result.add_amount(&Amount::new(sum, currency.clone())),
                None => {
                    // reduce this currency from the lots, in lot order
                    for position in balance.positions() {
                        let term = f(&position);
                        if &term.commodity == currency {
                            result.add_amount(&term);
                        }
                    }
                }
            }
        }
        result
    }
}

/// The term `units(position)` adds for a lot.
fn units_term(position: &Position) -> Amount {
    position.units.clone()
}

/// The term `cost(position)` adds for a lot.
fn cost_term(position: &Position) -> Amount {
    position.at_cost()
}

/// The running totals of one pass over the filtered rows.
pub(crate) struct RunningState {
    balance: Inventory,
    units: Option<LinearSum>,
    cost: Option<LinearSum>,
}

impl RunningState {
    /// A state keeping the `totals`.
    pub fn new(totals: &[Running]) -> Self {
        RunningState {
            balance: Inventory::new(),
            units: totals.contains(&Running::Units).then(LinearSum::default),
            cost: totals.contains(&Running::Cost).then(LinearSum::default),
        }
    }

    /// Add a row that passed the filter.
    pub fn add(&mut self, row: &Row<'_>) {
        let position = table::position(row);
        if self.units.is_none() && self.cost.is_none() {
            self.balance.add_owned_position(position);
            return;
        }
        let key = (position.units.commodity.clone(), position.cost.clone());
        let before = self.balance.lot(&key).cloned();
        self.balance.add_owned_position(position);
        let after = self.balance.lot(&key).cloned();
        let (currency, cost) = key;
        let lot = |number: Option<BigDecimal>| number.map(|number| Position::new(Amount::new(number, currency.clone()), cost.clone()));
        let (before, after) = (lot(before), lot(after));
        for (sum, term) in [(&mut self.units, units_term as fn(&Position) -> Amount), (&mut self.cost, cost_term)] {
            if let Some(sum) = sum {
                sum.change(before.as_ref().map(term), after.as_ref().map(term));
            }
        }
    }

    /// The value of the running `total`.
    pub fn value(&self, total: Running) -> Inventory {
        match total {
            Running::Balance => self.balance.clone(),
            Running::Units => match &self.units {
                Some(sum) => sum.value(&self.balance, units_term),
                None => self.balance.units(),
            },
            Running::Cost => match &self.cost {
                Some(sum) => sum.value(&self.balance, cost_term),
                None => self.balance.at_cost(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::table::MaybeOwned;
    use crate::value::Cost;

    fn row(number: &str, currency: &str, cost: Option<&str>) -> Row<'static> {
        Row {
            entry: 0,
            posting_index: 0,
            account: "Assets:Broker",
            units: MaybeOwned::owned(Amount::new(BigDecimal::from_str(number).unwrap(), currency)),
            cost: cost.map(|cost| {
                MaybeOwned::owned(Cost {
                    number: BigDecimal::from_str(cost).unwrap(),
                    currency: "USD".to_owned(),
                    date: None,
                    label: None,
                })
            }),
            price: None,
            account_balance: None,
        }
    }

    /// After each row, the running sums equal `units()` / `cost()` of the running balance,
    /// down to the scale of every number.
    #[test]
    fn running_sums_write_numbers_like_the_functions_of_the_balance() {
        let mut state = RunningState::new(&[Running::Balance, Running::Units, Running::Cost]);
        let mut seen = vec![];
        for (number, currency, cost) in [
            ("1.000", "STK", Some("10")),
            ("2", "STK", Some("11.5")),
            // the 1.000 lot closes: units(balance) is written with the scale of what is left
            ("-1.000", "STK", Some("10")),
            ("5.00", "USD", None),
            // a negative lot next to a positive one: the sums fall back to the lots
            ("-3", "STK", Some("12")),
            ("1", "STK", Some("0")),
            ("3", "STK", Some("12")),
            ("0.0", "STK", Some("11.5")),
            ("-2", "STK", Some("11.5")),
            ("-1", "STK", Some("0")),
            ("-5.00", "USD", None),
        ] {
            state.add(&row(number, currency, cost));
            let balance = state.value(Running::Balance);
            let units = state.value(Running::Units);
            let cost = state.value(Running::Cost);
            assert_eq!(format!("{:?}", units), format!("{:?}", balance.units()), "{number} {currency}");
            assert_eq!(format!("{:?}", cost), format!("{:?}", balance.at_cost()), "{number} {currency}");
            seen.push(units.to_string());
        }
        assert_eq!(seen[1], "3.000 STK");
        assert_eq!(seen[2], "2 STK");
        assert_eq!(seen[3], "2 STK, 5.00 USD");
        assert_eq!(seen[10], "");
    }
}
