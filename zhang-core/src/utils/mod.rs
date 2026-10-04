use std::path::PathBuf;

pub mod calculable;
pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod string_;

pub fn has_path_visited<'a>(visited: impl IntoIterator<Item = &'a PathBuf>, path: &PathBuf) -> bool {
    visited.into_iter().any(|pathbuf| pathbuf.eq(path))
}

macro_rules! feature_enable {
    ($feature_name: expr, $feature_process:expr) => {
        if $feature_name {
            $feature_process
        }
    };
    ($feature_name: expr, $feature_process:expr, $not_feature_process: expr) => {
        if $feature_name {
            $feature_process
        } else {
            $not_feature_process
        }
    };
}

/// A decimal written out in plain notation, which both zhang and beancount read: never with an exponent, as
/// `BigDecimal`'s `Display` writes a very small or very large number (`1E-9`), and keeping its scale (`-12.50`)
pub fn plain_decimal(value: &bigdecimal::BigDecimal) -> String {
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

#[cfg(test)]
mod plain_decimal_test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;

    use super::plain_decimal;

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
            assert_eq!(plain_decimal(&BigDecimal::from_str(number).unwrap()), plain, "{number}");
        }
    }
}
