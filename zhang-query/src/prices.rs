//! The price map used by valuation functions (`convert`, `value`, `getprice`).

use std::collections::HashMap;

use bigdecimal::{BigDecimal, One, Zero};
use chrono::NaiveDate;
use zhang_core::domains::schemas::PriceDomain;

use crate::decimal;

/// Exchange rates between currency pairs, built from the ledger's `price` directives.
///
/// Semantics follow beancount's price map: for a pair the rate is the latest price on or
/// before the requested date (or the latest overall when no date is given); prices on
/// the same date replace each other (the last one wins); a pair without direct prices
/// uses the inverse of the opposite pair. When both directions are quoted, the direction
/// with fewer price points is inverted and merged into the other one.
#[derive(Debug, Default, Clone)]
pub struct PriceMap {
    /// base -> quote -> sorted (date, rate), one entry per date; only one direction of a
    /// pair is stored and the other one is inverted on lookup
    rates: HashMap<String, HashMap<String, Vec<(NaiveDate, BigDecimal)>>>,
}

impl PriceMap {
    pub fn from_prices<'a>(prices: impl IntoIterator<Item = &'a PriceDomain>) -> Self {
        Self::from_points(
            prices
                .into_iter()
                .map(|price| (price.datetime.date(), price.commodity.as_str(), price.target_commodity.as_str(), &price.amount)),
        )
    }

    /// Build from `(date, base, quote, rate)` points in ledger order.
    pub fn from_points<'a>(points: impl IntoIterator<Item = (NaiveDate, &'a str, &'a str, &'a BigDecimal)>) -> Self {
        let mut forward: HashMap<(String, String), Vec<(NaiveDate, BigDecimal)>> = HashMap::new();
        let mut first_seen: Vec<(String, String)> = vec![];
        for (date, base, quote, rate) in points {
            let key = (base.to_owned(), quote.to_owned());
            if !forward.contains_key(&key) {
                first_seen.push(key.clone());
            }
            forward.entry(key).or_default().push((date, rate.clone()));
        }

        // reconcile pairs quoted in both directions into the direction with more points
        let mut merged: HashMap<(String, String), Vec<(NaiveDate, BigDecimal)>> = HashMap::new();
        for key in first_seen {
            let Some(points) = forward.remove(&key) else {
                continue;
            };
            let inverse_key = (key.1.clone(), key.0.clone());
            match forward.remove(&inverse_key) {
                None => {
                    merged.insert(key, points);
                }
                Some(inverse_points) => {
                    let (keep_key, mut keep, other) = if points.len() >= inverse_points.len() {
                        (key, points, inverse_points)
                    } else {
                        (inverse_key, inverse_points, points)
                    };
                    keep.extend(other.into_iter().filter_map(|(date, rate)| invert(&rate).map(|rate| (date, rate))));
                    merged.insert(keep_key, keep);
                }
            }
        }

        let mut rates: HashMap<String, HashMap<String, Vec<(NaiveDate, BigDecimal)>>> = HashMap::new();
        for ((base, quote), mut points) in merged {
            // stable sort keeps ledger order within a date, so the last price of a day wins
            points.sort_by_key(|(date, _)| *date);
            let mut deduped: Vec<(NaiveDate, BigDecimal)> = Vec::with_capacity(points.len());
            for (date, rate) in points {
                match deduped.last_mut() {
                    Some(last) if last.0 == date => *last = (date, rate),
                    _ => deduped.push((date, rate)),
                }
            }
            rates.entry(base).or_default().insert(quote, deduped);
        }
        PriceMap { rates }
    }

    /// The rate converting one unit of `base` into `quote`, as of `date` (inclusive) or the
    /// latest one when `date` is `None`. The rate of a currency to itself is 1.
    pub fn rate(&self, base: &str, quote: &str, date: Option<NaiveDate>) -> Option<BigDecimal> {
        if base == quote {
            return Some(BigDecimal::one());
        }
        if let Some(points) = self.rates.get(base).and_then(|quotes| quotes.get(quote)) {
            return latest(points, date, false).cloned();
        }
        let points = self.rates.get(quote).and_then(|quotes| quotes.get(base))?;
        // zero prices have no inverse: they are skipped like in beancount's inverted lists
        latest(points, date, true).and_then(invert)
    }

    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }
}

/// The latest point on or before `date` (or the last one), optionally skipping zero rates.
fn latest(points: &[(NaiveDate, BigDecimal)], date: Option<NaiveDate>, skip_zero: bool) -> Option<&BigDecimal> {
    let end = match date {
        None => points.len(),
        Some(date) => points.partition_point(|(point_date, _)| *point_date <= date),
    };
    points[..end].iter().rev().find(|(_, rate)| !skip_zero || !rate.is_zero()).map(|(_, rate)| rate)
}

fn invert(rate: &BigDecimal) -> Option<BigDecimal> {
    if rate.is_zero() {
        None
    } else {
        decimal::div(&BigDecimal::one(), rate)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::from_str(s).unwrap()
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
