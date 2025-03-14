use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cfg_if::cfg_if;
use chrono_tz::Tz;
use itertools::Itertools;
use log::error;
use once_cell::sync::OnceCell;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use zhang_ast::error::ErrorKind;
use zhang_ast::utils::inventory::BookingMethod;
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::constants::*;
use crate::domains::Operations;
use crate::features::Features;
use crate::{ZhangError, ZhangResult};
use minijinja::Environment;


#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: i32,
    pub default_booking_method: BookingMethod,
    pub timezone: Tz,
    pub features: Features,
    pub directive_output_path: String,
}

#[derive(Debug, AsRefStr, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum BuiltinOption {
    OperatingCurrency,
    DefaultRounding,
    DefaultBalanceTolerancePrecision,
    DefaultCommodityPrecision,
    DefaultBookingMethod,
    Timezone,
    DirectiveOutputPath,
}

fn detect_timezone() -> String {
    cfg_if! {
        if #[cfg(feature = "iana-time-zone")] {
            static DETECTED_TZ: OnceCell<String> = OnceCell::new();
            DETECTED_TZ
                .get_or_init(|| match iana_time_zone::get_timezone() {
                    Ok(timezone) => {
                        log::info!("detect system timezone is {}", timezone);
                        timezone
                    }
                    Err(e) => {
                        log::warn!("cannot get timezone, fall back to use GMT+8 as default timezone: {}", e);
                        DEFAULT_TIMEZONE.to_owned()
                    }
                })
                .to_string()
        }else {
             crate::constants::DEFAULT_TIMEZONE.to_owned()
        }
    }
}

impl BuiltinOption {
    pub fn default_value(&self) -> String {
        match self {
            BuiltinOption::OperatingCurrency => DEFAULT_OPERATING_CURRENCY.to_owned(),
            BuiltinOption::DefaultRounding => DEFAULT_ROUNDING_PLAIN.to_owned(),
            BuiltinOption::DefaultBalanceTolerancePrecision => DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN.to_owned(),
            BuiltinOption::DefaultCommodityPrecision => DEFAULT_COMMODITY_PRECISION_PLAIN.to_owned(),
            BuiltinOption::DefaultBookingMethod => DEFAULT_BOOKING_METHOD.to_owned(),
            BuiltinOption::Timezone => detect_timezone(),
            BuiltinOption::DirectiveOutputPath => DEFAULT_DIRECTIVE_OUTPUT_PATH.to_owned(),
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

                    let has_operating_currency = operation.option::<String>(&key)?.is_some();
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
                BuiltinOption::DefaultBookingMethod => {
                    self.default_booking_method = BookingMethod::from_str(&value).map_err(|_| ZhangError::InvalidOptionValue)?
                },
                BuiltinOption::DirectiveOutputPath => {
                    let mut env = Environment::new();
                    let res = env.add_template("directive_output_path", &value);
                    if res.is_err() {
                        return Err(ZhangError::InvalidOptionValue);
                    }
                    self.directive_output_path = value.to_string();
                }
            }
        }
        self.features.handle_options(&key, &value);

        Ok(value)
    }
}

impl Default for InMemoryOptions {
    fn default() -> Self {
        InMemoryOptions {
            operating_currency: DEFAULT_OPERATING_CURRENCY.to_string(),
            default_rounding: DEFAULT_ROUNDING,
            default_balance_tolerance_precision: DEFAULT_BALANCE_TOLERANCE_PRECISION,
            default_booking_method: DEFAULT_BOOKING_METHOD.parse().expect("invalid booking method"),
            timezone: DEFAULT_TIMEZONE.parse().expect("invalid timezone"),
            features: Features::default(),
            directive_output_path: DEFAULT_DIRECTIVE_OUTPUT_PATH.to_string(),
        }
    }
}
