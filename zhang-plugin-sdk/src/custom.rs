//! Config written in the ledger as dated `custom` directives.
//!
//! The convention: a plugin named `<plugin>` (what its `name` export returns) reads the directives
//!
//! ```zhang
//! 2024-01-01 custom "<plugin>" "<key>" <values…>
//! ```
//!
//! for example
//!
//! ```zhang
//! 2024-01-01 custom "large-expense" "threshold" 100 USD
//! 2024-07-01 custom "large-expense" "threshold" "150 USD"
//! ```
//!
//! Because they are dated, the config can change over time: an entry dated 2024-03-05 sees the threshold of
//! 2024-01-01, one dated 2024-08-01 the threshold of 2024-07-01. The values stay strings (`100 USD` arrives as
//! the two values `"100"` and `"USD"`); [`Values`] parses them, and [`Values::amount`] joins such a pair.
//!
//! Only a processor sees the whole stream, so only a processor can read `custom` config. A mapper sees one
//! directive at a time.
//!
//! ```
//! use zhang_plugin_sdk::config::Config;
//! use zhang_plugin_sdk::{custom, Error, Stream};
//!
//! fn process(stream: Stream) -> Result<Stream, Error> {
//!     let config = Config::load().with_custom(custom::entries("large-expense", &stream));
//!     // config.resolve("threshold", date, Some(&meta)) now sees the dated `custom` directives too
//!     Ok(stream)
//! }
//! # process(vec![]).unwrap();
//! ```

use chrono::NaiveDate;
use zhang_ast::{Directive, Meta, SpanInfo, Spanned, StringOrAccount};

use crate::config::{Source, Values};

/// one `custom "<plugin>" "<key>" <values…>` directive
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEntry {
    /// the directive's date (the day of a dated-time directive)
    pub date: NaiveDate,
    /// the first value after the plugin name
    pub key: String,
    /// the values after the key, from [`Source::Custom`]; empty for `custom "<plugin>" "<key>"`
    pub values: Values,
    /// the directive's metadata
    pub meta: Meta,
    /// where the directive is, e.g. to [report a problem](crate::errors::emit_error_at) with it
    pub span: SpanInfo,
}

/// every `custom "<plugin>" "<key>" …` directive of `stream`, in stream order. `plugin` is compared with the
/// directive's first string exactly; an account value counts as its full name
pub fn entries(plugin: &str, stream: &[Spanned<Directive>]) -> Vec<CustomEntry> {
    stream
        .iter()
        .filter_map(|directive| match &directive.data {
            Directive::Custom(custom) if custom.custom_type.as_str() == plugin => {
                let (key, values) = custom.values.split_first()?;
                let date = custom.date.naive_date();
                Some(CustomEntry {
                    date,
                    key: text(key),
                    values: Values::new(Source::Custom(date), values.iter().map(text)),
                    meta: custom.meta.clone(),
                    span: directive.span.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

/// the entry for `key` in effect on `date`: the latest one dated on or before it, and of several on that day the
/// last one in `entries`
pub fn latest<'a>(entries: &'a [CustomEntry], key: &str, date: NaiveDate) -> Option<&'a CustomEntry> {
    entries
        .iter()
        .filter(|entry| entry.key == key && entry.date <= date)
        .fold(None, |best: Option<&CustomEntry>, entry| match best {
            Some(best) if best.date > entry.date => Some(best),
            _ => Some(entry),
        })
}

fn text(value: &StringOrAccount) -> String {
    match value {
        StringOrAccount::String(string) => string.as_str().to_owned(),
        StringOrAccount::Account(account) => account.name().to_owned(),
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use chrono::NaiveDate;
    use zhang_ast::{Account, Custom, Date, Directive, Meta, SpanInfo, Spanned, StringOrAccount, ZhangString};

    use super::{entries, latest};
    use crate::config::Source;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn custom(day: &str, plugin: &str, values: Vec<StringOrAccount>) -> Spanned<Directive> {
        Spanned::new(
            Directive::Custom(Custom {
                date: Date::Date(date(day)),
                custom_type: ZhangString::quote(plugin),
                values,
                meta: Meta::default(),
            }),
            SpanInfo::simple(0, 10),
        )
    }

    fn string(value: &str) -> StringOrAccount {
        StringOrAccount::String(ZhangString::unquote(value))
    }

    #[test]
    fn should_collect_the_custom_directives_of_one_plugin() {
        let stream = vec![
            custom("2024-01-01", "guard", vec![string("threshold"), string("100"), string("USD")]),
            custom("2024-01-01", "other", vec![string("threshold"), string("1")]),
            custom(
                "2024-02-01",
                "guard",
                vec![string("account"), StringOrAccount::Account(Account::from_str("Expenses:Food").unwrap())],
            ),
            custom("2024-03-01", "guard", vec![string("strict")]),
        ];

        let found = entries("guard", &stream);

        assert_eq!(found.len(), 3);
        assert_eq!((found[0].date, found[0].key.as_str()), (date("2024-01-01"), "threshold"));
        assert_eq!(found[0].values.as_slice(), ["100", "USD"]);
        assert_eq!(found[0].values.source(), Source::Custom(date("2024-01-01")));
        assert_eq!(found[1].values.account(0).unwrap().name(), "Expenses:Food");
        assert!(found[2].values.is_empty());
        assert_eq!(found[2].span, SpanInfo::simple(0, 10));
    }

    #[test]
    fn should_pick_the_latest_entry_on_or_before_the_date() {
        let stream = vec![
            custom("2024-03-01", "guard", vec![string("threshold"), string("200 USD")]),
            custom("2024-01-01", "guard", vec![string("threshold"), string("100 USD")]),
            custom("2024-03-01", "guard", vec![string("threshold"), string("250 USD")]),
        ];
        let found = entries("guard", &stream);
        let value = |day: &str| latest(&found, "threshold", date(day)).map(|it| it.values.as_slice()[0].clone());

        assert_eq!(value("2023-12-31"), None);
        assert_eq!(value("2024-01-01").as_deref(), Some("100 USD"));
        assert_eq!(value("2024-02-29").as_deref(), Some("100 USD"));
        assert_eq!(value("2024-03-01").as_deref(), Some("250 USD"));
        assert_eq!(value("2025-01-01").as_deref(), Some("250 USD"));
        assert_eq!(latest(&found, "other", date("2025-01-01")), None);
    }
}
