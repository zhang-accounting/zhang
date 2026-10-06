use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cfg_if::cfg_if;
use chrono_tz::Tz;
use indexmap::IndexMap;
use itertools::Itertools;
use log::{error, warn};
use minijinja::Environment;
use once_cell::sync::OnceCell;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::constants::*;
use crate::domains::schemas::{CommodityDomain, ErrorDomain, OptionDomain};
use crate::features::Features;
use crate::inventory::BookingMethod;
use crate::utils::hashmap::HashMapOfExt;
use crate::{ZhangError, ZhangResult};

#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    /// the precision of a commodity whose directive has no valid `precision` meta, the operating currency included
    pub default_commodity_precision: i32,
    /// deprecated: stands in for [`default_commodity_precision`](Self::default_commodity_precision) for the operating
    /// currency when the ledger does not write that option. It never gives a balance assertion a tolerance
    pub default_balance_tolerance_precision: i32,
    pub default_booking_method: BookingMethod,
    pub timezone: Tz,
    pub features: Features,
    pub directive_output_path: String,
    /// whether the ledger writes `default_commodity_precision`, as opposed to the default
    /// [`BuiltinOption::default_options`] adds for it; only a written one overrides the deprecated option
    commodity_precision_written: bool,
    /// whether an `operating_currency` option was read yet: the options read before it (the defaults come first)
    /// must not define the built-in `CNY` when the ledger names another currency
    operating_currency_read: bool,
    /// the value of every option, by key, as [`InMemoryOptions::parse`] resolved it: the built-in ones, the ones it
    /// does not know, and the beancount ones. A key the ledger writes again keeps the last value
    pub values: HashMap<String, String>,
    /// the commodities the options define: the operating currency, as the options read last defined it, in the order
    /// first defined. A ledger that names several operating currencies has them all
    commodities: IndexMap<String, CommodityDomain>,
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
                    // no text and no file: `is_default_directive` tells these from the options the ledger writes
                    SpanInfo::default(),
                )
            })
            .collect_vec()
    }

    /// whether `span` is that of an option [`BuiltinOption::default_options`] added for a key the ledger does not
    /// write, rather than of one written in the ledger: a written option carries its source text
    pub fn is_default_directive(span: &SpanInfo) -> bool {
        span.content.is_empty() && span.filename.is_none()
    }
}

impl InMemoryOptions {
    /// read the option `key`, and keep its value as resolved ([`InMemoryOptions::option`]); what is wrong with it is
    /// reported in `errors`
    pub fn parse(&mut self, key: impl Into<String>, value: impl Into<String>, errors: &mut Vec<ErrorDomain>, span: &SpanInfo) -> ZhangResult<()> {
        let key = key.into();
        let value = self.resolve(&key, value.into(), errors, span)?;
        self.values.insert(key, value);
        Ok(())
    }

    /// the value of the option `key` as resolved: the value written, or what replaces an invalid one
    fn resolve(&mut self, key: &str, value: String, errors: &mut Vec<ErrorDomain>, span: &SpanInfo) -> ZhangResult<String> {
        if let Ok(option) = BuiltinOption::from_str(key) {
            match option {
                BuiltinOption::OperatingCurrency => {
                    let has_operating_currency = self.values.contains_key(key);
                    if has_operating_currency {
                        errors.push(ErrorDomain::new(ErrorKind::MultipleOperatingCurrencyDetect, span, HashMap::default()));
                    }
                    value.clone_into(&mut self.operating_currency);
                    self.operating_currency_read = true;
                    self.define_operating_currency();
                }
                BuiltinOption::DefaultRounding => {
                    self.default_rounding = Rounding::from_str(&value).map_err(|_| ZhangError::InvalidOptionValue)?;
                    self.define_operating_currency();
                }
                BuiltinOption::DefaultBalanceTolerancePrecision => {
                    if !BuiltinOption::is_default_directive(span) {
                        warn!(
                            "option \"{key}\" is deprecated: it gives balance assertions no tolerance, and sets the precision of the \
                             operating currency only while \"default_commodity_precision\" is not set; set that option, or a \
                             `commodity` directive, instead"
                        );
                    }
                    if let Ok(ret) = value.parse::<i32>() {
                        self.default_balance_tolerance_precision = ret
                    }
                    self.define_operating_currency();
                }
                BuiltinOption::DefaultCommodityPrecision => {
                    self.default_commodity_precision = value.parse::<i32>().map_err(|_| ZhangError::InvalidOptionValue)?;
                    self.commodity_precision_written |= !BuiltinOption::is_default_directive(span);
                    self.define_operating_currency();
                }
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
                    // an invalid or unsupported method is reported and leaves the default method as
                    // it was (FIFO unless set before), like the `booking_method` account meta
                    let (method, error) = BookingMethod::resolve(&value, self.default_booking_method);
                    self.default_booking_method = method;
                    if let Some(kind) = error {
                        errors.push(ErrorDomain::new(kind, span, HashMap::of("booking_method", value.clone())));
                        return Ok(method.to_string());
                    }
                }
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
        self.features.handle_options(key, &value);

        Ok(value)
    }

    /// the value of the option `key`, read as a `T`; `None` when the ledger has no such option
    pub fn option<T>(&self, key: impl AsRef<str>) -> ZhangResult<Option<T>>
    where
        T: FromStr,
    {
        let value = self.values.get(key.as_ref());
        value.map(|value| T::from_str(value).map_err(|_| ZhangError::InvalidOptionValue)).transpose()
    }

    /// every option with its value as resolved, in no particular order
    pub fn all(&self) -> Vec<OptionDomain> {
        let values = self.values.iter().map(|(key, value)| (key.clone(), value.clone()));
        values.map(|(key, value)| OptionDomain { key, value }).collect_vec()
    }

    /// the commodities the options define (the operating currency), in the order first defined
    pub fn commodities(&self) -> impl Iterator<Item = &CommodityDomain> {
        self.commodities.values()
    }

    /// the precision of the commodity `operating_currency` defines: `default_commodity_precision` when the ledger
    /// writes it, else the deprecated `default_balance_tolerance_precision` (both default to the same built-in value)
    fn operating_currency_precision(&self) -> i32 {
        if self.commodity_precision_written {
            self.default_commodity_precision
        } else {
            self.default_balance_tolerance_precision
        }
    }

    /// (re)define the commodity of the operating currency from the options read so far. Every option it depends on
    /// calls this, so the definition the last option leaves is the same whatever their order; a dated `commodity`
    /// directive for it, processed after every option, replaces it
    fn define_operating_currency(&mut self) {
        if let Some(commodity) = self.operating_currency_commodity() {
            self.commodities.insert(commodity.name.clone(), commodity);
        }
    }

    /// the commodity the options define, the operating currency, as the options read so far define it before any
    /// `commodity` directive; none before an `operating_currency` option was read
    pub fn operating_currency_commodity(&self) -> Option<CommodityDomain> {
        self.operating_currency_read.then(|| CommodityDomain {
            name: self.operating_currency.clone(),
            precision: self.operating_currency_precision(),
            prefix: None,
            suffix: None,
            rounding: self.default_rounding,
        })
    }
}

impl Default for InMemoryOptions {
    fn default() -> Self {
        InMemoryOptions {
            operating_currency: DEFAULT_OPERATING_CURRENCY.to_string(),
            default_rounding: DEFAULT_ROUNDING,
            default_commodity_precision: DEFAULT_COMMODITY_PRECISION,
            default_balance_tolerance_precision: DEFAULT_BALANCE_TOLERANCE_PRECISION,
            default_booking_method: DEFAULT_BOOKING_METHOD.parse().expect("invalid booking method"),
            timezone: DEFAULT_TIMEZONE.parse().expect("invalid timezone"),
            features: Features::default(),
            directive_output_path: DEFAULT_DIRECTIVE_OUTPUT_PATH.to_string(),
            commodity_precision_written: false,
            operating_currency_read: false,
            values: HashMap::new(),
            commodities: IndexMap::new(),
        }
    }
}
