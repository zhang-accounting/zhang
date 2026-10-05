//! Decimal arithmetic shared by booking, query evaluation and plugin valuation.
//!
//! # Rounding policy
//!
//! zhang computes like beancount, in Python's default decimal context (28 significant digits,
//! round-half-even), so the numbers zhang derives are those beancount derives:
//! - Sums and differences are exact, and so is [`mul`], the product a query's `*` computes.
//! - Division has to round: [`div`] rounds to 28 significant digits, so `1/3` is
//!   `0.3333333333333333333333333333`. The per-unit share of a total ([`per_unit`]: the per-unit
//!   cost of a total cost, the per-unit price of a total price) is such a quotient. Booking keeps
//!   a lot at exactly that cost, so the cost a query shows is the cost a sale can name.
//! - Weights (units × a per-unit cost or price), the cost of a position and valuations at market
//!   rates use [`mul_in_context`]. It is exact whenever the product fits in 28 digits, which every
//!   product of written numbers does in practice, and otherwise rounds like beancount: a per-unit
//!   cost or an inverted rate carries 28 significant digits, so `7 × 17/7` weighs
//!   `17.00000000000000000000000000`, as in beancount, not `17.000000000000000000000000003`.
//!
//! # Writing a number
//!
//! A number zhang writes as text, in the API, in error metas, in plugin payloads or in an exported ledger, is written
//! by [`plain_decimal`]: in plain notation, never with an exponent, and with its scale (`0.00` stays `0.00`).
//! `BigDecimal`'s own `Display` and `Serialize` write a very small or very large number with an exponent (`1E-7`);
//! a serialized decimal field uses the [`plain`] functions instead.
//!
//! `BigDecimal`'s own `*` normalises when an operand is one (`-1000.00 × 1` becomes `-1000`),
//! and its own `/` divides at 100 digits, so ledger numbers are multiplied and divided through
//! this module: [`mul`] and [`mul_in_context`] keep the scale `lhs.scale + rhs.scale`, and [`div`]
//! keeps the ideal scale of an exact quotient.

use std::num::NonZeroU64;

use bigdecimal::{BigDecimal, RoundingMode, Zero};

/// Number of significant digits kept by [`div`] and [`mul_in_context`] (Python's default
/// context).
pub const DIVISION_PRECISION: u64 = 28;

/// Divide `lhs` by `rhs`, returning `None` on division by zero.
///
/// Exact quotients keep the "ideal" scale `lhs.scale - rhs.scale` (so `10.00 / 2` is
/// `5.00`); inexact quotients are rounded to [`DIVISION_PRECISION`] significant digits.
pub fn div(lhs: &BigDecimal, rhs: &BigDecimal) -> Option<BigDecimal> {
    if rhs.is_zero() {
        return None;
    }
    let quotient = lhs / rhs;
    let quotient = if quotient.digits() > DIVISION_PRECISION {
        quotient.with_precision_round(NonZeroU64::new(DIVISION_PRECISION).expect("non zero precision"), RoundingMode::HalfEven)
    } else {
        quotient
    };
    let ideal_scale = lhs.fractional_digit_count() - rhs.fractional_digit_count();
    let normalized = quotient.normalized();
    if normalized.fractional_digit_count() < ideal_scale
        && normalized.digits() + (ideal_scale - normalized.fractional_digit_count()) as u64 <= DIVISION_PRECISION
    {
        Some(normalized.with_scale(ideal_scale))
    } else {
        Some(normalized)
    }
}

/// The per-unit share of `total` spread over `units`: `total / |units|` in the division context
/// ([`div`]), the way beancount turns a total cost (`{{T}}`) or a total price (`@@ T`) into a
/// per-unit one. `None` for zero units, which have no per-unit share.
pub fn per_unit(total: &BigDecimal, units: &BigDecimal) -> Option<BigDecimal> {
    div(total, &units.abs())
}

/// Multiply exactly. The product keeps the scale `lhs.scale + rhs.scale`
/// (`-1000.00 × 1 = -1000.00`); see the module docs for the rounding policy.
pub fn mul(lhs: &BigDecimal, rhs: &BigDecimal) -> BigDecimal {
    let (lhs_int, lhs_scale) = lhs.as_bigint_and_exponent();
    let (rhs_int, rhs_scale) = rhs.as_bigint_and_exponent();
    BigDecimal::new(lhs_int * rhs_int, lhs_scale + rhs_scale)
}

/// Multiply in Python's default decimal context: exact when the product fits in
/// [`DIVISION_PRECISION`] significant digits, otherwise rounded half-even to that many.
/// Used for valuation at market rates; see the module docs.
pub fn mul_in_context(lhs: &BigDecimal, rhs: &BigDecimal) -> BigDecimal {
    let mut product = mul(lhs, rhs);
    let precision = NonZeroU64::new(DIVISION_PRECISION).expect("non zero precision");
    // a carry (9.99… → 10.00…) adds a digit; the second pass drops the extra trailing zero
    while product.digits() > DIVISION_PRECISION {
        product = product.with_precision_round(precision, RoundingMode::HalfEven);
    }
    product
}

/// A decimal written out in plain notation, which both zhang and beancount read: never with an exponent, as
/// `BigDecimal`'s `Display` writes a very small or very large number (`1E-9`), and keeping its scale (`-12.50`)
pub fn plain_decimal(value: &BigDecimal) -> String {
    let (digits, scale) = value.as_bigint_and_exponent();
    let negative = digits.sign() == bigdecimal::num_bigint::Sign::Minus;
    let digits = digits.magnitude().to_string();
    let mut plain = String::with_capacity(digits.len() + 3);
    if negative {
        plain.push('-');
    }
    if scale <= 0 {
        plain.push_str(&digits);
        if digits != "0" {
            plain.extend(std::iter::repeat_n('0', scale.unsigned_abs() as usize));
        }
    } else {
        let scale = scale as usize;
        if digits.len() > scale {
            let (integer, fraction) = digits.split_at(digits.len() - scale);
            plain.push_str(integer);
            plain.push('.');
            plain.push_str(fraction);
        } else {
            plain.push_str("0.");
            plain.extend(std::iter::repeat_n('0', scale - digits.len()));
            plain.push_str(&digits);
        }
    }
    plain
}

/// `serialize_with` functions that write a decimal field as a string in plain notation ([`plain_decimal`]), where
/// `BigDecimal`'s own `Serialize` writes its `Display` (`"1E-7"`). Deserializing reads either form
pub mod plain {
    use std::collections::{BTreeMap, HashMap};

    use bigdecimal::BigDecimal;
    use serde::{Serialize, Serializer};

    use super::plain_decimal;

    /// a decimal, as `"0.0000001"`
    pub fn serialize<S: Serializer>(value: &BigDecimal, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&plain_decimal(value))
    }

    /// an optional decimal, as a string or `null`
    pub fn serialize_option<S: Serializer>(value: &Option<BigDecimal>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(&Plain(value)),
            None => serializer.serialize_none(),
        }
    }

    /// a map of decimals, each as a string
    pub fn serialize_map<K: Serialize, S: Serializer>(map: &HashMap<K, BigDecimal>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(map.iter().map(|(key, value)| (key, Plain(value))))
    }

    /// an ordered map of decimals, each as a string
    pub fn serialize_ordered_map<K: Serialize, S: Serializer>(map: &BTreeMap<K, BigDecimal>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(map.iter().map(|(key, value)| (key, Plain(value))))
    }

    /// a decimal that serializes in plain notation
    struct Plain<'a>(&'a BigDecimal);

    impl Serialize for Plain<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serialize(self.0, serializer)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn d(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).unwrap()
    }

    #[test]
    fn division_follows_python_decimal_context() {
        assert_eq!(div(&d("1"), &d("3")).unwrap().to_string(), "0.3333333333333333333333333333");
        assert_eq!(div(&d("3"), &d("2")).unwrap().to_string(), "1.5");
        assert_eq!(div(&d("10.00"), &d("2")).unwrap().to_string(), "5.00");
        assert_eq!(div(&d("2"), &d("3")).unwrap().to_string(), "0.6666666666666666666666666667");
        assert_eq!(div(&d("1"), &d("0")), None);
    }

    #[test]
    fn should_write_a_decimal_without_an_exponent() {
        for (number, plain) in [
            ("0.000000001", "0.000000001"),
            ("1E-9", "0.000000001"),
            ("-0.0000000125", "-0.0000000125"),
            ("123456789012345678901234567890", "123456789012345678901234567890"),
            ("1.2E+30", "1200000000000000000000000000000"),
            ("1.234567890123456789", "1.234567890123456789"),
            ("-12.50", "-12.50"),
            ("0", "0"),
            ("0.00", "0.00"),
            ("100", "100"),
        ] {
            assert_eq!(plain_decimal(&d(number)), plain, "{number}");
        }
    }

    #[test]
    fn a_per_unit_share_divides_over_the_absolute_units() {
        assert_eq!(per_unit(&d("100"), &d("3")).unwrap().to_string(), "33.33333333333333333333333333");
        assert_eq!(per_unit(&d("100"), &d("-3")).unwrap().to_string(), "33.33333333333333333333333333");
        assert_eq!(per_unit(&d("1000.00"), &d("10")).unwrap().to_string(), "100.00");
        assert_eq!(per_unit(&d("30"), &d("-3")).unwrap().to_string(), "10");
        assert_eq!(per_unit(&d("17"), &d("7")).unwrap().to_string(), "2.428571428571428571428571429");
        assert_eq!(per_unit(&d("100"), &d("0")), None);
    }

    #[test]
    fn multiplication_in_context_rounds_to_28_digits() {
        let exact = mul(&d("1600"), &d("0.9090909090909090909090909091"));
        assert_eq!(exact.to_string(), "1454.5454545454545454545454545600");
        assert_eq!(
            mul_in_context(&d("1600"), &d("0.9090909090909090909090909091")).to_string(),
            "1454.545454545454545454545455"
        );
        assert_eq!(
            mul_in_context(&d("9.999999999999999999999999999"), &d("1.0000000000000000000000000001")).to_string(),
            "10.00000000000000000000000000"
        );
        assert_eq!(mul_in_context(&d("-1000.00"), &d("1")).to_string(), "-1000.00");
    }

    #[test]
    fn multiplication_keeps_the_sum_of_scales() {
        assert_eq!(mul(&d("-1000.00"), &d("1")).to_string(), "-1000.00");
        assert_eq!(mul(&d("1"), &d("4.50")).to_string(), "4.50");
        assert_eq!(mul(&d("3.513"), &d("136.65")).to_string(), "480.05145");
        assert_eq!(mul(&d("0.00"), &d("2.5")).to_string(), "0.000");
        assert_eq!(mul(&d("2"), &d("1.5")).to_string(), "3.0");
    }
}
