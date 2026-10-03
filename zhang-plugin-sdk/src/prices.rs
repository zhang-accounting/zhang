//! Exchange rates from the ledger's `price` directives.
//!
//! zhang never precomputes prices for plugins: a processor builds a [`PriceMap`] from the stream it receives.
//! The map follows beancount's price map, and gives exactly the rates zhang's query engine uses for `convert`,
//! `value` and `getprice`, so a plugin's valuations agree with what zhang shows:
//!
//! - the rate of a pair on a date is the **latest price on or before that date**; there is none before the
//!   first price;
//! - prices of one pair on **the same day replace each other**: the last one in stream order wins (the stream is
//!   sorted by date, keeping source order within a day);
//! - a pair without prices of its own uses the **inverse** of the opposite pair, `1 / rate`, skipping zero prices,
//!   which have no inverse;
//! - when a pair is quoted **in both directions**, the two merge into one history, as in beancount: the direction
//!   with more prices is kept and the other one is inverted into it. So the rate on a date is the latest quote in
//!   either direction, and of two quotes on the same day, the one of the direction with fewer prices wins;
//! - the rate of a commodity to itself is 1.
//!
//! # Precision
//!
//! Rates from `price` directives are exact. An inverse rate is exact when it terminates, keeping the scale
//! beancount would (`1 / 8` is `0.125`); otherwise it is rounded half-even to 28 significant digits, Python's
//! default decimal context, which beancount computes in and zhang's query engine mirrors (`1 / 7` is
//! `0.1428571428571428571428571429`). [`PriceMap::convert`] multiplies exactly when the product fits in 28
//! significant digits and rounds it half-even to 28 otherwise, like zhang's `convert()`.
//!
//! # Implicit prices
//!
//! [`PriceMap::from_stream_with_implicit`] also takes the prices written on postings, `@` (per unit) and `@@`
//! (total, divided by the units), like beancount's `implicit_prices` plugin. zhang itself does not, so rates that
//! depend on them differ from what zhang shows. A posting without units is skipped: plugins see transactions
//! before booking, so such a posting carries no price yet.
//!
//! ```
//! use std::str::FromStr;
//!
//! use zhang_plugin_sdk::ast::amount::Amount;
//! use zhang_plugin_sdk::ast::{Date, Directive, Meta, Price, SpanInfo, Spanned};
//! use zhang_plugin_sdk::bigdecimal::BigDecimal;
//! use zhang_plugin_sdk::chrono::NaiveDate;
//! use zhang_plugin_sdk::prices::PriceMap;
//!
//! let day = |text: &str| NaiveDate::from_str(text).unwrap();
//! let number = |text: &str| BigDecimal::from_str(text).unwrap();
//! // 2024-01-01 price USD 8 CNY
//! let price = Directive::Price(Price {
//!     date: Date::Date(day("2024-01-01")),
//!     currency: "USD".to_owned(),
//!     amount: Amount::new(number("8"), "CNY"),
//!     meta: Meta::default(),
//! });
//! let prices = PriceMap::from_stream(&[Spanned::new(price, SpanInfo::default())]);
//!
//! assert_eq!(prices.rate("USD", "CNY", day("2024-03-01")), Some(number("8")));
//! assert_eq!(prices.rate("CNY", "USD", day("2024-03-01")), Some(number("0.125")));
//! assert_eq!(prices.rate("USD", "CNY", day("2023-12-31")), None);
//! let lunch = Amount::new(number("2.50"), "USD");
//! assert_eq!(prices.convert(&lunch, "CNY", day("2024-03-01")), Some(Amount::new(number("20.00"), "CNY")));
//! ```

use std::collections::HashMap;
use std::num::NonZeroU64;

use bigdecimal::{BigDecimal, One, RoundingMode, Zero};
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, SingleTotalPrice, Spanned};

/// the significant digits a rounded division or product keeps: Python's default decimal context
const PRECISION: u64 = 28;

/// a pair's prices, sorted by date, one per date
type History = Vec<(NaiveDate, BigDecimal)>;

/// Exchange rates between commodities, built from `price` directives. See the [module docs](self) for the
/// semantics and the precision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PriceMap {
    /// base → quote → history; only one direction of a pair is kept, the other is inverted on lookup
    rates: HashMap<String, HashMap<String, History>>,
}

impl PriceMap {
    /// the rates of the `price` directives in `stream`
    pub fn from_stream(stream: &[Spanned<Directive>]) -> PriceMap {
        PriceMap::from_points(price_points(stream, false))
    }

    /// the rates of the `price` directives in `stream` and of the `@`/`@@` prices of its postings
    pub fn from_stream_with_implicit(stream: &[Spanned<Directive>]) -> PriceMap {
        PriceMap::from_points(price_points(stream, true))
    }

    /// build from `(date, base, quote, rate)` points in stream order
    fn from_points(points: Vec<(NaiveDate, String, String, BigDecimal)>) -> PriceMap {
        let mut forward: HashMap<(String, String), History> = HashMap::new();
        let mut first_seen: Vec<(String, String)> = vec![];
        for (date, base, quote, rate) in points {
            let key = (base, quote);
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

    /// whether the map has no prices
    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }

    /// The rate converting one unit of `base` into `quote` on `date`: the latest price on or before it, directly
    /// or inverted; 1 when `base` is `quote`; `None` without a price on or before `date`.
    pub fn rate(&self, base: &str, quote: &str, date: NaiveDate) -> Option<BigDecimal> {
        if base == quote {
            return Some(BigDecimal::one());
        }
        if let Some(history) = self.rates.get(base).and_then(|quotes| quotes.get(quote)) {
            return latest(history, date, false).cloned();
        }
        let history = self.rates.get(quote).and_then(|quotes| quotes.get(base))?;
        latest(history, date, true).and_then(invert)
    }

    /// `amount` in `target` at the rate of `date`; `None` without a rate. The number is rounded as the
    /// [module docs](self#precision) say.
    pub fn convert(&self, amount: &Amount, target: &str, date: NaiveDate) -> Option<Amount> {
        let rate = self.rate(&amount.commodity, target, date)?;
        Some(Amount::new(mul_in_context(&amount.number, &rate), target))
    }
}

/// the `(date, base, quote, rate)` points of `stream`, in stream order; with `implicit`, also the prices of postings
fn price_points(stream: &[Spanned<Directive>], implicit: bool) -> Vec<(NaiveDate, String, String, BigDecimal)> {
    let mut points = vec![];
    for directive in stream {
        match &directive.data {
            Directive::Price(price) => points.push((
                price.date.naive_date(),
                price.currency.clone(),
                price.amount.commodity.clone(),
                price.amount.number.clone(),
            )),
            Directive::Transaction(txn) if implicit => {
                for posting in &txn.postings {
                    let (Some(units), Some(price)) = (&posting.units, &posting.price) else {
                        continue;
                    };
                    let per_unit = match price {
                        SingleTotalPrice::Single(price) => Some((price.number.clone(), &price.commodity)),
                        SingleTotalPrice::Total(total) => div(&total.number, &units.number.abs()).map(|number| (number, &total.commodity)),
                    };
                    if let Some((number, commodity)) = per_unit {
                        points.push((txn.date.naive_date(), units.commodity.clone(), commodity.clone(), number));
                    }
                }
            }
            _ => {}
        }
    }
    points
}

/// the latest rate on or before `date`, skipping zero rates if `skip_zero`
fn latest(history: &[(NaiveDate, BigDecimal)], date: NaiveDate, skip_zero: bool) -> Option<&BigDecimal> {
    let end = history.partition_point(|(day, _)| *day <= date);
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

/// `lhs / rhs` in Python's default decimal context, like beancount: an exact quotient keeps the ideal scale
/// `lhs.scale - rhs.scale` when that fits (`10.00 / 2` is `5.00`), an inexact one is rounded half-even to
/// [`PRECISION`] significant digits; `None` for a zero `rhs`. The same computation as zhang's query engine.
fn div(lhs: &BigDecimal, rhs: &BigDecimal) -> Option<BigDecimal> {
    if rhs.is_zero() {
        return None;
    }
    let quotient = lhs / rhs;
    let quotient = if quotient.digits() > PRECISION {
        quotient.with_precision_round(precision(), RoundingMode::HalfEven)
    } else {
        quotient
    };
    let ideal_scale = lhs.fractional_digit_count() - rhs.fractional_digit_count();
    let normalized = quotient.normalized();
    if normalized.fractional_digit_count() < ideal_scale && normalized.digits() + (ideal_scale - normalized.fractional_digit_count()) as u64 <= PRECISION {
        Some(normalized.with_scale(ideal_scale))
    } else {
        Some(normalized)
    }
}

/// `lhs × rhs` keeping the scale `lhs.scale + rhs.scale` (`-1000.00 × 1` is `-1000.00`), rounded half-even to
/// [`PRECISION`] significant digits when it has more
fn mul_in_context(lhs: &BigDecimal, rhs: &BigDecimal) -> BigDecimal {
    let (lhs_digits, lhs_scale) = lhs.as_bigint_and_exponent();
    let (rhs_digits, rhs_scale) = rhs.as_bigint_and_exponent();
    let mut product = BigDecimal::new(lhs_digits * rhs_digits, lhs_scale + rhs_scale);
    // a carry (9.99… → 10.00…) adds a digit; the second pass drops it
    while product.digits() > PRECISION {
        product = product.with_precision_round(precision(), RoundingMode::HalfEven);
    }
    product
}

fn precision() -> NonZeroU64 {
    NonZeroU64::new(PRECISION).expect("the precision is not zero")
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Account, Date, Directive, Flag, Meta, Posting, Price, SingleTotalPrice, SpanInfo, Spanned, Transaction};

    use super::{div, mul_in_context, PriceMap};

    fn day(text: &str) -> NaiveDate {
        NaiveDate::from_str(text).unwrap()
    }

    fn number(text: &str) -> BigDecimal {
        BigDecimal::from_str(text).unwrap()
    }

    fn price(date: &str, base: &str, rate: &str, quote: &str) -> Spanned<Directive> {
        Spanned::new(
            Directive::Price(Price {
                date: Date::Date(day(date)),
                currency: base.to_owned(),
                amount: Amount::new(number(rate), quote),
                meta: Meta::default(),
            }),
            SpanInfo::default(),
        )
    }

    fn posting(account: &str, units: Option<(&str, &str)>, price: Option<SingleTotalPrice>) -> Posting {
        Posting {
            flag: None,
            account: Account::from_str(account).unwrap(),
            units: units.map(|(n, c)| Amount::new(number(n), c)),
            cost: None,
            price,
            comment: None,
            meta: Meta::default(),
        }
    }

    fn transaction(date: &str, postings: Vec<Posting>) -> Spanned<Directive> {
        Spanned::new(
            Directive::Transaction(Transaction {
                date: Date::Date(day(date)),
                flag: Some(Flag::Okay),
                payee: None,
                narration: None,
                tags: Default::default(),
                links: Default::default(),
                postings,
                meta: Meta::default(),
            }),
            SpanInfo::default(),
        )
    }

    #[test]
    fn should_take_the_latest_price_on_or_before_the_date() {
        let prices = PriceMap::from_stream(&[
            price("2024-01-01", "USD", "7", "CNY"),
            price("2024-02-01", "USD", "8", "CNY"),
            price("2024-02-01", "USD", "9", "CNY"),
        ]);
        assert_eq!(prices.rate("USD", "CNY", day("2023-12-31")), None);
        assert_eq!(prices.rate("USD", "CNY", day("2024-01-01")), Some(number("7")));
        assert_eq!(prices.rate("USD", "CNY", day("2024-01-31")), Some(number("7")));
        assert_eq!(prices.rate("USD", "CNY", day("2024-02-01")), Some(number("9")), "the last price of a day wins");
        assert_eq!(prices.rate("CNY", "USD", day("2024-03-01")), Some(number("0.1111111111111111111111111111")));
        assert_eq!(prices.rate("EUR", "EUR", day("1970-01-01")), Some(number("1")));
        assert_eq!(prices.rate("EUR", "CNY", day("2024-03-01")), None);
        assert!(PriceMap::from_stream(&[]).is_empty());
    }

    #[test]
    fn should_merge_a_pair_quoted_in_both_directions() {
        // USD→CNY has more prices, so CNY→USD is inverted into it
        let prices = PriceMap::from_stream(&[
            price("2024-01-01", "USD", "7", "CNY"),
            price("2024-02-01", "CNY", "0.125", "USD"),
            price("2024-03-01", "USD", "7.5", "CNY"),
            price("2024-03-01", "CNY", "0.2", "USD"),
            price("2024-04-01", "USD", "6", "CNY"),
        ]);
        assert_eq!(
            prices.rate("USD", "CNY", day("2024-02-15")),
            Some(number("8")),
            "the latest quote in either direction"
        );
        assert_eq!(
            prices.rate("USD", "CNY", day("2024-03-01")),
            Some(number("5")),
            "the inverted quote comes last on its day"
        );
        assert_eq!(prices.rate("CNY", "USD", day("2024-03-01")), Some(number("0.2")));
        assert_eq!(prices.rate("CNY", "USD", day("2024-04-02")), Some(number("0.1666666666666666666666666667")));
    }

    #[test]
    fn should_skip_zero_prices_when_inverting() {
        let prices = PriceMap::from_stream(&[price("2024-01-01", "OPT", "2", "USD"), price("2024-02-01", "OPT", "0", "USD")]);
        assert_eq!(prices.rate("OPT", "USD", day("2024-03-01")), Some(number("0")));
        assert_eq!(prices.rate("USD", "OPT", day("2024-03-01")), Some(number("0.5")));
    }

    #[test]
    fn should_round_like_python_decimal() {
        assert_eq!(div(&number("1"), &number("8")), Some(number("0.125")));
        assert_eq!(div(&number("10.00"), &number("2")).unwrap().to_string(), "5.00");
        assert_eq!(div(&number("1"), &number("7")).unwrap().to_string(), "0.1428571428571428571428571429");
        assert_eq!(div(&number("1"), &number("0")), None);
        assert_eq!(mul_in_context(&number("2.50"), &number("8")).to_string(), "20.00");
        assert_eq!(
            mul_in_context(&number("3"), &number("0.3333333333333333333333333333")).to_string(),
            "0.9999999999999999999999999999"
        );
        assert_eq!(
            mul_in_context(&number("300"), &number("0.3333333333333333333333333333")).to_string(),
            "99.99999999999999999999999999"
        );
    }

    #[test]
    fn should_convert_or_not() {
        let prices = PriceMap::from_stream(&[price("2024-01-01", "USD", "7.2", "CNY")]);
        let amount = Amount::new(number("40"), "USD");
        assert_eq!(prices.convert(&amount, "CNY", day("2024-01-02")), Some(Amount::new(number("288.0"), "CNY")));
        assert_eq!(prices.convert(&amount, "USD", day("2024-01-02")), Some(amount.clone()));
        assert_eq!(prices.convert(&amount, "EUR", day("2024-01-02")), None);
        assert_eq!(prices.convert(&amount, "CNY", day("2023-01-02")), None);
    }

    #[test]
    fn should_take_implicit_prices_only_when_asked() {
        let stream = [
            price("2024-01-01", "AAPL", "180", "USD"),
            transaction(
                "2024-01-05",
                vec![
                    posting(
                        "Assets:Stock",
                        Some(("10", "AAPL")),
                        Some(SingleTotalPrice::Single(Amount::new(number("185"), "USD"))),
                    ),
                    posting("Assets:Cash", None, None),
                ],
            ),
            transaction(
                "2024-01-06",
                vec![
                    posting(
                        "Assets:Cash",
                        Some(("-4", "EUR")),
                        Some(SingleTotalPrice::Total(Amount::new(number("30"), "CNY"))),
                    ),
                    // no units yet: no price before booking
                    posting("Assets:Bank", None, Some(SingleTotalPrice::Single(Amount::new(number("9"), "CNY")))),
                ],
            ),
        ];
        let explicit = PriceMap::from_stream(&stream);
        assert_eq!(explicit.rate("AAPL", "USD", day("2024-01-10")), Some(number("180")));
        assert_eq!(explicit.rate("EUR", "CNY", day("2024-01-10")), None);

        let implicit = PriceMap::from_stream_with_implicit(&stream);
        assert_eq!(implicit.rate("AAPL", "USD", day("2024-01-10")), Some(number("185")));
        assert_eq!(implicit.rate("AAPL", "USD", day("2024-01-04")), Some(number("180")));
        assert_eq!(
            implicit.rate("EUR", "CNY", day("2024-01-10")),
            Some(number("7.5")),
            "@@ divides the total by the units"
        );
    }
}
