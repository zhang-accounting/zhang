use crate::constants::{
    DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN, DEFAULT_COMMODITY_PRECISION_PLAIN, DEFAULT_OPERATING_CURRENCY, DEFAULT_ROUNDING_PLAIN,
    KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING, KEY_OPERATING_CURRENCY,
};
use itertools::Itertools;
use sqlx::SqliteConnection;
use std::str::FromStr;
use std::string::ToString;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::ZhangResult;

#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: i32,
}

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

impl InMemoryOptions {
    pub async fn parse(&mut self, key: impl Into<String>, value: impl Into<String>, conn: &mut SqliteConnection) -> ZhangResult<()> {
        let value = value.into();
        let key = key.into();
        if let Ok(option) = BuiltinOption::from_str(&key) {
            match option {
                BuiltinOption::OperatingCurrency => {
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
                BuiltinOption::DefaultRounding => {
                    self.default_rounding = Rounding::from_str(&value).unwrap();
                }
                BuiltinOption::DefaultBalanceTolerancePrecision => {
                    if let Ok(ret) = value.parse::<i32>() {
                        self.default_balance_tolerance_precision = ret
                    }
                }
                BuiltinOption::DefaultCommodityPrecision => {}
            }
        }

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
