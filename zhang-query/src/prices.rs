//! The price map used by valuation (`convert`, `value`, `getprice`), and conversion of
//! positions and inventories with it ([`Position::convert`], [`Inventory::convert`]).
//!
//! Conversion follows beancount's `convert` module as beanquery uses it: a direct market rate
//! from the units currency to the target (inverse rates included), else for a position held at
//! cost the implied two-step rate units → cost currency → target (the book cost is never used),
//! else the units unchanged. Rates are the latest on or before the date, or the latest overall
//! without one. Products with market rates are rounded like Python's default decimal context
//! (28 significant digits, half-even), so results match beanquery even with inverted rates; see
//! [`crate::decimal`].

use std::sync::{Arc, PoisonError};

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_core::domains::schemas::PriceDomain;
use zhang_core::ledger::Ledger;
use zhang_shared::prices::Conversion;

use crate::decimal::mul_in_context as mul;
use crate::table::LedgerCache;
use crate::value::{Inventory, Position};

/// Exchange rates between currency pairs, built from the ledger's `price` directives.
///
/// Semantics follow beancount's price map: for a pair the rate is the latest price on or
/// before the requested date (or the latest overall when no date is given); prices on
/// the same date replace each other (the last one wins); a pair without direct prices
/// uses the inverse of the opposite pair. When both directions are quoted, the direction
/// with fewer price points is inverted and merged into the other one.
#[derive(Debug, Default, Clone)]
pub struct PriceMap(zhang_shared::prices::PriceMap);

impl PriceMap {
    /// The price map of a loaded ledger: every `price` directive of its store. It takes the
    /// store's read lock, so do not call it while holding the write lock.
    pub fn for_ledger(ledger: &Ledger) -> Self {
        let store = ledger.store.read().unwrap_or_else(PoisonError::into_inner);
        Self::from_prices(&store.prices)
    }

    /// The price map the queries of a loaded ledger value with: built once per ledger, kept in
    /// its cache and shared, so a caller can keep valuing with it, with the same prices as its
    /// queries, after releasing the ledger. It takes the store's read lock, so do not call it
    /// while holding the write lock.
    pub fn cached(ledger: &Ledger) -> Arc<PriceMap> {
        let store = ledger.store.read().unwrap_or_else(PoisonError::into_inner);
        LedgerCache::of(ledger, &store).shared_prices(&store).clone()
    }

    pub fn from_prices<'a>(prices: impl IntoIterator<Item = &'a PriceDomain>) -> Self {
        Self::from_points(
            prices
                .into_iter()
                .map(|price| (price.datetime.date(), price.commodity.as_str(), price.target_commodity.as_str(), &price.amount)),
        )
    }

    /// Build from `(date, base, quote, rate)` points in ledger order.
    pub fn from_points<'a>(points: impl IntoIterator<Item = (NaiveDate, &'a str, &'a str, &'a BigDecimal)>) -> Self {
        Self(zhang_shared::prices::PriceMap::from_points(
            points.into_iter().map(|(date, base, quote, rate)| (date, base, quote, rate.clone())),
        ))
    }

    /// The rate on or before `date`, or the latest without a date.
    pub fn rate(&self, base: &str, quote: &str, date: Option<NaiveDate>) -> Option<BigDecimal> {
        self.0.rate(base, quote, date)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// beancount `convert.convert_amount`: `units` converted with a direct market rate, else with
/// the implied rate through `via` (skipped when `via` is the target), else unchanged.
pub(crate) fn convert_units(units: &Amount, target: &str, via: Option<&str>, prices: &PriceMap, date: Option<NaiveDate>) -> Amount {
    match prices.0.conversion(&units.commodity, target, via, date) {
        Some(Conversion::Rate(rate)) => Amount::new(mul(&units.number, &rate), target),
        // two roundings, like beancount's `number * rate1 * rate2`
        Some(Conversion::Via(rate1, rate2)) => Amount::new(mul(&mul(&units.number, &rate1), &rate2), target),
        None => units.clone(),
    }
}

impl Position {
    /// The units converted to `target` at the market rates of `prices` as of `date` (the latest
    /// rates when `None`), as BQL's `convert(position, target, date)` does: a direct or inverse
    /// rate, else through the cost currency of a position held at cost, else the units
    /// unchanged. A position without cost converts like an amount.
    pub fn convert(&self, target: &str, prices: &PriceMap, date: Option<NaiveDate>) -> Amount {
        let via = self.cost.as_ref().map(|cost| cost.currency.as_str());
        convert_units(&self.units, target, via, prices, date)
    }
}

impl Inventory {
    /// Every position [converted](Position::convert) to `target`, summed per currency: BQL's
    /// `convert(inventory, target, date)`. Positions without a rate keep their units, so the
    /// result may hold other currencies than `target`.
    pub fn convert(&self, target: &str, prices: &PriceMap, date: Option<NaiveDate>) -> Inventory {
        self.reduce(|position| position.convert(target, prices, date))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bigdecimal::One;

    use super::*;
    use crate::decimal;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::from_str(s).unwrap()
    }

    fn position(number: &str, currency: &str, cost: Option<(&str, &str)>) -> Position {
        let cost = cost.map(|(number, currency)| crate::value::Cost {
            number: BigDecimal::from_str(number).unwrap(),
            currency: currency.to_owned(),
            date: None,
            label: None,
        });
        Position::new(Amount::new(BigDecimal::from_str(number).unwrap(), currency), cost)
    }

    /// The public valuation API: direct and inverse rates, the cost currency as a step, the
    /// rates as of a date, and units without a rate kept as they are.
    #[test]
    fn positions_and_inventories_convert_like_bql() {
        let rates = ["150", "160", "7", "1.1", "4"].map(|it| BigDecimal::from_str(it).unwrap());
        let prices = PriceMap::from_points(vec![
            (date("2024-01-01"), "AAPL", "USD", &rates[0]),
            (date("2024-02-01"), "AAPL", "USD", &rates[1]),
            (date("2024-01-01"), "USD", "CNY", &rates[2]),
            (date("2024-01-01"), "EUR", "USD", &rates[3]),
            (date("2024-01-01"), "XYZ", "EUR", &rates[4]),
        ]);
        let lot = position("10", "AAPL", Some(("100", "USD")));
        assert_eq!(lot.convert("USD", &prices, None).to_string(), "1600 USD");
        assert_eq!(lot.convert("USD", &prices, Some(date("2024-01-31"))).to_string(), "1500 USD");
        // no AAPL→CNY rate: through the cost currency, AAPL→USD→CNY
        assert_eq!(lot.convert("CNY", &prices, None).to_string(), "11200 CNY");
        // no rate at all before the first price
        assert_eq!(lot.convert("CNY", &prices, Some(date("2023-12-31"))).to_string(), "10 AAPL");
        // only EUR→USD is quoted: USD→EUR uses its inverse, rounded to 28 significant digits
        assert_eq!(
            position("1.10", "USD", None).convert("EUR", &prices, None).to_string(),
            "1.000000000000000000000000000 EUR"
        );
        let inventory = Inventory::from_iter([lot, position("5", "XYZ", Some(("2", "EUR"))), position("-1011.00", "USD", None)]);
        assert_eq!(inventory.convert("USD", &prices, None).to_string(), "611.00 USD");
        assert_eq!(inventory.convert("CNY", &prices, None).to_string(), "4123.00 CNY, 5 XYZ");
    }

    #[test]
    fn latest_rate_on_or_before_date_and_inverse() {
        let r1 = BigDecimal::from_str("7").unwrap();
        let r2 = BigDecimal::from_str("8").unwrap();
        let r3 = BigDecimal::from_str("9").unwrap();
        let map = PriceMap::from_points(vec![
            (date("2024-01-01"), "USD", "CNY", &r1),
            (date("2024-02-01"), "USD", "CNY", &r2),
            (date("2024-02-01"), "USD", "CNY", &r3),
        ]);
        assert_eq!(map.rate("USD", "CNY", None), Some(r3.clone()));
        assert_eq!(map.rate("USD", "CNY", Some(date("2024-01-15"))), Some(r1));
        assert_eq!(map.rate("USD", "CNY", Some(date("2023-12-31"))), None);
        assert_eq!(map.rate("CNY", "USD", Some(date("2024-03-01"))), decimal::div(&BigDecimal::one(), &r3));
        assert_eq!(map.rate("CNY", "CNY", None), Some(BigDecimal::one()));
        assert_eq!(map.rate("EUR", "CNY", None), None);
    }
}
