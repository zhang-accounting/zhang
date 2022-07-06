use crate::core::amount::Amount;
use crate::core::utils::latest_map::LatestMap;
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::{AccountName, Currency};
use bigdecimal::{BigDecimal, Signed, Zero};
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::{BTreeMap, HashMap};
use std::ops::{Add, AddAssign, Sub};
use std::sync::{Arc, RwLock as StdRwLock};
use indexmap::IndexMap;

#[derive(Debug, PartialEq)]
pub enum LotInfo {
    Lot(Currency, BigDecimal),
    FIFO,
    FILO,
}

#[derive(Clone, Debug)]
pub struct CommodityInventory {
    total: BigDecimal,
    lots: IndexMap<(Currency, BigDecimal), BigDecimal>,
}

impl CommodityInventory {
    pub fn new() -> CommodityInventory {
        Self {
            total: BigDecimal::zero(),
            lots: IndexMap::new(),
        }
    }
    pub fn insert(&mut self, number: &BigDecimal, lot_info: LotInfo) {
        match lot_info {
            LotInfo::Lot(target_currency, lot_number) => {
                let target_lots = self.lots.entry((target_currency, lot_number)).or_default();
                target_lots.add_assign(number);
                // todo check if the sign is negative

                self.total.add_assign(number);
            }
            LotInfo::FIFO => {
                let mut number = number.clone();
                for (_, amount) in self.lots.iter_mut() {
                    if number.is_zero() {
                        break;
                    };
                    if (&amount as &BigDecimal).add(&number).is_negative() {
                        number.add_assign(amount as &BigDecimal);
                        *amount = BigDecimal::zero();
                    } else {
                        amount.add_assign(&number);
                    };
                }
            }
            LotInfo::FILO => {
                let mut number = number.clone();
                for (_, amount) in self.lots.iter_mut().rev() {
                    if number.is_zero() {
                        break;
                    };
                    if (&amount as &BigDecimal).add(&number).is_negative() {
                        number.add_assign(amount as &BigDecimal);
                        *amount = BigDecimal::zero();
                    } else {
                        amount.add_assign(&number);
                    };
                }
            }
        }
    }
}

/// Inventory likes a warehouse to record how many commodities are used, and how much are they.
/// And for investment tracing purpose, we need to record more details about how much we brought the commodity, and when.
/// That's why we need to use `lots` to record the info.
#[derive(Debug, Clone)]
pub struct Inventory {
    #[deprecated]
    pub(crate) inner: HashMap<Currency, BigDecimal>,
    #[deprecated]
    pub(crate) lots: HashMap<Currency, HashMap<(Currency, BigDecimal), BigDecimal>>,
    #[deprecated]
    pub(crate) summaries: HashMap<Currency, BigDecimal>,

    pub(crate) currencies: HashMap<Currency, CommodityInventory>,
    pub(crate) prices: Arc<StdRwLock<DatedPriceGrip>>,
}

impl Inventory {
    pub fn add_lot(&mut self, currency: Currency, lot: (Currency, Option<BigDecimal>), number: BigDecimal) {
        // add lot into lots
        // let lot_detail = self.lots.entry(currency).or_insert_with(Default::default);
        // let lot_balance = lot_detail.entry(lot).or_insert_with(|| BigDecimal::zero());
        // lot_balance.add_assign(number);
        // // todo check the lot balance, use for the negative posting operation, once the substitution due to negative balance then will raise a error
        // if lot_balance.is_negative() {
        //
        // }
    }

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
            lots: Default::default(),
            summaries: Default::default(),
            currencies: Default::default(),
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
            lots: Default::default(),
            summaries: Default::default(),
            currencies: Default::default(),
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
    mod commodities_inventory {
        use crate::core::utils::inventory::{CommodityInventory, LotInfo};
        use bigdecimal::{BigDecimal, One};

        #[test]
        fn should_record_into_lots_given_one_lot() {
            let mut inventory = CommodityInventory::new();
            inventory.insert(
                &BigDecimal::one(),
                LotInfo::Lot("CNY".to_string(), BigDecimal::from(7i32)),
            );
            assert_eq!(BigDecimal::from(1i32), inventory.total);
            assert_eq!(
                &BigDecimal::from(1i32),
                inventory
                    .lots
                    .get(&("CNY".to_string(), BigDecimal::from(7i32)))
                    .unwrap()
            );
        }
        #[test]
        fn should_record_with_multiple_lots() {
            let mut inventory = CommodityInventory::new();
            inventory.insert(
                &BigDecimal::one(),
                LotInfo::Lot("CNY".to_string(), BigDecimal::from(7i32)),
            );
            inventory.insert(
                &BigDecimal::from(11i32),
                LotInfo::Lot("CNY".to_string(), BigDecimal::from(8i32)),
            );
            assert_eq!(BigDecimal::from(12i32), inventory.total);
            assert_eq!(2, inventory.lots.len());
            assert_eq!(
                &BigDecimal::from(1i32),
                inventory
                    .lots
                    .get(&("CNY".to_string(), BigDecimal::from(7i32)))
                    .unwrap()
            );
            assert_eq!(
                &BigDecimal::from(11i32),
                inventory
                    .lots
                    .get(&("CNY".to_string(), BigDecimal::from(8i32)))
                    .unwrap()
            );
        }
    }
    // mod inventory {
    //     use crate::core::amount::Amount;
    //     use crate::core::utils::inventory::Inventory;
    //     use bigdecimal::BigDecimal;
    //     use std::sync::Arc;
    //
    //     #[test]
    //     fn should_add_amount() {
    //         let mut inventory = Inventory {
    //             inner: Default::default(),
    //             lots: Default::default(),
    //             summaries: Default::default(),
    //             prices: Arc::new(Default::default()),
    //         };
    //
    //         inventory.add_amount(Amount::new(BigDecimal::from(1i32), "CNY"));
    //         inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
    //         assert_eq!(inventory.inner.get("CNY").unwrap(), &BigDecimal::from(1i32));
    //         assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(3i32));
    //
    //         inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
    //         assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(6i32));
    //     }
    //
    //     #[test]
    //     fn should_sub_amount() {
    //         let mut inventory = Inventory {
    //             inner: Default::default(),
    //             lots: Default::default(),
    //             summaries: Default::default(),
    //             prices: Arc::new(Default::default()),
    //         };
    //         inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
    //
    //         inventory.sub_amount(Amount::new(BigDecimal::from(1i32), "CNY"));
    //         assert_eq!(inventory.inner.get("CNY").unwrap(), &BigDecimal::from(-1i32));
    //
    //         inventory.sub_amount(Amount::new(BigDecimal::from(1i32), "USD"));
    //         assert_eq!(inventory.inner.get("USD").unwrap(), &BigDecimal::from(2i32));
    //     }
    //
    //     #[test]
    //     fn should_get_correct_amount() {
    //         let mut inventory = Inventory {
    //             inner: Default::default(),
    //             lots: Default::default(),
    //             summaries: Default::default(),
    //             prices: Arc::new(Default::default()),
    //         };
    //         inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
    //         inventory.add_amount(Amount::new(BigDecimal::from(10i32), "CNY"));
    //
    //         assert_eq!(inventory.get(&"USD".to_string()), BigDecimal::from(3i32));
    //         assert_eq!(inventory.get(&"CNY".to_string()), BigDecimal::from(10i32));
    //     }
    //     #[test]
    //     fn should_pin() {
    //         let mut inventory = Inventory {
    //             inner: Default::default(),
    //             lots: Default::default(),
    //             summaries: Default::default(),
    //             prices: Arc::new(Default::default()),
    //         };
    //         inventory.add_amount(Amount::new(BigDecimal::from(3i32), "USD"));
    //
    //         let other = inventory.pin();
    //         inventory.add_amount(Amount::new(BigDecimal::from(4i32), "USD"));
    //
    //         assert_eq!(inventory.get(&"USD".to_string()), BigDecimal::from(7i32));
    //         assert_eq!(other.get(&"USD".to_string()), BigDecimal::from(3i32));
    //     }
    // }
}
