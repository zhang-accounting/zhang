//! Price histories shared by the query engine and plugin SDK.
//!
//! A pair keeps the latest price on or before a date, or its latest price with no
//! date. Same-day prices keep the last one. Opposite histories merge into the
//! direction with more points (the first seen on a tie); the other direction is
//! inverted on lookup, skipping zero prices. Decimal division and valuation
//! products follow Python's 28 significant digits and half-even rounding.
//!
//! The map reads price points. Callers adapt their inputs and keep their own
//! conversion policies.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, One, Zero};
use chrono::NaiveDate;

use crate::decimal::div;

type History = Vec<(NaiveDate, BigDecimal)>;

/// Exchange rates between commodities, reconciled into one history per pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PriceMap {
    rates: HashMap<String, HashMap<String, History>>,
}

impl PriceMap {
    /// build from `(date, base, quote, rate)` points in stream order
    pub fn from_points<S: Into<String>>(points: impl IntoIterator<Item = (NaiveDate, S, S, BigDecimal)>) -> Self {
        let mut forward: HashMap<(String, String), History> = HashMap::new();
        let mut first_seen: Vec<(String, String)> = vec![];
        for (date, base, quote, rate) in points {
            let key = (base.into(), quote.into());
            if !forward.contains_key(&key) {
                first_seen.push(key.clone());
            }
            forward.entry(key).or_default().push((date, rate));
        }

        // a pair quoted in both directions keeps the direction with more prices (the first seen on a tie), and the
        // other one is inverted into it, after the kept prices
        let mut rates: HashMap<String, HashMap<String, History>> = HashMap::new();
        for key in first_seen {
            let Some(points) = forward.remove(&key) else {
                continue;
            };
            let inverse_key = (key.1.clone(), key.0.clone());
            let ((base, quote), mut history) = match forward.remove(&inverse_key) {
                None => (key, points),
                Some(inverse_points) => {
                    let (keep_key, mut keep, other) = if points.len() >= inverse_points.len() {
                        (key, points, inverse_points)
                    } else {
                        (inverse_key, inverse_points, points)
                    };
                    keep.extend(other.into_iter().filter_map(|(date, rate)| invert(&rate).map(|rate| (date, rate))));
                    (keep_key, keep)
                }
            };
            // a stable sort keeps stream order within a day, so the last price of the day wins
            history.sort_by_key(|(date, _)| *date);
            let mut deduped: History = Vec::with_capacity(history.len());
            for (date, rate) in history {
                match deduped.last_mut() {
                    Some(last) if last.0 == date => last.1 = rate,
                    _ => deduped.push((date, rate)),
                }
            }
            rates.entry(base).or_default().insert(quote, deduped);
        }
        PriceMap { rates }
    }

    /// Whether the map has no prices.
    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }

    /// The latest rate on or before `date`, or the latest overall without a date.
    /// A commodity's rate to itself is one; a missing rate is `None`.
    pub fn rate(&self, base: &str, quote: &str, date: Option<NaiveDate>) -> Option<BigDecimal> {
        if base == quote {
            return Some(BigDecimal::one());
        }
        if let Some(history) = self.rates.get(base).and_then(|quotes| quotes.get(quote)) {
            return latest(history, date, false).cloned();
        }
        let history = self.rates.get(quote).and_then(|quotes| quotes.get(base))?;
        latest(history, date, true).and_then(invert)
    }

    /// The rates that convert `from` to `to` on `date` (the latest ones without a date), as beancount's
    /// `convert_amount` picks them: the market rate of the pair, or of its inverse, else, for units held at a
    /// cost in `via`, the two rates from `from` to `via` and from `via` to `to` (skipped when `via` is `to`).
    /// `None` when no price converts it.
    pub fn conversion(&self, from: &str, to: &str, via: Option<&str>, date: Option<NaiveDate>) -> Option<Conversion> {
        if let Some(rate) = self.rate(from, to, date) {
            return Some(Conversion::Rate(rate));
        }
        let via = via.filter(|via| *via != to)?;
        Some(Conversion::Via(self.rate(from, via, date)?, self.rate(via, to, date)?))
    }
}

/// How [`PriceMap::conversion`] converts units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversion {
    /// with the market rate of the pair
    Rate(BigDecimal),
    /// through the cost currency, with two rates
    Via(BigDecimal, BigDecimal),
}

/// the latest rate on or before `date`, skipping zero rates if `skip_zero`
fn latest(history: &[(NaiveDate, BigDecimal)], date: Option<NaiveDate>, skip_zero: bool) -> Option<&BigDecimal> {
    let end = date.map_or(history.len(), |date| history.partition_point(|(day, _)| *day <= date));
    history[..end]
        .iter()
        .rev()
        .find(|(_, rate)| !skip_zero || !rate.is_zero())
        .map(|(_, rate)| rate)
}

/// `1 / rate`; `None` for a zero rate. A round quotient (`1 / 0.1`) comes out of the division
/// with a negative scale, which prints as `1E+1`: it gets the scale 0, so it reads `10` wherever
/// the rate is shown.
fn invert(rate: &BigDecimal) -> Option<BigDecimal> {
    div(&BigDecimal::one(), rate).map(|rate| if rate.fractional_digit_count() < 0 { rate.with_scale(0) } else { rate })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::from_str(s).unwrap()
    }

    fn d(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).unwrap()
    }

    /// A round inverse rate is written as a plain number, not in E notation, both when the
    /// opposite pair is inverted on lookup and when it is merged into a pair quoted both ways.
    #[test]
    fn a_round_inverse_rate_has_no_negative_scale() {
        let only_inverse = PriceMap::from_points(vec![(date("2024-01-01"), "CNY", "USD", d("0.1"))]);
        assert_eq!(only_inverse.rate("USD", "CNY", None).unwrap().to_string(), "10");
        assert_eq!(only_inverse.rate("CNY", "USD", None).unwrap().to_string(), "0.1");
        let both_ways = PriceMap::from_points(vec![
            (date("2024-01-01"), "USD", "CNY", d("7.00")),
            (date("2024-02-01"), "CNY", "USD", d("0.01")),
        ]);
        assert_eq!(both_ways.rate("USD", "CNY", None).unwrap().to_string(), "100");
        assert_eq!(both_ways.rate("USD", "CNY", Some(date("2024-01-15"))).unwrap().to_string(), "7.00");
        // an inexact inverse keeps Python's 28 significant digits
        let inexact = PriceMap::from_points(vec![(date("2024-01-01"), "CNY", "USD", d("0.14"))]);
        assert_eq!(inexact.rate("USD", "CNY", None).unwrap().to_string(), "7.142857142857142857142857143");
    }

    /// A conversion takes the pair's rate, else goes through the cost currency, and needs a price on or before
    /// the date.
    #[test]
    fn a_conversion_takes_the_pair_else_the_cost_currency() {
        let prices = PriceMap::from_points(vec![(date("2024-01-10"), "USD", "CNY", d("7")), (date("2024-01-10"), "AAPL", "USD", d("200"))]);
        let on = Some(date("2024-01-10"));
        assert_eq!(prices.conversion("CNY", "CNY", None, on), Some(Conversion::Rate(d("1"))));
        assert_eq!(prices.conversion("USD", "CNY", None, on), Some(Conversion::Rate(d("7"))));
        assert_eq!(prices.conversion("AAPL", "CNY", Some("USD"), on), Some(Conversion::Via(d("200"), d("7"))));
        assert_eq!(prices.conversion("AAPL", "CNY", None, on), None);
        assert_eq!(prices.conversion("AAPL", "USD", Some("USD"), on), Some(Conversion::Rate(d("200"))));
        // no price yet
        assert_eq!(prices.conversion("USD", "CNY", None, Some(date("2024-01-09"))), None);
        assert_eq!(prices.conversion("EUR", "CNY", Some("CNY"), on), None);
    }
}
