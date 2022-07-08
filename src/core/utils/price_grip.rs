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
    pub(crate) inner: LatestMap<NaiveDateTime, PriceGrip>,
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

#[cfg(test)]
mod test {
    mod price_grip {
        use crate::core::utils::price_grip::PriceGrip;
        use bigdecimal::BigDecimal;

        #[test]
        fn should_insert_price() {
            let mut grip = PriceGrip::default();
            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(10i32));
            assert_eq!(
                grip.inner.get("USD").unwrap().get("CNY").unwrap(),
                &BigDecimal::from(10i32)
            );

            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(20i32));
            assert_eq!(
                grip.inner.get("USD").unwrap().get("CNY").unwrap(),
                &BigDecimal::from(20i32)
            );
        }

        #[test]
        fn should_get_price() {
            let mut grip = PriceGrip::default();
            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(7i32));
            assert_eq!(
                grip.get(&"USD".to_string(), &"CNY".to_string()),
                Some(BigDecimal::from(7i32))
            );
            assert_eq!(grip.get(&"USD".to_string(), &"CCY".to_string()), None);
            assert_eq!(grip.get(&"CNY".to_string(), &"USD".to_string()), None);
        }
    }
    mod dated_price_grip {
        use crate::core::utils::price_grip::DatedPriceGrip;
        use bigdecimal::BigDecimal;
        use chrono::NaiveDateTime;

        #[test]
        fn should_insert_price_grip() {
            let mut dpg = DatedPriceGrip::default();
            dpg.insert(
                NaiveDateTime::from_timestamp(0, 0),
                "USD".to_string(),
                "CNY".to_string(),
                BigDecimal::from(7i32),
            );
            assert!(dpg.inner.contains_key(&NaiveDateTime::from_timestamp(0, 0)));
            assert_eq!(
                dpg.inner
                    .get_latest(&NaiveDateTime::from_timestamp(0, 0))
                    .unwrap()
                    .get(&"USD".to_string(), &"CNY".to_string())
                    .unwrap(),
                BigDecimal::from(7i32)
            )
        }
        #[test]
        fn should_return_it_given_same_from_to_currency() {
            let dpg = DatedPriceGrip::default();
            assert_eq!(
                dpg.calculate(
                    &NaiveDateTime::from_timestamp(0, 0),
                    &"CNY".to_string(),
                    &"CNY".to_string(),
                    &BigDecimal::from(100i32)
                ),
                BigDecimal::from(100i32)
            );
        }
        #[test]
        fn should_calculate_to_target_currency() {
            let mut dpg = DatedPriceGrip::default();
            dpg.insert(
                NaiveDateTime::from_timestamp(0, 0),
                "USD".to_string(),
                "CNY".to_string(),
                BigDecimal::from(7i32),
            );

            assert_eq!(
                dpg.calculate(
                    &NaiveDateTime::from_timestamp(0, 0),
                    &"USD".to_string(),
                    &"CNY".to_string(),
                    &BigDecimal::from(100i32)
                ),
                BigDecimal::from(700i32)
            );

            assert_eq!(
                dpg.calculate(
                    &NaiveDateTime::from_timestamp(1000, 0),
                    &"USD".to_string(),
                    &"CNY".to_string(),
                    &BigDecimal::from(100i32)
                ),
                BigDecimal::from(700i32)
            );
        }
    }
}
