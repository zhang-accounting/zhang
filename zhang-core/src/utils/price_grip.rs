use std::collections::HashMap;

use bigdecimal::BigDecimal;
use zhang_ast::Currency;

#[derive(Debug, Clone, Default)]
pub struct PriceGrip {
    inner: HashMap<Currency, HashMap<Currency, BigDecimal>>,
}

impl PriceGrip {
    pub fn insert(&mut self, from: Currency, to: Currency, amount: BigDecimal) {
        let target_currency_map = self.inner.entry(from).or_default();
        target_currency_map.insert(to, amount);
    }
    pub fn get(&self, from: &Currency, to: &Currency) -> Option<BigDecimal> {
        self.inner.get(from).and_then(|from_map| from_map.get(to)).cloned()
    }
}

#[cfg(test)]
mod test {
    mod price_grip {
        use bigdecimal::BigDecimal;

        use crate::utils::price_grip::PriceGrip;

        #[test]
        fn should_insert_price() {
            let mut grip = PriceGrip::default();
            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(10i32));
            assert_eq!(grip.inner.get("USD").unwrap().get("CNY").unwrap(), &BigDecimal::from(10i32));

            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(20i32));
            assert_eq!(grip.inner.get("USD").unwrap().get("CNY").unwrap(), &BigDecimal::from(20i32));
        }

        #[test]
        fn should_get_price() {
            let mut grip = PriceGrip::default();
            grip.insert("USD".to_string(), "CNY".to_string(), BigDecimal::from(7i32));
            assert_eq!(grip.get(&"USD".to_string(), &"CNY".to_string()), Some(BigDecimal::from(7i32)));
            assert_eq!(grip.get(&"USD".to_string(), &"CCY".to_string()), None);
            assert_eq!(grip.get(&"CNY".to_string(), &"USD".to_string()), None);
        }
    }
}
