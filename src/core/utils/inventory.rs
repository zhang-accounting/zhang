use crate::core::amount::Amount;
use crate::core::utils::latest_map::LatestMap;
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::{AccountName, Currency};
use bigdecimal::{BigDecimal, One, Signed, Zero};
use chrono::{NaiveDate, NaiveDateTime};
use indexmap::IndexMap;
use std::collections::{BTreeMap, HashMap};
use std::ops::Neg;
use std::ops::{Add, AddAssign, Sub};
use std::sync::{Arc, RwLock as StdRwLock};

#[derive(Debug, PartialEq)]
pub enum LotInfo {
    Lot(Currency, BigDecimal),
    FIFO,
    FILO,
}

#[derive(Clone, Debug)]
pub struct CommodityInventory {
    currency: String,
    pub(crate) total: BigDecimal,
    pub(crate) lots: IndexMap<(Currency, BigDecimal), BigDecimal>,
}

impl CommodityInventory {
    pub fn new(currency: impl Into<String>) -> CommodityInventory {
        let currency = currency.into();
        let mut inventory = Self {
            currency: currency.clone(),
            total: BigDecimal::zero(),
            lots: IndexMap::new(),
        };
        inventory.insert(&BigDecimal::zero(), LotInfo::Lot(currency, BigDecimal::one()));
        inventory
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
                self.total.add_assign(&number);
                for (_, amount) in self.lots.iter_mut() {
                    if number.is_zero() {
                        break;
                    };
                    if (&amount as &BigDecimal).add(&number).is_negative() {
                        number.add_assign(amount as &BigDecimal);
                        *amount = BigDecimal::zero();
                    } else {
                        amount.add_assign(&number);
                        number = BigDecimal::zero();
                    };
                }
                if !number.is_zero() {
                    let target_lots = self.lots.entry((self.currency.clone(), BigDecimal::one())).or_default();
                    target_lots.add_assign(number);
                }
            }
            LotInfo::FILO => {
                let mut number = number.clone();
                self.total.add_assign(&number);
                for (_, amount) in self.lots.iter_mut().rev() {
                    if number.is_zero() {
                        break;
                    };
                    if (&amount as &BigDecimal).add(&number).is_negative() {
                        number.add_assign(amount as &BigDecimal);
                        *amount = BigDecimal::zero();
                    } else {
                        amount.add_assign(&number);
                        number = BigDecimal::zero();
                    };
                }
                if !number.is_zero() {
                    let target_lots = self.lots.entry((self.currency.clone(), BigDecimal::one())).or_default();
                    target_lots.add_assign(number);
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
    pub fn add_lot(&mut self, amount: Amount, lot: LotInfo) {
        let commodity_inventory = self
            .currencies
            .entry(amount.currency)
            .or_insert_with_key(|key| CommodityInventory::new(key));
        commodity_inventory.insert(&amount.number, lot);
        dbg!(&commodity_inventory);
    }

    pub fn pin(&self) -> Inventory {
        self.clone()
    }
    pub(crate) fn pop(&mut self) -> Option<Amount> {
        self.currencies
            .drain()
            .take(1)
            .next()
            .map(|(currency, currency_inventory)| Amount::new(currency_inventory.total, currency))
    }

    pub fn get_total(&self, currency: &Currency) -> BigDecimal {
        self.currencies.get(currency).map(|it|it.total.clone()).unwrap_or_else(BigDecimal::zero)
    }

    pub fn calculate_to_currency(&self, date: NaiveDateTime, currency: &Currency) -> BigDecimal {
        let price_guard = self.prices.read().unwrap();
        let mut sum = BigDecimal::zero();
        for (each_currency, currency_inventory) in &self.currencies {
            let decimal = price_guard.calculate(&date, each_currency, currency, &currency_inventory.total);
            sum.add_assign(decimal);
        }
        sum
    }
    pub fn is_zero(&self) -> bool {
        self.currencies.iter().all(|pair| pair.1.total.is_zero())
    }
    pub fn size(&self) -> usize {
        self.currencies.len()
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

        for (currency, currency_inventory) in &self.currencies {
            for (lot_info, number) in currency_inventory.lots.iter() {
                new_inventory.add_lot(
                    Amount::new(number.clone(), currency.clone()),
                    LotInfo::Lot(lot_info.0.clone(), lot_info.1.clone()),
                )
            }
        }
        for (currency, currency_inventory) in &rhs.currencies {
            for (lot_info, number) in currency_inventory.lots.iter() {
                new_inventory.add_lot(
                    Amount::new(number.clone(), currency.clone()),
                    LotInfo::Lot(lot_info.0.clone(), lot_info.1.clone()),
                )
            }
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
        for (currency, currency_inventory) in &self.currencies {
            for (lot_info, number) in currency_inventory.lots.iter() {
                new_inventory.add_lot(
                    Amount::new(number.clone(), currency.clone()),
                    LotInfo::Lot(lot_info.0.clone(), lot_info.1.clone()),
                )
            }
        }
        for (currency, currency_inventory) in &rhs.currencies {
            for (lot_info, number) in currency_inventory.lots.iter() {
                new_inventory.add_lot(
                    Amount::new(number.neg(), currency.clone()),
                    LotInfo::Lot(lot_info.0.clone(), lot_info.1.clone()),
                )
            }
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
            let mut inventory = CommodityInventory::new("USD");
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
            let mut inventory = CommodityInventory::new("USD");
            inventory.insert(
                &BigDecimal::one(),
                LotInfo::Lot("CNY".to_string(), BigDecimal::from(7i32)),
            );
            inventory.insert(
                &BigDecimal::from(11i32),
                LotInfo::Lot("CNY".to_string(), BigDecimal::from(8i32)),
            );
            assert_eq!(BigDecimal::from(12i32), inventory.total);
            assert_eq!(3, inventory.lots.len());
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
