//! What the plugin is configured with.
//!
//! zhang hands every plugin instance a string-to-string config (Extism's plug-in config):
//!
//! - **flat keys**: the ledger's options (`option "k" "v"`), then the meta of the plugin's `plugin` directive,
//!   which wins on a conflict and keeps only the last value of a repeated key. `allowed_hosts` is the one meta
//!   key that never becomes a flat key.
//! - **reserved keys**, set by the host and winning over any option or meta of the same name:
//!   - `zhang.abi`: the plugin ABI version the host speaks, `"1"` ([`Config::abi`]).
//!   - `zhang.plugin`: the `plugin` directive as written, as JSON: the module, the positional arguments and
//!     every value of every meta key ([`Config::plugin`]).
//!   - `zhang.seed`: the plugin's deterministic seed, a decimal `u64` ([`Config::seed`],
//!     [`clock::rng`](crate::clock::rng)).
//!
//! A zhang older than plugin ABI v1 sets none of the reserved keys; [`Config::abi`] tells you.
//!
//! On top of that, [`Config::resolve`] looks a setting up through the dated `custom` directives of the ledger
//! (see [`custom`](crate::custom)) and the metadata of the entry being processed.
//!
//! Every value is a string as written; [`Values`] parses numbers, amounts, dates, booleans and accounts.

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Meta};

use crate::abi;
use crate::custom::{self, CustomEntry};

/// config key holding the plugin ABI version the host speaks
pub const ABI_KEY: &str = "zhang.abi";
/// config key holding the plugin's `plugin` directive as written, a [`PluginDirective`] as JSON
pub const PLUGIN_KEY: &str = "zhang.plugin";
/// config key holding the plugin's seed, a decimal `u64`
pub const SEED_KEY: &str = "zhang.seed";

/// the meta key granting network access, the one meta key that never becomes a flat config key
const ALLOWED_HOSTS_KEY: &str = "allowed_hosts";

/// The plugin's `plugin` directive as written, from the `zhang.plugin` config key. The directive
///
/// ```zhang
/// plugin "fx-rate.wasm" "USD" "strict"
///   allowed_hosts: "api.frankfurter.dev"
///   tag: "first"
///   tag: "second"
/// ```
///
/// gives `module` `"fx-rate.wasm"`, `args` `["USD", "strict"]` and `meta`
/// `{"allowed_hosts": ["api.frankfurter.dev"], "tag": ["first", "second"]}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDirective {
    /// the module as written in the directive
    #[serde(default)]
    pub module: String,
    /// the positional values after the module, in order; beancount's `plugin "module" "config"` passes its
    /// config string here
    #[serde(default)]
    pub args: Vec<String>,
    /// every meta key with all its values in source order, including capability keys such as `allowed_hosts`
    #[serde(default)]
    pub meta: BTreeMap<String, Vec<String>>,
}

/// A reserved config key is missing or unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// the host did not set `key`: it is older than plugin ABI v1
    Missing { key: &'static str },
    /// the host set `key` to something this SDK cannot read
    Invalid { key: &'static str, message: String },
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing { key } => write!(
                f,
                "the host sets no `{key}` config: this zhang is older than plugin ABI v1, upgrade zhang to run this plugin"
            ),
            ConfigError::Invalid { key, message } => write!(f, "the `{key}` config is not valid: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// where a setting came from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// the metadata of the entry being processed
    EntryMeta,
    /// a `custom "<plugin>" "<key>" …` directive dated this day
    Custom(NaiveDate),
    /// the meta of the plugin's `plugin` directive
    PluginMeta,
    /// a ledger option of that name. On a host older than plugin ABI v1, which merges the plugin's meta into
    /// the same keys, it may be the plugin's meta instead
    LedgerOption,
}

/// The string values of one setting, in order, with typed parsing. A setting has one value (a meta entry, an
/// option) or several (a `custom` directive, a repeated meta key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Values {
    source: Source,
    values: Vec<String>,
}

/// A value is missing or does not parse as the type asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError {
    /// the position of the value
    pub index: usize,
    /// the value, `None` when there is no value at `index`
    pub value: Option<String>,
    /// what the value should have been, e.g. `"a number"`
    pub expected: &'static str,
}

impl Display for ValueError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.value {
            None => write!(f, "there is no value at position {}, expected {}", self.index, self.expected),
            Some(value) => write!(f, "{value:?} at position {} is not {}", self.index, self.expected),
        }
    }
}

impl std::error::Error for ValueError {}

impl Values {
    /// these values, from `source`
    pub fn new(source: Source, values: impl IntoIterator<Item = impl Into<String>>) -> Values {
        Values {
            source,
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    /// where the values came from
    pub fn source(&self) -> Source {
        self.source
    }

    /// the values as written
    pub fn as_slice(&self) -> &[String] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.values.iter().map(String::as_str)
    }

    fn error(&self, index: usize, expected: &'static str) -> ValueError {
        ValueError {
            index,
            value: self.values.get(index).cloned(),
            expected,
        }
    }

    /// the value at `index` as written
    pub fn str(&self, index: usize) -> Result<&str, ValueError> {
        self.values.get(index).map(String::as_str).ok_or_else(|| self.error(index, "a value"))
    }

    /// the value at `index` parsed with [`FromStr`], e.g. `values.parse::<u32>(0)`
    pub fn parse<T: FromStr>(&self, index: usize) -> Result<T, ValueError> {
        self.str(index)?.trim().parse().map_err(|_| self.error(index, std::any::type_name::<T>()))
    }

    /// the value at `index` as a decimal number such as `100`, `-2.50` or `1e3`
    pub fn number(&self, index: usize) -> Result<BigDecimal, ValueError> {
        BigDecimal::from_str(self.str(index)?.trim()).map_err(|_| self.error(index, "a number"))
    }

    /// the value at `index` as a boolean: `true`, `yes` or `1`, or `false`, `no` or `0`, in any case
    pub fn bool(&self, index: usize) -> Result<bool, ValueError> {
        match self.str(index)?.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(true),
            "false" | "no" | "0" => Ok(false),
            _ => Err(self.error(index, "a boolean (true or false)")),
        }
    }

    /// the value at `index` as a date, `YYYY-MM-DD`
    pub fn date(&self, index: usize) -> Result<NaiveDate, ValueError> {
        NaiveDate::parse_from_str(self.str(index)?.trim(), "%Y-%m-%d").map_err(|_| self.error(index, "a date (YYYY-MM-DD)"))
    }

    /// the value at `index` as an account name such as `Assets:Bank`
    pub fn account(&self, index: usize) -> Result<Account, ValueError> {
        let value = self.str(index)?.trim();
        Account::from_str(value)
            .ok()
            .filter(|account| !account.components.is_empty() && account.components.iter().all(|it| !it.is_empty()))
            .ok_or_else(|| self.error(index, "an account (such as Assets:Bank)"))
    }

    /// The amount starting at `index`: either one value holding both parts (`"100 USD"`), or a number at
    /// `index` and its commodity at `index + 1` (`"100" "USD"`, as `custom "p" "k" 100 USD` is written).
    pub fn amount(&self, index: usize) -> Result<Amount, ValueError> {
        let value = self.str(index)?.trim();
        let (number, commodity, commodity_index) = match value.split_once(char::is_whitespace) {
            Some((number, commodity)) => (number, commodity.trim(), index),
            None => (value, self.str(index + 1).map_err(|_| self.error(index + 1, "a commodity"))?.trim(), index + 1),
        };
        let number = BigDecimal::from_str(number).map_err(|_| self.error(index, "an amount (such as 100 USD)"))?;
        if !is_commodity(commodity) {
            return Err(self.error(commodity_index, "a commodity"));
        }
        Ok(Amount::new(number, commodity))
    }
}

/// `commodity = ASCII_ALPHA (ASCII_ALPHANUMERIC | "." | "_" | "-" | "'")*`, as zhang writes commodities
fn is_commodity(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '\''))
}

/// where a [`Config`] reads its flat keys
#[derive(Debug, Clone)]
enum Keys {
    /// the host's config
    Host,
    /// fixed values, for tests
    Map(BTreeMap<String, String>),
}

/// The plugin's config: flat keys, the reserved `zhang.*` keys, and the `custom` directives given to
/// [`Config::with_custom`]. [`Config::load`] reads the host's config; [`Config::from_map`] builds one for tests.
#[derive(Debug, Clone)]
pub struct Config {
    keys: Keys,
    plugin: Result<PluginDirective, ConfigError>,
    custom: Vec<CustomEntry>,
}

impl Config {
    /// the config the host gave this plugin instance; empty on a native build
    pub fn load() -> Config {
        Config::with_keys(Keys::Host)
    }

    /// a config holding exactly these flat keys, reserved ones included, e.g. to unit test a plugin natively:
    ///
    /// ```
    /// use zhang_plugin_sdk::config::Config;
    ///
    /// let config = Config::from_map([("zhang.abi", "1"), ("currency", "USD")]);
    /// assert_eq!(config.abi(), Ok(1));
    /// assert_eq!(config.get("currency").unwrap().str(0), Ok("USD"));
    /// ```
    pub fn from_map<K: Into<String>, V: Into<String>>(entries: impl IntoIterator<Item = (K, V)>) -> Config {
        Config::with_keys(Keys::Map(entries.into_iter().map(|(key, value)| (key.into(), value.into())).collect()))
    }

    fn with_keys(keys: Keys) -> Config {
        let plugin = match lookup(&keys, PLUGIN_KEY) {
            None => Err(ConfigError::Missing { key: PLUGIN_KEY }),
            Some(json) => serde_json::from_str(&json).map_err(|e| ConfigError::Invalid {
                key: PLUGIN_KEY,
                message: e.to_string(),
            }),
        };
        Config { keys, plugin, custom: vec![] }
    }

    /// this config, resolving through `entries` too, usually [`custom::entries`] of the stream
    pub fn with_custom(mut self, entries: Vec<CustomEntry>) -> Config {
        self.custom = entries;
        self
    }

    /// the `custom` directives given to [`Config::with_custom`]
    pub fn custom(&self) -> &[CustomEntry] {
        &self.custom
    }

    /// The raw flat value of `key`: the plugin's meta (its last value) over a ledger option of that name.
    pub fn get(&self, key: &str) -> Option<Values> {
        let value = lookup(&self.keys, key)?;
        // the host consumes `allowed_hosts`: its meta values never become a flat key
        let in_meta = key != ALLOWED_HOSTS_KEY && self.plugin.as_ref().is_ok_and(|plugin| plugin.meta.contains_key(key));
        Some(Values::new(if in_meta { Source::PluginMeta } else { Source::LedgerOption }, [value]))
    }

    /// The plugin ABI version the host speaks. [`ConfigError::Missing`] means a zhang older than plugin ABI v1,
    /// which sets no reserved keys and links none of the host functions; a plugin needing them can fail with
    /// this error to tell the user to upgrade.
    pub fn abi(&self) -> Result<u32, ConfigError> {
        let value = lookup(&self.keys, ABI_KEY).ok_or(ConfigError::Missing { key: ABI_KEY })?;
        value.trim().parse().map_err(|_| ConfigError::Invalid {
            key: ABI_KEY,
            message: format!("{value:?} is not a version number"),
        })
    }

    /// the plugin's `plugin` directive as written
    pub fn plugin(&self) -> Result<&PluginDirective, ConfigError> {
        self.plugin.as_ref().map_err(Clone::clone)
    }

    /// every value of the meta key `key` of the plugin's `plugin` directive, in source order; `None` when the
    /// directive has no such key, or on a host older than plugin ABI v1
    pub fn meta(&self, key: &str) -> Option<Values> {
        let values = self.plugin.as_ref().ok()?.meta.get(key)?;
        Some(Values::new(Source::PluginMeta, values.iter().cloned()))
    }

    /// The ledger option `key`. The host merges options and the plugin's meta into the same flat keys, so an
    /// option is out of reach when the plugin's meta has the same key: then this is `None`.
    pub fn option(&self, key: &str) -> Option<Values> {
        self.get(key).filter(|values| values.source() == Source::LedgerOption)
    }

    /// the plugin's seed; see [`clock::rng`](crate::clock::rng)
    pub fn seed(&self) -> Result<u64, ConfigError> {
        parse_seed(lookup(&self.keys, SEED_KEY))
    }

    /// Look up the setting `key` for an entry dated `date` whose metadata is `entry_meta`, through these sources,
    /// the first one holding the key winning:
    ///
    /// 1. the entry's metadata, all its values of `key`;
    /// 2. the latest `custom "<plugin>" "<key>" …` dated on or before `date` among the
    ///    [`custom` entries](Config::with_custom); of several on that day, the last one;
    /// 3. the meta of the plugin's `plugin` directive, every value;
    /// 4. a ledger option named `key`.
    ///
    /// Positional plugin arguments have no key, so they never answer; read them from [`Config::plugin`].
    pub fn resolve(&self, key: &str, date: NaiveDate, entry_meta: Option<&Meta>) -> Option<Values> {
        if let Some(meta) = entry_meta {
            let values = meta.get_all(key);
            if !values.is_empty() {
                return Some(Values::new(Source::EntryMeta, values.into_iter().map(|it| it.as_str().to_owned())));
            }
        }
        if let Some(entry) = custom::latest(&self.custom, key, date) {
            return Some(entry.values.clone());
        }
        self.meta(key).or_else(|| self.option(key))
    }
}

fn lookup(keys: &Keys, key: &str) -> Option<String> {
    match keys {
        Keys::Host => abi::config_get(key),
        Keys::Map(map) => map.get(key).cloned(),
    }
}

/// the seed the host set, from the `zhang.seed` config value
pub(crate) fn host_seed() -> Result<u64, ConfigError> {
    parse_seed(abi::config_get(SEED_KEY))
}

fn parse_seed(value: Option<String>) -> Result<u64, ConfigError> {
    let value = value.ok_or(ConfigError::Missing { key: SEED_KEY })?;
    value.trim().parse().map_err(|_| ConfigError::Invalid {
        key: SEED_KEY,
        message: format!("{value:?} is not a decimal u64"),
    })
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Meta, ZhangString};

    use super::{Config, ConfigError, PluginDirective, Source, ValueError, Values};
    use crate::custom::CustomEntry;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn values(items: &[&str]) -> Values {
        Values::new(Source::LedgerOption, items.iter().copied())
    }

    fn amount(number: &str, commodity: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), commodity)
    }

    #[test]
    fn should_parse_amounts_written_as_one_or_two_values() {
        assert_eq!(values(&["100", "USD"]).amount(0), Ok(amount("100", "USD")));
        assert_eq!(values(&["100 USD"]).amount(0), Ok(amount("100", "USD")));
        assert_eq!(values(&["k", " -2.50  CNY "]).amount(1), Ok(amount("-2.50", "CNY")));
        assert_eq!(
            values(&["100"]).amount(0),
            Err(ValueError {
                index: 1,
                value: None,
                expected: "a commodity"
            })
        );
        assert_eq!(
            values(&["ten", "USD"]).amount(0).unwrap_err().to_string(),
            "\"ten\" at position 0 is not an amount (such as 100 USD)"
        );
        assert_eq!(values(&["100", "1USD"]).amount(0).unwrap_err().index, 1);
    }

    #[test]
    fn should_parse_numbers_booleans_dates_and_accounts() {
        let values = values(&["12.5", "Yes", "0", "2024-03-01", "Assets:Bank:Checking", "maybe", "Bank", "42"]);
        assert_eq!(values.number(0), Ok(BigDecimal::from_str("12.5").unwrap()));
        assert_eq!((values.bool(1), values.bool(2)), (Ok(true), Ok(false)));
        assert_eq!(values.date(3), Ok(date("2024-03-01")));
        assert_eq!(values.account(4).unwrap().name(), "Assets:Bank:Checking");
        assert_eq!(values.parse::<u32>(7), Ok(42));
        assert!(values.bool(5).is_err());
        assert!(values.date(5).is_err());
        assert!(values.number(5).is_err());
        assert!(values.account(6).is_err());
        assert_eq!(values.str(8).unwrap_err().to_string(), "there is no value at position 8, expected a value");
    }

    #[test]
    fn should_tell_an_old_host_apart() {
        let old = Config::from_map([("threshold", "100 USD")]);
        assert_eq!(old.abi(), Err(ConfigError::Missing { key: "zhang.abi" }));
        assert_eq!(old.plugin(), Err(ConfigError::Missing { key: "zhang.plugin" }));
        assert_eq!(old.seed(), Err(ConfigError::Missing { key: "zhang.seed" }));
        assert!(old.abi().unwrap_err().to_string().contains("older than plugin ABI v1"));
        // without `zhang.plugin` a flat key cannot be told apart from an option
        assert_eq!(old.get("threshold").unwrap().source(), Source::LedgerOption);
        assert_eq!(old.meta("threshold"), None);

        let broken = Config::from_map([("zhang.abi", "one"), ("zhang.plugin", "{"), ("zhang.seed", "-1")]);
        assert!(matches!(broken.abi(), Err(ConfigError::Invalid { key: "zhang.abi", .. })));
        assert!(matches!(broken.plugin(), Err(ConfigError::Invalid { key: "zhang.plugin", .. })));
        assert!(matches!(broken.seed(), Err(ConfigError::Invalid { key: "zhang.seed", .. })));
    }

    /// what the host gives a plugin declared as
    ///
    /// ```zhang
    /// option "threshold" "1000 USD"
    /// option "currency" "USD"
    /// plugin "guard.wasm" "strict"
    ///   threshold: "300 USD"
    ///   tag: "a"
    ///   tag: "b"
    /// ```
    fn v1_config() -> Config {
        let directive = r#"{"module":"guard.wasm","args":["strict"],"meta":{"tag":["a","b"],"threshold":["300 USD"]}}"#;
        Config::from_map([
            ("currency", "USD"),
            ("threshold", "300 USD"),
            ("tag", "b"),
            ("zhang.abi", "1"),
            ("zhang.plugin", directive),
            ("zhang.seed", "42"),
        ])
    }

    #[test]
    fn should_read_the_reserved_keys_of_a_v1_host() {
        let config = v1_config();
        assert_eq!(config.abi(), Ok(1));
        assert_eq!(config.seed(), Ok(42));
        assert_eq!(config.plugin().unwrap().args, vec!["strict".to_owned()]);
        assert_eq!(config.meta("tag").unwrap().as_slice(), ["a", "b"]);
        assert_eq!(config.get("tag").unwrap().as_slice(), ["b"]);
        assert_eq!(config.get("tag").unwrap().source(), Source::PluginMeta);
        assert_eq!(config.option("currency").unwrap().as_slice(), ["USD"]);
        assert_eq!(config.option("threshold"), None, "the plugin meta hides the option");
        // `allowed_hosts` is in the directive's meta but never a flat key
        let hosts = Config::from_map([
            ("allowed_hosts", "an option"),
            ("zhang.plugin", r#"{"module":"m","args":[],"meta":{"allowed_hosts":["api.example.com"]}}"#),
        ]);
        assert_eq!(hosts.option("allowed_hosts").unwrap().as_slice(), ["an option"]);
        assert_eq!(hosts.meta("allowed_hosts").unwrap().as_slice(), ["api.example.com"]);
        // a newer host may add fields
        let newer: PluginDirective = serde_json::from_str(r#"{"module":"m","args":[],"meta":{},"stage":"raw"}"#).unwrap();
        assert_eq!(newer.module, "m");
    }

    fn custom(day: &str, key: &str, items: &[&str]) -> CustomEntry {
        CustomEntry {
            date: date(day),
            key: key.to_owned(),
            values: Values::new(Source::Custom(date(day)), items.iter().copied()),
            meta: Meta::default(),
            span: Default::default(),
        }
    }

    #[test]
    fn should_resolve_entry_meta_then_custom_then_plugin_meta_then_option() {
        let config = v1_config().with_custom(vec![
            custom("2024-01-01", "threshold", &["100", "USD"]),
            custom("2024-03-01", "threshold", &["200", "USD"]),
            custom("2024-03-01", "threshold", &["250", "USD"]),
            custom("2024-02-01", "other", &["x"]),
        ]);
        let resolve = |key: &str, day: &str, meta: Option<&Meta>| config.resolve(key, date(day), meta).map(|it| (it.source(), it.as_slice().join(" ")));

        let mut meta = Meta::default();
        meta.insert("threshold".to_owned(), ZhangString::quote("50 USD"));
        assert_eq!(resolve("threshold", "2024-03-05", Some(&meta)), Some((Source::EntryMeta, "50 USD".to_owned())));
        // the latest custom on or before the date; of two on one day, the last
        assert_eq!(
            resolve("threshold", "2024-03-05", Some(&Meta::default())),
            Some((Source::Custom(date("2024-03-01")), "250 USD".to_owned()))
        );
        assert_eq!(
            resolve("threshold", "2024-03-01", None),
            Some((Source::Custom(date("2024-03-01")), "250 USD".to_owned()))
        );
        assert_eq!(
            resolve("threshold", "2024-02-29", None),
            Some((Source::Custom(date("2024-01-01")), "100 USD".to_owned()))
        );
        // before the first custom: the plugin's meta, every value
        assert_eq!(resolve("threshold", "2023-12-31", None), Some((Source::PluginMeta, "300 USD".to_owned())));
        assert_eq!(resolve("tag", "2024-03-05", None), Some((Source::PluginMeta, "a b".to_owned())));
        // then the ledger option
        assert_eq!(resolve("currency", "2024-03-05", None), Some((Source::LedgerOption, "USD".to_owned())));
        assert_eq!(resolve("missing", "2024-03-05", None), None);
        assert_eq!(
            config.resolve("threshold", date("2024-03-05"), None).unwrap().amount(0),
            Ok(super::Amount::new(BigDecimal::from(250), "USD"))
        );
    }
}
