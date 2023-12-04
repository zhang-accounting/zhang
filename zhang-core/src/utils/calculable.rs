use std::collections::HashMap;
use std::ops::{AddAssign, Mul};

use bigdecimal::{BigDecimal, Zero};
use chrono::DateTime;
use chrono_tz::Tz;
use zhang_ast::amount::{Amount, CalculatedAmount};

use crate::constants::KEY_OPERATING_CURRENCY;
use crate::domains::Operations;
use crate::ZhangResult;

pub trait Calculable {
    fn calculate(&self, date: DateTime<Tz>, operations: &mut Operations) -> ZhangResult<CalculatedAmount>;
}

impl Calculable for Vec<Amount> {
    fn calculate(&self, date: DateTime<Tz>, operations: &mut Operations) -> ZhangResult<CalculatedAmount> {
        let operating_currency = operations.option(KEY_OPERATING_CURRENCY)?.expect("cannot find operating currency").value;

        let mut total = BigDecimal::zero();
        let mut detail = HashMap::new();

        for amount in self.iter() {
            let number = amount.number.clone();
            let currency = amount.currency.clone();

            if currency.eq(&operating_currency) {
                total.add_assign(&number);
            } else if let Some(price) = operations.get_price(date.naive_local(), &currency, &operating_currency)? {
                total.add_assign((&number).mul(price.amount));
            }

            let currency_amount = detail.entry(currency).or_insert_with(BigDecimal::zero);
            currency_amount.add_assign(&number);
        }

        Ok(CalculatedAmount {
            calculated: Amount::new(total, operating_currency),
            detail,
        })
    }
}
