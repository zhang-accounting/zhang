use std::collections::HashMap;
use std::ops::{Add, AddAssign};
use std::str::FromStr;

use bigdecimal::{BigDecimal, One, Signed, Zero};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use indexmap::IndexMap;

use crate::amount::Amount;
use crate::error::ErrorKind;
use crate::Currency;

pub type AmountLotPair = (Option<Amount>, Option<LotInfo>);

// todo: remove lot info
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum LotInfo {
    Lot(Currency, BigDecimal),
    Fifo,
    Filo,
}

pub enum BookingMethod {
    STRICT,
    FIFO,
    LIFO,
    AVERAGE,
    AVERAGE_ONLY,
    NONE,
}

impl FromStr for BookingMethod {
    type Err = ErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "STRICT" => Ok(BookingMethod::STRICT),
            "FIFO" => Ok(BookingMethod::FIFO),
            "LIFO" => Ok(BookingMethod::LIFO),
            "AVERAGE" => Ok(BookingMethod::AVERAGE),
            "AVERAGE_ONLY" => Ok(BookingMethod::AVERAGE_ONLY),
            "NONE" => Ok(BookingMethod::NONE),
            _ => Err(ErrorKind::ParseInvalidMeta),
        }
    }
}

/// retrieve the lot meta info from posting
pub struct LotMeta {
    pub txn_date: NaiveDate,

    pub cost: Option<Amount>,
    pub cost_date: Option<NaiveDate>,
    pub price: Option<Amount>,
}

#[derive(Clone, Debug)]
pub struct CommodityInventory {
    currency: String,
    pub total: BigDecimal,
    pub lots: IndexMap<(Currency, BigDecimal), BigDecimal>,
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
            LotInfo::Fifo => {
                let mut number = number.clone();
                self.total.add_assign(&number);
                for (_, amount) in self.lots.iter_mut() {
                    if number.is_zero() {
                        break;
                    };
                    if (amount as &BigDecimal).add(&number).is_negative() {
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
            LotInfo::Filo => {
                let mut number = number.clone();
                self.total.add_assign(&number);
                for (_, amount) in self.lots.iter_mut().rev() {
                    if number.is_zero() {
                        break;
                    };
                    if (amount as &BigDecimal).add(&number).is_negative() {
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
    pub currencies: HashMap<Currency, CommodityInventory>,
}

impl Inventory {
    pub fn add_lot(&mut self, amount: Amount, lot: LotInfo) {
        let commodity_inventory = self.currencies.entry(amount.currency).or_insert_with_key(|key| CommodityInventory::new(key));
        commodity_inventory.insert(&amount.number, lot);
    }

    pub(crate) fn pop(&mut self) -> Option<Amount> {
        self.currencies
            .drain()
            .take(1)
            .next()
            .map(|(currency, currency_inventory)| Amount::new(currency_inventory.total, currency))
    }

    pub fn get_total(&self, currency: &Currency) -> BigDecimal {
        self.currencies.get(currency).map(|it| it.total.clone()).unwrap_or_else(BigDecimal::zero)
    }

    pub fn is_zero(&self) -> bool {
        self.currencies.iter().all(|pair| pair.1.total.is_zero())
    }
    pub fn size(&self) -> usize {
        self.currencies.len()
    }
}
