use std::collections::HashMap;
use std::ops::{Add, Div, Mul, Neg, Sub};

use bigdecimal::{BigDecimal, Zero};
use serde::Serialize;

#[derive(Serialize)]
pub struct CalculatedAmount {
    pub calculated: Amount,
    pub detail: HashMap<String, BigDecimal>,
}

impl CalculatedAmount {
    pub fn new(currency: &str) -> CalculatedAmount {
        let mut detail = HashMap::new();
        detail.insert(currency.to_owned(), BigDecimal::zero());
        CalculatedAmount {
            calculated: Amount::new(BigDecimal::zero(), currency.to_owned()),
            detail,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone, Serialize)]
pub struct Amount {
    pub number: BigDecimal,
    pub currency: String,
}

impl Amount {
    pub fn new(number: BigDecimal, currency: impl Into<String>) -> Amount {
        Amount {
            number,
            currency: currency.into(),
        }
    }

    ///
    /// ```rust
    /// use bigdecimal::BigDecimal;
    /// use zhang_ast::amount::Amount;
    /// assert!(Amount::new(BigDecimal::from(0i32), "CNY").is_zero());
    /// assert!(Amount::new(BigDecimal::from(-0i32), "CNY").is_zero());
    /// assert!(!Amount::new(BigDecimal::from(100i32), "CNY").is_zero());
    /// assert!(!Amount::new(BigDecimal::from(-100i32), "CNY").is_zero());
    /// ```
    pub fn is_zero(&self) -> bool {
        self.number.is_zero()
    }

    /// ```rust
    /// use bigdecimal::BigDecimal;
    /// use zhang_ast::amount::Amount;
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(0i32), "CNY").abs(),
    ///     Amount::new(BigDecimal::from(0i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(-0i32), "CNY").abs(),
    ///     Amount::new(BigDecimal::from(0i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(100i32), "CNY").abs(),
    ///     Amount::new(BigDecimal::from(100i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(-100i32), "CNY").abs(),
    ///     Amount::new(BigDecimal::from(100i32), "CNY")
    /// );
    /// ```
    pub fn abs(&self) -> Amount {
        Amount {
            number: self.number.abs(),
            currency: self.currency.clone(),
        }
    }
    /// ```rust
    /// use bigdecimal::BigDecimal;
    /// use zhang_ast::amount::Amount;
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(0i32), "CNY").neg(),
    ///     Amount::new(BigDecimal::from(0i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(-0i32), "CNY").neg(),
    ///     Amount::new(BigDecimal::from(0i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(100i32), "CNY").neg(),
    ///     Amount::new(BigDecimal::from(-100i32), "CNY")
    /// );
    /// assert_eq!(
    ///     Amount::new(BigDecimal::from(-100i32), "CNY").neg(),
    ///     Amount::new(BigDecimal::from(100i32), "CNY")
    /// );
    /// ```
    pub fn neg(&self) -> Amount {
        Amount::new((&(self.number)).neg(), self.currency.clone())
    }
}

///
/// ```rust
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// assert_eq!(Amount::new(BigDecimal::from(-100i32), "CNY").to_string(), "-100 CNY");
/// assert_eq!(Amount::new(BigDecimal::from(100i32), "CNY").to_string(), "100 CNY");
/// ```
impl ToString for Amount {
    fn to_string(&self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

///
/// ```rust
/// use std::ops::Add;
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// let a = Amount::new(BigDecimal::from(1i32), "CNY");
/// let b = BigDecimal::from(2i32);
/// let ret = Amount::new(BigDecimal::from(3i32), "CNY");
/// assert_eq!((&a).add(b), ret);
/// ```
impl Add<BigDecimal> for &Amount {
    type Output = Amount;

    fn add(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).add(rhs),
            currency: self.currency.clone(),
        }
    }
}

///
/// ```rust
/// use std::ops::Sub;
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// let a = BigDecimal::from(1i32);
/// let b = Amount::new(BigDecimal::from(2i32), "CNY");
/// let ret = Amount::new(BigDecimal::from(1i32), "CNY");
/// assert_eq!((&b).sub(a), ret);
/// ```
impl Sub<BigDecimal> for &Amount {
    type Output = Amount;

    fn sub(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).sub(rhs),
            currency: self.currency.clone(),
        }
    }
}

///
/// ```rust
/// use std::ops::Mul;
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// let a = Amount::new(BigDecimal::from(3i32), "CNY");
/// let b = BigDecimal::from(2i32);
/// let ret = Amount::new(BigDecimal::from(6i32), "CNY");
/// assert_eq!((&a).mul(b), ret);
/// ```
impl Mul<BigDecimal> for &Amount {
    type Output = Amount;

    fn mul(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).mul(rhs),
            currency: self.currency.clone(),
        }
    }
}

///
/// ```rust
/// use std::ops::Div;
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// let a = Amount::new(BigDecimal::from(4i32), "CNY");
/// let b = BigDecimal::from(2i32);
/// let ret = Amount::new(BigDecimal::from(2i32), "CNY");
/// assert_eq!((&a).div(b), ret);
/// ```
impl Div<BigDecimal> for &Amount {
    type Output = Amount;

    fn div(self, rhs: BigDecimal) -> Self::Output {
        Amount {
            number: (&self.number).div(rhs),
            currency: self.currency.clone(),
        }
    }
}
///
/// ```rust
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// let a = Amount::new(BigDecimal::from(4i32), "CNY");
/// let ret = Amount::new(BigDecimal::from(-4i32), "CNY");
/// assert_eq!(a.neg(), ret);
/// ```
impl Neg for Amount {
    type Output = Amount;

    fn neg(mut self) -> Self::Output {
        self.number = self.number.neg();
        self
    }
}
