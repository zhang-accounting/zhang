use crate::core::amount::Amount;
use crate::core::utils::latest_map::LatestMap;
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::{AccountName, Currency};
use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::HashMap;
use std::ops::{Add, AddAssign, Sub};
use std::sync::{Arc, RwLock as StdRwLock};

#[derive(Debug, Clone)]
pub struct Inventory {
    pub(crate) inner: HashMap<Currency, BigDecimal>,
    pub(crate) prices: Arc<StdRwLock<DatedPriceGrip>>,
}

impl Inventory {
    pub fn add_amount(&mut self, amount: Amount) {
        let decimal1 = BigDecimal::zero();
        let x = self.inner.get(&amount.currency).unwrap_or(&decimal1);
        let decimal = (x).add(&amount.number);
        self.inner.insert(amount.currency, decimal);
    }

    pub fn sub_amount(&mut self, amount: Amount) {
        let decimal1 = BigDecimal::zero();
        let x = self.inner.get(&amount.currency).unwrap_or(&decimal1);
        let decimal = (x).sub(&amount.number);
        self.inner.insert(amount.currency, decimal);
    }
    pub fn pin(&self) -> Inventory {
        self.clone()
    }
    pub fn pop(&mut self) -> Option<Amount> {
        self.inner
            .drain()
            .take(1)
            .next()
            .map(|(currency, number)| Amount::new(number, currency))
    }
    pub fn get(&self, currency: &Currency) -> BigDecimal {
        self.inner.get(currency).cloned().unwrap_or_else(BigDecimal::zero)
    }
    pub fn calculate_to_currency(&self, date: NaiveDateTime, currency: &Currency) -> BigDecimal {
        let price_guard = self.prices.read().unwrap();
        let mut sum = BigDecimal::zero();
        for (each_currency, each_number) in &self.inner {
            let decimal = price_guard.calculate(&date, each_currency, currency, each_number);
            sum.add_assign(decimal);
        }
        sum
    }
    pub fn is_zero(&self) -> bool {
        self.inner.iter().all(|pair| pair.1.is_zero())
    }
    pub fn size(&self) -> usize {
        self.inner.len()
    }
}

impl Add for &Inventory {
    type Output = Inventory;

    fn add(self, rhs: Self) -> Self::Output {
        let mut new_inventory = Inventory {
            inner: Default::default(),
            prices: self.prices.clone(),
        };
        for (currency, amount) in &self.inner {
            new_inventory.add_amount(Amount::new(amount.clone(), currency));
        }
        for (currency, amount) in &rhs.inner {
            new_inventory.add_amount(Amount::new(amount.clone(), currency));
        }
        new_inventory
    }
}

impl Sub for &Inventory {
    type Output = Inventory;

    fn sub(self, rhs: Self) -> Self::Output {
        let mut new_inventory = Inventory {
            inner: Default::default(),
            prices: self.prices.clone(),
        };
        for (currency, amount) in &self.inner {
            new_inventory.add_amount(Amount::new(amount.clone(), currency));
        }
        for (currency, amount) in &rhs.inner {
            new_inventory.sub_amount(Amount::new(amount.clone(), currency));
        }
        new_inventory
    }
}

#[derive(Debug, Clone, Default)]
pub struct DailyAccountInventory {
    inner: LatestMap<NaiveDate, HashMap<AccountName, Inventory>>,
}

impl DailyAccountInventory {
    pub(crate) fn insert_account_inventory(
        &mut self, day: NaiveDate, account_inventory_map: HashMap<AccountName, Inventory>,
    ) {
        self.inner.insert(day, account_inventory_map);
    }
    pub(crate) fn get_account_inventory(&self, day: &NaiveDate) -> HashMap<AccountName, Inventory> {
        self.inner.get_latest(day).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod test {
    mod inventory {
        use crate::core::amount::Amount;
        use crate::core::utils::inventory::Inventory;
        use bigdecimal::BigDecimal;
        use std::sync::Arc;

        #[test]
        fn should_add_amount() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(Default::default()),
            };

            inventory.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));
            inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
            assert_eq!(inventory.inner.get("CNY").unwrap(), &BigDecimal::from(1i32));
            assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(3i32));

            inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
            assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(6i32));
        }

        #[test]
        fn should_sub_amount() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(Default::default()),
            };
            inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));

            inventory.sub_amount(Amount::new(BigDecimal::from(1i32), "CNY"));
            assert_eq!(inventory.inner.get("CNY").unwrap(), &BigDecimal::from(-1i32));

            inventory.sub_amount(Amount::new(BigDecimal::from(1i32), "USD"));
            assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(2i32));
        }

        #[test]
        fn should_get_correct_amount() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(Default::default()),
            };
            inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
            inventory.add_amount(Amount::new(BigDecimal::from(10i32), "CNY"));

            assert_eq!(inventory.get(&"USD".to_string()), BigDecimal::from(3i32));
            assert_eq!(inventory.get(&"CNY".to_string()), BigDecimal::from(10i32));
        }
        #[test]
        fn should_pin() {
            let mut inventory = Inventory {
                inner: Default::default(),
                prices: Arc::new(Default::default()),
            };
            inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));

            let other = inventory.pin();
            inventory.add_amount(Amount::new(BigDecimal::from(4i32), "USD"));

            assert_eq!(inventory.get(&"USD".to_string()), BigDecimal::from(7i32));
            assert_eq!(other.get(&"USD".to_string()), BigDecimal::from(3i32));
        }
    }
}
