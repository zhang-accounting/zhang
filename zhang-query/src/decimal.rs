//! Decimal arithmetic of the query engine. Valuation and plugin prices use the same
//! implementation; see [`zhang_shared::decimal`] for precision and scale rules.

use bigdecimal::BigDecimal;
pub use zhang_shared::decimal::{div, mul, mul_in_context, per_unit, DIVISION_PRECISION};

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
    fn plain_string_never_uses_exponents() {
        assert_eq!(to_plain_string(&d("-12.50")), "-12.50");
        assert_eq!(to_plain_string(&d("0.0000001")), "0.0000001");
        assert_eq!(to_plain_string(&d("1E+3")), "1000");
        assert_eq!(to_plain_string(&d("0")), "0");
        assert_eq!(to_plain_string(&d("-0.05")), "-0.05");
    }
}
