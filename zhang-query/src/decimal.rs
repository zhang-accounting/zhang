//! Exact decimal helpers shared by the evaluator and the function library.
//!
//! Addition, subtraction and multiplication of [`BigDecimal`] are exact. Division is the
//! only operation that has to round: it follows the default context of Python's
//! `decimal` module (28 significant digits, round-half-even), which is what beancount
//! and beanquery use, so `1/3` gives `0.3333333333333333333333333333`.

use std::num::NonZeroU64;

use bigdecimal::{BigDecimal, RoundingMode, Zero};

/// Number of significant digits kept by [`div`].
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

/// Render a decimal without exponent notation, preserving its scale (`-12.50` stays `-12.50`).
pub fn to_plain_string(value: &BigDecimal) -> String {
    let (int_val, scale) = value.as_bigint_and_exponent();
    let negative = int_val.sign() == bigdecimal::num_bigint::Sign::Minus;
    let digits = int_val.magnitude().to_string();
    let mut out = String::with_capacity(digits.len() + 3);
    if negative {
        out.push('-');
    }
    if scale <= 0 {
        out.push_str(&digits);
        if digits != "0" {
            out.extend(std::iter::repeat_n('0', (-scale) as usize));
        }
    } else {
        let scale = scale as usize;
        if digits.len() > scale {
            let (int_part, frac_part) = digits.split_at(digits.len() - scale);
            out.push_str(int_part);
            out.push('.');
            out.push_str(frac_part);
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', scale - digits.len()));
            out.push_str(&digits);
        }
    }
    out
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
    fn plain_string_never_uses_exponents() {
        assert_eq!(to_plain_string(&d("-12.50")), "-12.50");
        assert_eq!(to_plain_string(&d("0.0000001")), "0.0000001");
        assert_eq!(to_plain_string(&d("1E+3")), "1000");
        assert_eq!(to_plain_string(&d("0")), "0");
        assert_eq!(to_plain_string(&d("-0.05")), "-0.05");
    }
}
