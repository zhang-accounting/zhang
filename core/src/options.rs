use crate::constants::{
    DEFAULT_BALANCE_TOLERANCE_PRECISION, DEFAULT_OPERATING_CURRENCY, DEFAULT_ROUNDING, KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION, KEY_DEFAULT_ROUNDING,
    KEY_OPERATING_CURRENCY,
};
use sqlx::SqliteConnection;
use std::str::FromStr;
use zhang_ast::{Directive, Options, Rounding, SpanInfo, Spanned, ZhangString};

use crate::ZhangResult;

#[derive(Debug)]
pub struct InMemoryOptions {
    pub operating_currency: String,
    pub default_rounding: Rounding,
    pub default_balance_tolerance_precision: i32,
}

pub fn default_options() -> [Spanned<Directive>; 3] {
    [
        Spanned::new(
            Directive::Option(Options {
                key: ZhangString::quote(KEY_OPERATING_CURRENCY),
                value: ZhangString::quote(DEFAULT_OPERATING_CURRENCY),
            }),
            SpanInfo::default(),
        ),
        Spanned::new(
            Directive::Option(Options {
                key: ZhangString::quote(KEY_DEFAULT_ROUNDING),
                value: ZhangString::quote(DEFAULT_ROUNDING.to_string()),
            }),
            SpanInfo::default(),
        ),
        Spanned::new(
            Directive::Option(Options {
                key: ZhangString::quote(KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION),
                value: ZhangString::quote(DEFAULT_BALANCE_TOLERANCE_PRECISION.to_string()),
            }),
            SpanInfo::default(),
        ),
    ]
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
                    r#"INSERT INTO commodities (name, precision, prefix, suffix, rounding)
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
