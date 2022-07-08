use crate::core::models::Rounding;
use std::str::FromStr;

#[derive(Debug)]
pub struct Options {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: usize,
}

impl Options {
    pub fn parse(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let value = value.into();
        match key.into().as_str() {
            "operating_currency" => {
                self.operating_currency = value;
            }
            "default_rounding" => {
                self.default_rounding = Rounding::from_str(&value).unwrap();
            }
            "default_balance_tolerance" => {
                if let Ok(ret) = value.parse::<usize>() {
                    self.default_balance_tolerance_precision = ret
                }
            }
            _ => {}
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            operating_currency: "CNY".to_string(),
            default_rounding: Rounding::RoundDown,
            default_balance_tolerance_precision: 2,
        }
    }
}
