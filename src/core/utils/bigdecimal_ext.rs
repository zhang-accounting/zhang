use bigdecimal::{BigDecimal, Signed, ToPrimitive};
use bigdecimal::num_bigint::BigInt;

pub trait BigDecimalExt {
    fn round_with(self, round_digits: i64, is_up: bool) -> BigDecimal;
}

impl BigDecimalExt for &BigDecimal {
    fn round_with(self, round_digits: i64, is_up: bool) -> BigDecimal {
        let (bigint, decimal_part_digits) = self.as_bigint_and_exponent();
        let need_to_round_digits = decimal_part_digits - round_digits;
        if round_digits >= 0 && need_to_round_digits <= 0 {
            return self.clone();
        }

        let mut number = bigint.to_i128().unwrap();
        if number < 0 {
            number = -number;
        }
        for _ in 0..(need_to_round_digits - 1) {
            number /= 10;
        }
        let digit = number % 10;

        if digit <= 4 {
            self.with_scale(round_digits)
        } else if bigint.is_negative() {
            if is_up {
                self.with_scale(round_digits) - BigDecimal::new(BigInt::from(1i32), round_digits)
            } else {
                self.with_scale(round_digits)
            }
        } else if is_up {
            self.with_scale(round_digits) + BigDecimal::new(BigInt::from(1i32), round_digits)
        } else {
            self.with_scale(round_digits)
        }
    }
}
