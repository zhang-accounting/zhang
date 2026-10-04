//! Decimal helpers shared by the evaluator and the function library.
//!
//! # Rounding policy
//!
//! - Ledger arithmetic is exact: sums, differences and products of ledger numbers
//!   (`units × cost`, weights, `*` in queries) never round. beancount computes these in
//!   Python's decimal context (28 significant digits), which is exact for every product of
//!   ledger numbers that fits in 28 digits, so the results agree.
//! - Division has to round: [`div`] follows Python's default context (28 significant
//!   digits, round-half-even), so `1/3` is `0.3333333333333333333333333333`.
//! - Valuation at market rates uses [`mul_in_context`], which rounds like beancount's decimal
//!   context, because inverted rates carry 28 significant digits and beanquery rounds those
//!   products.
//!
//! `BigDecimal`'s own `*` normalises when an operand is one (`-1000.00 × 1` becomes
//! `-1000`), so all products go through [`mul`] or [`mul_in_context`], which keep the scale
//! `lhs.scale + rhs.scale`.

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

/// Render a decimal without exponent notation, preserving its scale (`-12.50` stays `-12.50`), as
/// zhang-core's [`zhang_core::utils::plain_decimal`] writes it.
pub fn to_plain_string(value: &BigDecimal) -> String {
    zhang_core::utils::plain_decimal(value)
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
        assert_eq!(to_plain_string(&div(&d("1"), &d("3")).unwrap()), "0.3333333333333333333333333333");
        assert_eq!(to_plain_string(&div(&d("3"), &d("2")).unwrap()), "1.5");
        assert_eq!(to_plain_string(&div(&d("10.00"), &d("2")).unwrap()), "5.00");
        assert_eq!(to_plain_string(&div(&d("2"), &d("3")).unwrap()), "0.6666666666666666666666666667");
        assert_eq!(div(&d("1"), &d("0")), None);
    }

    #[test]
    fn multiplication_in_context_rounds_to_28_digits() {
        let exact = mul(&d("1600"), &d("0.9090909090909090909090909091"));
        assert_eq!(to_plain_string(&exact), "1454.5454545454545454545454545600");
        assert_eq!(
            to_plain_string(&mul_in_context(&d("1600"), &d("0.9090909090909090909090909091"))),
            "1454.545454545454545454545455"
        );
        assert_eq!(
            to_plain_string(&mul_in_context(&d("9.999999999999999999999999999"), &d("1.0000000000000000000000000001"))),
            "10.00000000000000000000000000"
        );
        assert_eq!(to_plain_string(&mul_in_context(&d("-1000.00"), &d("1"))), "-1000.00");
    }

    #[test]
    fn multiplication_keeps_the_sum_of_scales() {
        assert_eq!(to_plain_string(&mul(&d("-1000.00"), &d("1"))), "-1000.00");
        assert_eq!(to_plain_string(&mul(&d("1"), &d("4.50"))), "4.50");
        assert_eq!(to_plain_string(&mul(&d("3.513"), &d("136.65"))), "480.05145");
        assert_eq!(to_plain_string(&mul(&d("0.00"), &d("2.5"))), "0.000");
        assert_eq!(to_plain_string(&mul(&d("2"), &d("1.5"))), "3.0");
    }

    #[test]
    fn plain_string_never_uses_exponents() {
        assert_eq!(to_plain_string(&d("-12.50")), "-12.50");
        assert_eq!(to_plain_string(&d("0.0000001")), "0.0000001");
        assert_eq!(to_plain_string(&d("1E+3")), "1000");
        assert_eq!(to_plain_string(&d("0")), "0");
        assert_eq!(to_plain_string(&d("-0.05")), "-0.05");
    }
}
