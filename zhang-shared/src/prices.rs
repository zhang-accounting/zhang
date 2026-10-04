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

/// `1 / rate`; `None` for a zero rate
fn invert(rate: &BigDecimal) -> Option<BigDecimal> {
    div(&BigDecimal::one(), rate)
}
