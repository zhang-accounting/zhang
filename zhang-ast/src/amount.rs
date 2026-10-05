use std::collections::HashMap;
use std::ops::{Add, Div, Mul, Neg, Sub};

use bigdecimal::{BigDecimal, Zero};
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::{Deserialize, Serialize};
use zhang_shared::decimal::plain_decimal;

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct CalculatedAmount {
    pub calculated: Amount,
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize_map")]
    pub detail: HashMap<String, BigDecimal>,
}

impl CalculatedAmount {
    pub fn new(commodity: &str) -> CalculatedAmount {
        let mut detail = HashMap::new();
        detail.insert(commodity.to_owned(), BigDecimal::zero());
        CalculatedAmount {
            calculated: Amount::new(BigDecimal::zero(), commodity.to_owned()),
            detail,
        }
    }
    pub fn persist_commodity(mut self, commodity: &str) -> Self {
        self.detail.entry(commodity.to_owned()).or_default();
        self
    }
}

#[derive(Eq, PartialEq, Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub struct Amount {
    /// serialized as a string in plain notation (`"0.0000001"`, never `"1E-7"`), with its scale
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize")]
    pub number: BigDecimal,
    pub commodity: String,
}

impl Amount {
    pub fn new(number: BigDecimal, commodity: impl Into<String>) -> Amount {
        Amount {
            number,
            commodity: commodity.into(),
        }
    }

    pub fn zero(commodity: impl Into<String>) -> Amount {
        Amount {
            number: BigDecimal::zero(),
            commodity: commodity.into(),
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
            commodity: self.commodity.clone(),
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
        Amount::new((&(self.number)).neg(), self.commodity.clone())
    }
}

///
/// ```rust
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// assert_eq!(Amount::new(BigDecimal::from(-100i32), "CNY").to_string(), "-100 CNY");
/// assert_eq!(Amount::new(BigDecimal::from(100i32), "CNY").to_string(), "100 CNY");
/// ```
///
/// The number is written in plain notation, with its scale ([`plain_decimal`]):
///
/// ```rust
/// use std::str::FromStr;
/// use bigdecimal::BigDecimal;
/// use zhang_ast::amount::Amount;
/// assert_eq!(Amount::new(BigDecimal::from_str("0.0000001").unwrap(), "BTC").to_string(), "0.0000001 BTC");
/// assert_eq!(Amount::new(BigDecimal::from_str("1.2E+30").unwrap(), "CNY").to_string(), "1200000000000000000000000000000 CNY");
/// assert_eq!(Amount::new(BigDecimal::from_str("0.00").unwrap(), "CNY").to_string(), "0.00 CNY");
/// ```
impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", plain_decimal(&self.number), self.commodity)
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
            commodity: self.commodity.clone(),
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
            commodity: self.commodity.clone(),
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
            commodity: self.commodity.clone(),
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
            commodity: self.commodity.clone(),
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use serde_json::json;

    use super::{Amount, CalculatedAmount};

    fn amount(number: &str, commodity: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), commodity)
    }

    /// A tiny or huge number is serialized in plain notation, as the queries and the exporter write it, not with
    /// the exponent of `BigDecimal`'s own `Serialize` (`"1E-7"`), and keeps its scale; either form reads back.
    #[test]
    fn an_amount_serializes_its_number_in_plain_notation() {
        for (number, plain) in [
            ("0.0000001", "0.0000001"),
            ("-0.00000012500", "-0.00000012500"),
            ("1.200000000000000000000000000E+30", "1200000000000000000000000000000"),
            ("1E+3", "1000"),
            ("0.00", "0.00"),
        ] {
            let value = serde_json::to_value(amount(number, "BTC")).unwrap();
            assert_eq!(value, json!({"number": plain, "commodity": "BTC"}), "{number}");
            let read: Amount = serde_json::from_value(value).unwrap();
            assert_eq!(read, amount(number, "BTC"), "{number}");
        }
        let read: Amount = serde_json::from_value(json!({"number": "1E-7", "commodity": "BTC"})).unwrap();
        assert_eq!(read, amount("0.0000001", "BTC"));

        let calculated = CalculatedAmount {
            calculated: amount("1.2E+30", "CNY"),
            detail: HashMap::from([("BTC".to_owned(), BigDecimal::from_str("0.0000001").unwrap())]),
        };
        assert_eq!(
            serde_json::to_value(calculated).unwrap(),
            json!({"calculated": {"number": "1200000000000000000000000000000", "commodity": "CNY"}, "detail": {"BTC": "0.0000001"}})
        );

        // the bare decimals of the directives too, as plugins read them
        let cost = crate::PostingCost {
            base: Some(amount("1", "USD")),
            compound_total: Some(BigDecimal::from_str("0.0000001").unwrap()),
            ..Default::default()
        };
        let value = serde_json::to_value(&cost).unwrap();
        assert_eq!((&value["base"]["number"], &value["compound_total"]), (&json!("1"), &json!("0.0000001")));
        assert_eq!(serde_json::from_value::<crate::PostingCost>(value).unwrap(), cost);
    }
}
