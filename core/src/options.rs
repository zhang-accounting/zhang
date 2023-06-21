use crate::constants::{
    DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN, DEFAULT_COMMODITY_PRECISION_PLAIN, DEFAULT_OPERATING_CURRENCY, DEFAULT_ROUNDING_PLAIN,
    KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING, KEY_OPERATING_CURRENCY,
};
use itertools::Itertools;
use sqlx::SqliteConnection;
use std::str::FromStr;
use std::string::ToString;
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};
use strum::{AsRefStr, EnumString, EnumIter, IntoEnumIterator};

use crate::ZhangResult;

#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: i32,
}

pub static DEFAULT_OPTIONS: [(&str, &str); 4] = [
    (KEY_OPERATING_CURRENCY, DEFAULT_OPERATING_CURRENCY),
    (KEY_DEFAULT_ROUNDING, DEFAULT_ROUNDING_PLAIN),
    (KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION, DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN),
    (KEY_DEFAULT_COMMODITY_PRECISION, DEFAULT_COMMODITY_PRECISION_PLAIN),
];

#[derive(Debug, AsRefStr, EnumIter, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum BuiltinOption {
    OperatingCurrency,
    DefaultRounding,
    DefaultBalanceTolerancePrecision,
    DefaultCommodityPrecision,
}

impl BuiltinOption {
    pub fn default_value(&self) -> &str {
        match self {
            BuiltinOption::OperatingCurrency => DEFAULT_OPERATING_CURRENCY,
            BuiltinOption::DefaultRounding => DEFAULT_ROUNDING_PLAIN,
            BuiltinOption::DefaultBalanceTolerancePrecision => DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN,
            BuiltinOption::DefaultCommodityPrecision => DEFAULT_COMMODITY_PRECISION_PLAIN,
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

pub fn default_options() -> Vec<Spanned<Directive>> {
    DEFAULT_OPTIONS
        .iter()
        .cloned()
        .map(|(key, value)| {
            Spanned::new(
                Directive::Option(Options {
                    key: ZhangString::quote(key),
                    value: ZhangString::quote(value),
                }),
                SpanInfo::default(),
            )
        })
        .collect_vec()
}

impl InMemoryOptions {
    pub async fn parse(&mut self, key: impl Into<String>, value: impl Into<String>, conn: &mut SqliteConnection) -> ZhangResult<()> {
        let value = value.into();
        let key = key.into();
        match key.as_str() {
            "operating_currency" => {
                let precision = self.default_balance_tolerance_precision;
                let prefix: Option<String> = None;
                let suffix: Option<String> = None;
                let rounding = Some(self.default_rounding);

                sqlx::query(
                    r#"INSERT OR REPLACE INTO commodities (name, precision, prefix, suffix, rounding)
                        VALUES ($1, $2, $3, $4, $5);"#,
                )
                .bind(&value)
                .bind(precision)
                .bind(prefix)
                .bind(suffix)
                .bind(rounding.map(|it| it.to_string()))
                .execute(conn)
                .await?;
                self.operating_currency = value;
            }
            "default_rounding" => {
                self.default_rounding = Rounding::from_str(&value).unwrap();
            }
            "default_balance_tolerance" => {
                if let Ok(ret) = value.parse::<i32>() {
                    self.default_balance_tolerance_precision = ret
                }
            }
            _ => {}
        };
        Ok(())
    }
}

impl Default for InMemoryOptions {
    fn default() -> Self {
        InMemoryOptions {
            operating_currency: "CNY".to_string(),
            default_rounding: Rounding::RoundDown,
            default_balance_tolerance_precision: 2,
        }
    }
}
