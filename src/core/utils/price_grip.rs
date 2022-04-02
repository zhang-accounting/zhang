use crate::core::utils::latest_map::LatestMap;
use crate::core::Currency;
use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDateTime;
use std::collections::HashMap;
use std::ops::Mul;

#[derive(Debug, Clone, Default)]
pub struct PriceGrip {
    inner: HashMap<Currency, HashMap<Currency, BigDecimal>>,
}

impl PriceGrip {
    pub fn insert(&mut self, from: Currency, to: Currency, amount: BigDecimal) {
        let target_currency_map = self.inner.entry(from).or_insert_with(HashMap::new);
        target_currency_map.insert(to, amount);
    }
    pub fn get(&self, from: &Currency, to: &Currency) -> Option<BigDecimal> {
        self.inner.get(from).and_then(|from_map| from_map.get(to)).cloned()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DatedPriceGrip {
    inner: LatestMap<NaiveDateTime, PriceGrip>,
}

impl DatedPriceGrip {
    pub fn insert(&mut self, date: NaiveDateTime, from: Currency, to: Currency, amount: BigDecimal) {
        if !self.inner.contains_key(&date) {
            self.inner.insert(date, PriceGrip::default());
        }
        if let Some(a) = self.inner.get_mut(&date) {
            a.insert(from, to, amount);
        }
    }

    pub fn calculate(&self, date: &NaiveDateTime, from: &Currency, to: &Currency, number: &BigDecimal) -> BigDecimal {
        if from.eq(to) {
            return number.clone();
        }
        let price = self
            .inner
            .get_latest(date)
            .and_then(|grip| grip.get(from, to))
            .unwrap_or_else(BigDecimal::zero);
        number.mul(price)
    }
}
