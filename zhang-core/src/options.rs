use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono_tz::Tz;
use itertools::Itertools;
use log::{error, info, warn};
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::constants::{
    DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN, DEFAULT_COMMODITY_PRECISION_PLAIN, DEFAULT_OPERATING_CURRENCY, DEFAULT_ROUNDING_PLAIN, DEFAULT_TIMEZONE,
};
use crate::domains::Operations;
use crate::{ZhangError, ZhangResult};

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
    pub fn default_options(options_key: HashSet<Cow<str>>) -> Vec<Spanned<Directive>> {
        BuiltinOption::iter()
            .filter(|it| !options_key.contains(it.key()))
            .map(|it| {
                Spanned::new(
                    Directive::Option(Options {
                        key: ZhangString::quote(it.as_ref()),
                        value: ZhangString::quote(it.default_value()),
                    }),
                    SpanInfo::default(),
                )
            })
            .collect_vec()
    }
}

impl InMemoryOptions {
    pub fn parse(&mut self, key: impl Into<String>, value: impl Into<String>, operation: &mut Operations, span: &SpanInfo) -> ZhangResult<String> {
        let value = value.into();
        let key = key.into();
        if let Ok(option) = BuiltinOption::from_str(&key) {
            match option {
                BuiltinOption::OperatingCurrency => {
                    let precision = self.default_balance_tolerance_precision;
                    let prefix: Option<String> = None;
                    let suffix: Option<String> = None;
                    let rounding = self.default_rounding;

                    let has_operating_currency = operation.option(key)?.is_some();
                    if has_operating_currency {
                        operation.new_error(ErrorKind::MultipleOperatingCurrencyDetect, span, HashMap::default())?;
                    }
                    operation.insert_commodity(&value, precision, prefix, suffix, rounding)?;

                    value.clone_into(&mut self.operating_currency);
                }
                BuiltinOption::DefaultRounding => {
                    self.default_rounding = Rounding::from_str(&value).map_err(|_| ZhangError::InvalidOptionValue)?;
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
            timezone: BuiltinOption::Timezone.default_value().parse().expect("invalid timezone"),
        }
    }
}
