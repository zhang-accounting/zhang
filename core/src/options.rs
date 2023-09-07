use itertools::Itertools;
use log::{error, info, warn};
use std::str::FromStr;
use std::string::ToString;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::constants::{
    DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN, DEFAULT_COMMODITY_PRECISION_PLAIN, DEFAULT_OPERATING_CURRENCY, DEFAULT_ROUNDING_PLAIN, DEFAULT_TIMEZONE,
};
use crate::domains::Operations;
use crate::ZhangResult;
use chrono_tz::Tz;

#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: i32,
    pub timezone: Tz,
}

#[derive(Debug, AsRefStr, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum BuiltinOption {
    OperatingCurrency,
    DefaultRounding,
    DefaultBalanceTolerancePrecision,
    DefaultCommodityPrecision,
    Timezone,
}

impl BuiltinOption {
    pub fn default_value(&self) -> String {
        match self {
            BuiltinOption::OperatingCurrency => DEFAULT_OPERATING_CURRENCY.to_owned(),
            BuiltinOption::DefaultRounding => DEFAULT_ROUNDING_PLAIN.to_owned(),
            BuiltinOption::DefaultBalanceTolerancePrecision => DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN.to_owned(),
            BuiltinOption::DefaultCommodityPrecision => DEFAULT_COMMODITY_PRECISION_PLAIN.to_owned(),
            BuiltinOption::Timezone => match iana_time_zone::get_timezone() {
                Ok(timezone) => {
                    info!("detect system timezone is {}", timezone);
                    timezone
                }
                Err(e) => {
                    warn!("cannot get timezone, fall back to use GMT+8 as default timezone: {}", e);
                    DEFAULT_TIMEZONE.to_owned()
                }
            },
        }
    }
    pub fn key(&self) -> &str {
        self.as_ref()
    }
    pub fn default_options() -> Vec<Spanned<Directive>> {
        BuiltinOption::iter()
            .map(|key| {
                Spanned::new(
                    Directive::Option(Options {
                        key: ZhangString::quote(key.as_ref()),
                        value: ZhangString::quote(key.default_value()),
                    }),
                    SpanInfo::default(),
                )
            })
            .collect_vec()
    }
}

impl InMemoryOptions {
    pub async fn parse(&mut self, key: impl Into<String>, value: impl Into<String>, operation: &mut Operations) -> ZhangResult<String> {
        let value = value.into();
        let key = key.into();
        if let Ok(option) = BuiltinOption::from_str(&key) {
            match option {
                BuiltinOption::OperatingCurrency => {
                    let precision = self.default_balance_tolerance_precision;
                    let prefix: Option<String> = None;
                    let suffix: Option<String> = None;
                    let rounding = Some(self.default_rounding);

                    operation
                        .insert_commodity(&value, precision, prefix, suffix, rounding.map(|it| it.to_string()))
                        .await?;

                    self.operating_currency = value.to_owned();
                }
                BuiltinOption::DefaultRounding => {
                    self.default_rounding = Rounding::from_str(&value).unwrap();
                }
                BuiltinOption::DefaultBalanceTolerancePrecision => {
                    if let Ok(ret) = value.parse::<i32>() {
                        self.default_balance_tolerance_precision = ret
                    }
                }
                BuiltinOption::DefaultCommodityPrecision => {}
                BuiltinOption::Timezone => match value.parse::<Tz>() {
                    Ok(tz) => {
                        self.timezone = tz;
                    }
                    Err(e) => {
                        error!("timezone value '{value}' is not a valid timezone, fallback to use system timezone: {e}");
                        return Ok(BuiltinOption::Timezone.default_value());
                    }
                },
            }
        }
        Ok(value)
    }
}

impl Default for InMemoryOptions {
    fn default() -> Self {
        InMemoryOptions {
            operating_currency: "CNY".to_string(),
            default_rounding: Rounding::RoundDown,
            default_balance_tolerance_precision: 2,
            timezone: BuiltinOption::Timezone.default_value().parse().unwrap(),
        }
    }
}
