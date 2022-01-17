use crate::amount::Amount;
use crate::models::Account;
use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, Utc};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Add;
use std::str::FromStr;

#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub struct Cost {
    currency: String,
    number: BigDecimal,
    date: DateTime<Utc>,
    label: Option<String>,
}

pub type Currency = String;

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct Position {
    units: Amount,
    cost: Option<Cost>,
}

impl Position {
    pub fn new(units: Amount, cost: Option<Cost>) -> Self {
        Position { units, cost }
    }
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct Inventory {
    data: HashMap<(Currency, Option<Cost>), Position>,
}

pub enum MatchResult {
    Created,
    Reduced,
    Augmented,
    Ignored,
}

impl Inventory {
    pub fn new() -> Self {
        Inventory {
            data: HashMap::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn currencies(&self) -> Vec<Currency> {
        self.data
            .keys()
            .into_iter()
            .map(|(c, _)| c.to_string())
            .collect()
    }
    pub fn cost_currencies(&self) -> Vec<Currency> {
        self.data
            .keys()
            .into_iter()
            .map(|(_, cost)| cost)
            .filter(|cost| cost.is_some())
            .map(|cost| cost.as_ref().map(|it| it.currency.clone()))
            .map(|cost| cost.unwrap())
            .collect()
    }

    pub fn add_position(&mut self, position: Position) {
        self.add_amount(position.units, position.cost);
    }

    pub fn add_amount(&mut self, units: Amount, cost: impl Into<Option<Cost>>) {
        let cost = cost.into();
        let key = (units.currency.clone(), cost.clone());
        let has_pos = self.data.contains_key(&key);

        if has_pos {
            let entry = self.data.entry(key);
            if let Entry::Occupied(mut o) = entry {
                let o_mut = o.get_mut();
                let number = (&o_mut.units.number).add(&units.number);
                if number.is_zero() {
                    o.remove_entry();
                } else {
                    o_mut.units.number = number;
                }
            }
        } else {
            if units.number.is_zero() {
            } else {
                self.data
                    .insert(key, Position::new(units.clone(), cost.clone()));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl FromStr for Inventory {
    type Err = crate::error::AvaroError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // crate::p::parse_avaro()

        todo! {}
    }
}

#[cfg(test)]
mod test {
    use crate::amount::Amount;
    use crate::inventory::{Inventory, Position};

    mod from_string {
        use crate::amount::Amount;
        use crate::inventory::Inventory;
        use bigdecimal::BigDecimal;
        use std::str::FromStr;

        #[test]
        fn test_from_string() {
            let inventory = Inventory::from_str("10 USD").unwrap();
            assert_eq!(inventory, {
                let mut inventory1 = Inventory::new();
                inventory1.add_amount(
                    Amount {
                        number: BigDecimal::from(10i32),
                        currency: "USD".to_string(),
                    },
                    None,
                );
                inventory1
            })
        }
    }

    #[test]
    fn test_ctor_empty_len() {
        let inventory = Inventory::new();
        assert_eq!(0, inventory.len());
        assert_eq!(true, inventory.is_empty());

        let mut inventory1 = Inventory::new();
        inventory1.add_position(Position::new(Amount::new_with_i32(100, "USD"), None));
        inventory1.add_position(Position::new(Amount::new_with_i32(101, "USD"), None));
        assert_eq!(1, inventory1.len());
        assert_eq!(false, inventory1.is_empty());

        let mut inventory2 = Inventory::new();
        inventory2.add_position(Position::new(Amount::new_with_i32(100, "USD"), None));
        inventory2.add_position(Position::new(Amount::new_with_i32(101, "CAD"), None));
        assert_eq!(2, inventory2.len());
        assert_eq!(false, inventory2.is_empty());

        let mut inventory3 = Inventory::new();
        assert_eq!(0, inventory3.len());
        inventory3.add_position(Position::new(Amount::new_with_i32(100, "USD"), None));
        assert_eq!(1, inventory3.len());
        inventory3.add_position(Position::new(Amount::new_with_i32(101, "CAD"), None));
        assert_eq!(2, inventory3.len());
    }
}