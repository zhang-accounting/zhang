use bigdecimal::{BigDecimal, Zero};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct Amount {
    pub number: BigDecimal,
    pub currency: String,
}

impl Amount {

    pub fn none() -> Amount {
        Amount {
            number: Default::default(),
            currency: "".to_string()
        }
    }

    pub fn new_with_i32(number: i32, currency: impl Into<String>) -> Amount {
        Amount {
            number: BigDecimal::from(number),
            currency: currency.into(),
        }
    }
    pub fn new(number: BigDecimal, currency: impl Into<String>) -> Amount {
        Amount {
            number,
            currency: currency.into(),
        }
    }
    pub fn is_zero(&self) -> bool {
        self.number.is_zero()
    }

    pub fn abs(&self) -> Amount {
        Amount {
            number: self.number.abs(),
            currency: self.currency.clone(),
        }
    }
}

impl ToString for Amount {
    fn to_string(&self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl Add<BigDecimal> for &Amount {
    type Output = Amount;

    fn add(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).add(rhs),
            currency: self.currency.clone(),
        }
    }
}

impl Sub<BigDecimal> for &Amount {
    type Output = Amount;

    fn sub(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).sub(rhs),
            currency: self.currency.clone(),
        }
    }
}

impl Mul<BigDecimal> for &Amount {
    type Output = Amount;

    fn mul(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).mul(rhs),
            currency: self.currency.clone(),
        }
    }
}

impl Div<BigDecimal> for &Amount {
    type Output = Amount;

    fn div(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).div(rhs),
            currency: self.currency.clone(),
        }
    }
}

impl Neg for Amount {
    type Output = Amount;

    fn neg(mut self) -> Self::Output {
        self.number = self.number.neg();
        self
    }
}
