//! Price histories shared by the query engine and plugin SDK.
//!
//! A pair keeps the latest price on or before a date, or its latest price with no
//! date. Same-day prices keep the last one. Opposite histories merge into the
//! direction with more points (the first seen on a tie); the other direction reads
//! their inverses, skipping zero prices, each computed the first time it is read.
//! Decimal division and valuation products follow Python's 28 significant digits
//! and half-even rounding.
//!
//! The map reads price points. Callers adapt their inputs and keep their own
//! conversion policies.

use std::collections::HashMap;
use std::sync::OnceLock;

use bigdecimal::{BigDecimal, One, Zero};
use chrono::NaiveDate;

use crate::decimal::div;

type History = Vec<(NaiveDate, BigDecimal)>;

/// Exchange rates between commodities, reconciled into one history per pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PriceMap {
    rates: HashMap<String, HashMap<String, Pair>>,
}

/// The history of one pair, and the inverses of its rates, each computed the first time the opposite pair reads it and
/// kept: a division of 28 digits, which a query converting every posting through the opposite pair would otherwise
/// repeat for each one. Nothing is inverted for a pair never read the other way, so a map of many prices costs no more
/// to build than its history.
#[derive(Clone, Default)]
struct Pair {
    history: History,
    /// `1 / rate` of each point of `history`, `None` for a zero rate; the cells are made at the first such read
    inverses: OnceLock<Box<[OnceLock<Option<BigDecimal>>]>>,
}

impl Pair {
    fn new(history: History) -> Pair {
        Pair {
            history,
            inverses: OnceLock::new(),
        }
    }

    /// the inverse of the rate at `at` of the history
    fn inverse(&self, at: usize) -> Option<&BigDecimal> {
        let inverses = self.inverses.get_or_init(|| self.history.iter().map(|_| OnceLock::new()).collect());
        inverses[at].get_or_init(|| invert(&self.history[at].1)).as_ref()
    }
}

/// Two pairs are equal by their history: the inverses are a function of it, whichever were read.
impl PartialEq for Pair {
    fn eq(&self, other: &Pair) -> bool {
        self.history == other.history
    }
}

impl Eq for Pair {}

/// A pair prints as its history, whichever inverses were read.
impl std::fmt::Debug for Pair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.history.fmt(f)
    }
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
        let mut rates: HashMap<String, HashMap<String, Pair>> = HashMap::new();
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
            rates.entry(base).or_default().insert(quote, Pair::new(deduped));
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
        if let Some(pair) = self.rates.get(base).and_then(|quotes| quotes.get(quote)) {
            return latest(&pair.history, date).cloned();
        }
        // the opposite pair's latest non-zero rate, inverted (a zero rate has no inverse), the inverse computed once
        let pair = self.rates.get(quote).and_then(|bases| bases.get(base))?;
        let end = date.map_or(pair.history.len(), |date| pair.history.partition_point(|(day, _)| *day <= date));
        let at = pair.history[..end].iter().rposition(|(_, rate)| !rate.is_zero())?;
        pair.inverse(at).cloned()
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

/// the latest rate on or before `date`
fn latest(history: &[(NaiveDate, BigDecimal)], date: Option<NaiveDate>) -> Option<&BigDecimal> {
    let end = date.map_or(history.len(), |date| history.partition_point(|(day, _)| *day <= date));
    history[..end].last().map(|(_, rate)| rate)
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
    /// opposite pair is read inverted and when it is merged into a pair quoted both ways.
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

    /// The opposite pair reads the latest non-zero rate, inverted: a zero rate has no inverse, so the one before it
    /// stands, and a pair whose only rates are zero has none. The pair itself keeps its zero rates.
    #[test]
    fn the_opposite_pair_skips_a_zero_rate() {
        let prices = PriceMap::from_points(vec![
            (date("2024-01-01"), "CNY", "USD", d("0")),
            (date("2024-01-10"), "CNY", "USD", d("0.14")),
            (date("2024-02-01"), "CNY", "USD", d("0")),
        ]);
        assert_eq!(prices.rate("USD", "CNY", Some(date("2024-01-05"))), None);
        assert_eq!(prices.rate("USD", "CNY", Some(date("2024-01-10"))), Some(d("7.142857142857142857142857143")));
        assert_eq!(prices.rate("USD", "CNY", Some(date("2024-02-15"))), Some(d("7.142857142857142857142857143")));
        assert_eq!(prices.rate("USD", "CNY", None), Some(d("7.142857142857142857142857143")));
        assert_eq!(prices.rate("CNY", "USD", Some(date("2024-02-15"))), Some(d("0")));
        let only_zero = PriceMap::from_points(vec![(date("2024-01-01"), "CNY", "USD", d("0"))]);
        assert_eq!(only_zero.rate("USD", "CNY", None), None);
    }

    /// An inverse is computed the first time the opposite pair reads it, from whichever thread reads it first, and
    /// kept; a map read that way equals one never read so.
    #[test]
    fn an_inverse_is_computed_once_for_every_reader() {
        let points = || vec![(date("2024-01-01"), "CNY", "USD", d("0.14")), (date("2024-02-01"), "CNY", "USD", d("0.125"))];
        let (read, unread) = (PriceMap::from_points(points()), PriceMap::from_points(points()));
        let rates = std::thread::scope(|scope| {
            let readers = (0..4)
                .map(|_| scope.spawn(|| read.rate("USD", "CNY", Some(date("2024-02-15")))))
                .collect::<Vec<_>>();
            readers.into_iter().map(|it| it.join().unwrap()).collect::<Vec<_>>()
        });
        assert!(rates.iter().all(|it| *it == Some(d("8"))), "{rates:?}");
        assert_eq!(read, unread);
        let copy = read.clone();
        assert_eq!(copy.rate("USD", "CNY", Some(date("2024-01-15"))), Some(d("7.142857142857142857142857143")));
        assert_eq!(copy.rate("USD", "CNY", Some(date("2024-03-01"))).unwrap().to_string(), "8");
    }

    /// The opposite pair reads, digit for digit and scale for scale, the inverse of its latest non-zero rate on or
    /// before the date, as computed on every lookup before the inverses were kept: whatever the order of the reads,
    /// with zero rates, same-day prices and pairs quoted both ways. The map prints and compares as one never read.
    #[test]
    fn a_kept_inverse_reads_as_one_computed_on_lookup() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let pairs = [("USD", "CNY"), ("CNY", "USD"), ("EUR", "USD"), ("AAPL", "USD"), ("USD", "JPY"), ("JPY", "USD")];
        let rates = ["0", "0.1", "0.14", "3", "7.2345", "0.000123", "1250", "0.3333", "0.00", "8"];
        let day = |n: usize| NaiveDate::from_yo_opt(2024, 1 + n as u32).unwrap();
        let points = (0..600)
            .map(|_| {
                let (base, quote) = pairs[next(pairs.len())];
                (day(next(90)), base, quote, d(rates[next(rates.len())]))
            })
            .collect::<Vec<_>>();
        let (prices, unread) = (PriceMap::from_points(points.clone()), PriceMap::from_points(points));
        let printed = format!("{prices:?}");
        let on_lookup = |base: &str, quote: &str, date: Option<NaiveDate>| {
            let history = &unread.rates.get(quote)?.get(base)?.history;
            let end = date.map_or(history.len(), |date| history.partition_point(|(day, _)| *day <= date));
            history[..end].iter().rev().find(|(_, rate)| !rate.is_zero()).and_then(|(_, rate)| invert(rate))
        };
        let commodities = ["USD", "CNY", "EUR", "AAPL", "JPY"];
        let mut read = 0;
        for _ in 0..3000 {
            let (base, quote) = (commodities[next(commodities.len())], commodities[next(commodities.len())]);
            if base == quote || unread.rates.get(base).is_some_and(|quotes| quotes.contains_key(quote)) {
                continue;
            }
            let date = (next(10) > 0).then(|| day(next(100)));
            let expected = on_lookup(base, quote, date).map(|rate| (rate.to_string(), rate.fractional_digit_count()));
            let rate = prices.rate(base, quote, date).map(|rate| (rate.to_string(), rate.fractional_digit_count()));
            assert_eq!(rate, expected, "{base} in {quote} on {date:?}");
            read += usize::from(rate.is_some());
        }
        assert!(read > 300, "{read} inverses read");
        assert_eq!(prices, unread);
        assert_eq!(format!("{prices:?}"), printed);
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
