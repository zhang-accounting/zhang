use std::collections::HashMap;
use std::ops::AddAssign;
use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use strum::Display;

use crate::amount::Amount;
use crate::error::ErrorKind;
use crate::{Currency, PostingCost};

pub type AmountLotPair = (Option<Amount>, Option<Amount>);

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Clone, Copy, Display)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum BookingMethod {
    Strict,
    Fifo,
    Lifo,
    Average,
    AverageOnly,
    None,
}

impl FromStr for BookingMethod {
    type Err = ErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "STRICT" => Ok(BookingMethod::Strict),
            "FIFO" => Ok(BookingMethod::Fifo),
            "LIFO" => Ok(BookingMethod::Lifo),
            "AVERAGE" => Ok(BookingMethod::Average),
            "AVERAGE_ONLY" => Ok(BookingMethod::AverageOnly),
            "NONE" => Ok(BookingMethod::None),
            _ => Err(ErrorKind::ParseInvalidMeta),
        }
    }
}

/// retrieve the lot meta info from posting
#[derive(Debug)]
pub struct LotMeta {
    pub txn_date: NaiveDate,

    pub cost: Option<PostingCost>,
    pub price: Option<Amount>,
}

impl LotMeta {
    pub fn is_default_lot(&self) -> bool {
        self.cost.is_none() && self.price.is_none()
    }
}

/// Inventory likes a warehouse to record how many commodities are used, and how much are they.
/// 
/// And for investment tracing purpose, we need to record more details about how much we brought the commodity, and when.
/// That's why we need to use `lots` to record the info.
#[derive(Debug, Clone)]
pub struct Inventory {
    pub currencies: HashMap<Currency, BigDecimal>,
}

impl Inventory {
    pub fn add_amount(&mut self, amount: Amount) {
        let target_commodity_amount = self.currencies.entry(amount.currency).or_default();
        target_commodity_amount.add_assign(amount.number);
    }

    pub(crate) fn pop(&mut self) -> Option<Amount> {
        self.currencies
            .drain()
            .take(1)
            .next()
            .map(|(currency, currency_inventory)| Amount::new(currency_inventory, currency))
    }

    pub fn get_total(&self, currency: &Currency) -> BigDecimal {
        self.currencies.get(currency).cloned().unwrap_or_else(BigDecimal::zero)
    }

    pub fn is_zero(&self) -> bool {
        self.currencies.values().all(|it| it.is_zero())
    }
    pub fn size(&self) -> usize {
        self.currencies.values().filter(|it| !it.is_zero()).collect_vec().len()
    }
}
