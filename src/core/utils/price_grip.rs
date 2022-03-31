use crate::core::inventory::Currency;
use crate::core::utils::latest_map::LatestMap;
use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct PriceGrip {
    inner: HashMap<Currency, HashMap<Currency, BigDecimal>>,
}

impl PriceGrip {
    pub fn insert(&mut self, from: Currency, to: Currency, amount: BigDecimal) {
        let target_currency_map = self.inner.entry(from).or_insert_with(HashMap::new);
        target_currency_map.insert(to, amount);
    }
}

#[derive(Debug, Clone, Default)]
pub struct DatedPriceGrip {
    inner: LatestMap<NaiveDateTime, PriceGrip>,
}

impl DatedPriceGrip {
    pub fn insert(
        &mut self,
        date: NaiveDateTime,
        from: Currency,
        to: Currency,
        amount: BigDecimal,
    ) {
        if !self.inner.contains_key(&date) {
            self.inner.insert(date, PriceGrip::default());
        }
        if let Some(a) = self.inner.get_mut(&date) {
            a.insert(from, to, amount);
        }
    }
}
