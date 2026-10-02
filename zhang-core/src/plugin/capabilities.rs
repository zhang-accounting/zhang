//! What a `plugin` directive declares, parsed in one place.
//!
//! ```zhang
//! plugin "fx-rate.wasm"
//!   allowed_hosts: "api.frankfurter.dev"
//!   base_currency: "USD"
//! ```
//!
//! The meta of the directive carries both the capabilities the host grants the plugin and the
//! plugin's own config:
//! - `allowed_hosts` (multi-valued) grants HTTP access to those hosts only; no entry means no
//!   network. The host consumes it, so it is the only key that never becomes a flat config entry
//!   (the plugin still sees it in `zhang.plugin`, below).
//! - `timeout` bounds how long a single call into the plugin may run, [`DEFAULT_TIMEOUT`] without
//!   it. The value is whole seconds (`"90"`) or a whole number with a unit `ms`, `s`, `m` or `h`
//!   (`"500ms"`, `"30s"`, `"2m"`), above zero and at most a day. A repeated key keeps its last
//!   value. An invalid value is a `ParseInvalidMeta` error on the directive, and the plugin gets the
//!   default.
//! - every other key reaches the plugin as a flat config entry; a repeated key keeps its last value.
//! - keys starting with [`RESERVED_CONFIG_PREFIX`] are reserved for values the host sets. A meta
//!   entry (or a ledger option) may still use one, but the host's value wins. The host sets
//!   [`ABI_CONFIG_KEY`] (`zhang.abi`) to [`ABI_VERSION`], and [`PLUGIN_CONFIG_KEY`] (`zhang.plugin`)
//!   to the directive as written: its positional arguments and every meta value, as JSON (see
//!   [`PluginDirectiveConfig`]).
//!
//! Every other capability key (`timeout`, and later `allowed_paths`, `seed`, `stage`) is read here
//! *and* still passed through as config, so a plugin that already uses a meta key with that name
//! sees no change.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use log::warn;
use serde::{Deserialize, Serialize};
use zhang_ast::error::ErrorKind;
use zhang_ast::Plugin;

use crate::domains::schemas::OptionDomain;
use crate::utils::hashmap::HashMapOfExt;

/// meta key on a `plugin` directive granting it HTTP access to the given hosts
const ALLOWED_HOSTS_KEY: &str = "allowed_hosts";

/// meta key on a `plugin` directive bounding how long a single call into the plugin may run
const TIMEOUT_KEY: &str = "timeout";

/// how long a single call into a plugin may run when its directive declares no `timeout`
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// the longest `timeout` a plugin may declare
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// prefix of the config keys reserved for values the host sets
pub const RESERVED_CONFIG_PREFIX: &str = "zhang.";

/// config key holding the version of the plugin ABI the host speaks, [`ABI_VERSION`]
pub const ABI_CONFIG_KEY: &str = "zhang.abi";

/// the plugin ABI version this host speaks, a decimal integer as a string. Additions a plugin can
/// ignore (a new config key, a new host function) keep the version
pub const ABI_VERSION: &str = "1";

/// config key holding the plugin's directive as written, a [`PluginDirectiveConfig`] as JSON
pub const PLUGIN_CONFIG_KEY: &str = "zhang.plugin";

/// what the host grants a plugin, as declared by its directive's meta
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCapabilities {
    /// hosts this plugin may reach over HTTP; empty means no network access
    pub allowed_hosts: Vec<String>,
    /// how long a single call into the plugin may run before the host stops it
    pub timeout: Duration,
}

impl Default for PluginCapabilities {
    fn default() -> Self {
        PluginCapabilities {
            allowed_hosts: vec![],
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// a meta value the host cannot use, for the `plugin` directive to report; the host keeps its default
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationError {
    pub kind: ErrorKind,
    pub metas: HashMap<String, String>,
}

/// The plugin's directive as written, which the plugin receives as JSON under the config key
/// [`PLUGIN_CONFIG_KEY`] (`zhang.plugin`). It carries what the flat config cannot: the positional
/// arguments, and every value of a repeated meta key.
///
/// This shape is part of plugin ABI v1 ([`ABI_VERSION`]): fields may be added, never removed or
/// changed. The directive
///
/// ```zhang
/// plugin "fx-rate.wasm" "USD" "strict"
///   allowed_hosts: "api.frankfurter.dev"
///   tag: "first"
///   tag: "second"
/// ```
///
/// reaches the plugin as (compact JSON, wrapped here for reading)
///
/// ```json
/// {"module":"fx-rate.wasm","args":["USD","strict"],
///  "meta":{"allowed_hosts":["api.frankfurter.dev"],"tag":["first","second"]}}
/// ```
///
/// - `module` (string): the module as written in the directive.
/// - `args` (array of strings): the positional values after the module, in order; empty when there
///   are none. Beancount's `plugin "module" "config"` passes its config string here.
/// - `meta` (object of string arrays): every meta key of the directive with all its values in
///   source order; the keys are sorted. Unlike the flat config it keeps every value of a repeated
///   key, and it includes the capability keys such as `allowed_hosts` (a grant is not secret, and
///   a plugin can inspect what it was granted) as well as meta keys starting with
///   [`RESERVED_CONFIG_PREFIX`].
///
/// Every value is a string, exactly as written; parsing amounts, dates or numbers is up to the
/// plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDirectiveConfig {
    pub module: String,
    pub args: Vec<String>,
    pub meta: BTreeMap<String, Vec<String>>,
}

/// a parsed `plugin` directive: its capabilities and the config it hands to the plugin
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginDeclaration {
    pub capabilities: PluginCapabilities,
    /// the plugin's own config, taken from its directive's meta
    pub config: BTreeMap<String, String>,
    /// the meta values the host could not use
    pub errors: Vec<DeclarationError>,
    /// the directive as written, handed to the plugin as [`PLUGIN_CONFIG_KEY`]
    pub directive: PluginDirectiveConfig,
}

impl PluginDeclaration {
    /// the single place turning a `plugin` directive's meta into capabilities and config
    pub fn parse(directive: &Plugin) -> PluginDeclaration {
        let mut declaration = PluginDeclaration {
            directive: PluginDirectiveConfig {
                module: directive.module.as_str().to_owned(),
                args: directive.value.iter().map(|it| it.as_str().to_owned()).collect(),
                meta: BTreeMap::new(),
            },
            ..PluginDeclaration::default()
        };
        let mut timeout = None;
        // `get_flatten` yields the values of one key together, in source order
        for (key, value) in directive.meta.clone().get_flatten() {
            let value = value.to_plain_string();
            declaration.directive.meta.entry(key.clone()).or_default().push(value.clone());
            match key.as_str() {
                // consumed by the host. A later capability key gets its own arm that reads the value
                // and still inserts it into the config below.
                ALLOWED_HOSTS_KEY => declaration.capabilities.allowed_hosts.push(value),
                TIMEOUT_KEY => {
                    timeout = Some(value.clone());
                    declaration.config.insert(key, value);
                }
                _ => {
                    if key.starts_with(RESERVED_CONFIG_PREFIX) {
                        warn!(
                            "plugin {}: the meta key {key} is reserved for the host (prefix {RESERVED_CONFIG_PREFIX}); a value the host sets for it wins",
                            directive.module.as_str()
                        );
                    }
                    declaration.config.insert(key, value);
                }
            }
        }
        if let Some(value) = timeout {
            match parse_timeout(&value) {
                Some(timeout) => declaration.capabilities.timeout = timeout,
                None => declaration.errors.push(DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", directive.module.as_str(), TIMEOUT_KEY, value),
                }),
            }
        }
        declaration
    }

    /// the config values the host sets under [`RESERVED_CONFIG_PREFIX`] for this plugin:
    /// [`ABI_CONFIG_KEY`] and [`PLUGIN_CONFIG_KEY`]. Pass them to [`PluginDeclaration::config_with`]
    pub fn host_config(&self) -> Vec<(String, String)> {
        let directive = serde_json::to_string(&self.directive).expect("strings, arrays and maps of strings always serialize to JSON");
        vec![(ABI_CONFIG_KEY.to_owned(), ABI_VERSION.to_owned()), (PLUGIN_CONFIG_KEY.to_owned(), directive)]
    }

    /// the flat config the plugin receives. On a key conflict a later layer wins:
    /// the ledger's options, then the plugin's own config, then `reserved`, the values the host sets
    /// under [`RESERVED_CONFIG_PREFIX`].
    pub fn config_with(&self, options: &[OptionDomain], reserved: impl IntoIterator<Item = (String, String)>) -> BTreeMap<String, String> {
        let mut config: BTreeMap<String, String> = options.iter().map(|it| (it.key.clone(), it.value.clone())).collect();
        config.extend(self.config.iter().map(|(key, value)| (key.clone(), value.clone())));
        config.extend(reserved);
        config
    }
}

/// a `timeout` meta value: whole seconds, or a whole number with a unit `ms`, `s`, `m` or `h`.
/// `None` for anything else, for zero and for more than [`MAX_TIMEOUT`]
fn parse_timeout(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (amount, unit) = value.split_at(value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len()));
    let amount: u64 = amount.parse().ok()?;
    let timeout = match unit.trim_start() {
        "" | "s" => Duration::from_secs(amount),
        "ms" => Duration::from_millis(amount),
        "m" => Duration::from_secs(amount.checked_mul(60)?),
        "h" => Duration::from_secs(amount.checked_mul(60 * 60)?),
        _ => return None,
    };
    (!timeout.is_zero() && timeout <= MAX_TIMEOUT).then_some(timeout)
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashMap};
    use std::time::Duration;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Meta, Plugin, ZhangString};

    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::{DeclarationError, PluginCapabilities, PluginDeclaration, PluginDirectiveConfig, DEFAULT_TIMEOUT};
    use crate::utils::hashmap::HashMapOfExt;

    fn directive(meta: &[(&str, &str)]) -> Plugin {
        directive_with_args(&[], meta)
    }

    fn directive_with_args(args: &[&str], meta: &[(&str, &str)]) -> Plugin {
        Plugin {
            module: ZhangString::quote("fx-rate.wasm"),
            value: args.iter().map(|it| ZhangString::quote(*it)).collect(),
            meta: meta.iter().map(|(key, value)| (key.to_string(), ZhangString::quote(*value))).collect::<Meta>(),
        }
    }

    fn option(key: &str, value: &str) -> OptionDomain {
        OptionDomain {
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn should_strip_allowed_hosts_and_keep_their_order() {
        let declaration = PluginDeclaration::parse(&directive(&[
            ("allowed_hosts", "b.example"),
            ("base_currency", "USD"),
            ("allowed_hosts", "a.example"),
        ]));

        assert_eq!(
            declaration.capabilities,
            PluginCapabilities {
                allowed_hosts: vec!["b.example".to_owned(), "a.example".to_owned()],
                ..PluginCapabilities::default()
            }
        );
        assert_eq!(declaration.config, map(&[("base_currency", "USD")]));
    }

    #[test]
    fn should_keep_the_last_value_of_a_repeated_key() {
        let declaration = PluginDeclaration::parse(&directive(&[("tag", "first"), ("tag", "second")]));

        assert_eq!(declaration.config, map(&[("tag", "second")]));
    }

    #[test]
    fn should_pass_reserved_meta_keys_through() {
        let declaration = PluginDeclaration::parse(&directive(&[("zhang.mine", "kept")]));

        assert_eq!(declaration.config, map(&[("zhang.mine", "kept")]));
    }

    #[test]
    fn should_let_meta_override_options_and_the_host_override_both() {
        let declaration = PluginDeclaration::parse(&directive(&[("operating_currency", "EUR"), ("zhang.abi", "from meta")]));
        let options = [
            option("operating_currency", "CNY"),
            option("zhang.abi", "from option"),
            option("timezone", "UTC"),
        ];

        assert_eq!(
            declaration.config_with(&options, []),
            map(&[("operating_currency", "EUR"), ("timezone", "UTC"), ("zhang.abi", "from meta")])
        );
        assert_eq!(
            declaration.config_with(&options, [("zhang.abi".to_owned(), "1".to_owned())]),
            map(&[("operating_currency", "EUR"), ("timezone", "UTC"), ("zhang.abi", "1")])
        );
    }

    #[test]
    fn should_give_a_plugin_without_timeout_the_default_of_a_minute() {
        let declaration = PluginDeclaration::parse(&directive(&[("base_currency", "USD")]));

        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(60));
        assert_eq!(declaration.capabilities.timeout, DEFAULT_TIMEOUT);
        assert_eq!(declaration.errors, vec![]);
    }

    #[test]
    fn should_read_the_timeout_and_still_pass_it_through() {
        for (value, timeout) in [
            ("90", Duration::from_secs(90)),
            ("30s", Duration::from_secs(30)),
            (" 2m ", Duration::from_secs(120)),
            ("500ms", Duration::from_millis(500)),
            ("1 h", Duration::from_secs(3600)),
            ("24h", Duration::from_secs(24 * 3600)),
        ] {
            let declaration = PluginDeclaration::parse(&directive(&[("timeout", value)]));

            assert_eq!(declaration.capabilities.timeout, timeout, "timeout {value:?}");
            assert_eq!(declaration.errors, vec![], "timeout {value:?}");
            assert_eq!(declaration.config, map(&[("timeout", value)]), "timeout {value:?}");
        }
    }

    #[test]
    fn should_use_the_last_timeout() {
        let declaration = PluginDeclaration::parse(&directive(&[("timeout", "soon"), ("timeout", "5")]));

        assert_eq!(declaration.capabilities.timeout, Duration::from_secs(5));
        assert_eq!(declaration.errors, vec![]);
    }

    #[test]
    fn should_report_an_invalid_timeout_and_fall_back_to_the_default() {
        for value in ["soon", "", "0", "0s", "-5", "1.5s", "30sec", "2M", "25h", "18446744073709551616"] {
            let declaration = PluginDeclaration::parse(&directive(&[("timeout", value)]));

            assert_eq!(declaration.capabilities.timeout, DEFAULT_TIMEOUT, "timeout {value:?}");
            assert_eq!(
                declaration.errors,
                vec![DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", "fx-rate.wasm", "timeout", value),
                }],
                "timeout {value:?}"
            );
            assert_eq!(declaration.config, map(&[("timeout", value)]), "timeout {value:?}");
        }
    }

    #[test]
    fn should_pass_the_directive_as_written_as_json() {
        let declaration = PluginDeclaration::parse(&directive_with_args(
            &["USD", "strict \"mode\""],
            &[
                ("tag", "first"),
                ("allowed_hosts", "b.example"),
                ("base_currency", "USD"),
                ("tag", "second"),
                ("allowed_hosts", "a.example"),
            ],
        ));

        assert_eq!(
            declaration.directive,
            PluginDirectiveConfig {
                module: "fx-rate.wasm".to_owned(),
                args: vec!["USD".to_owned(), "strict \"mode\"".to_owned()],
                meta: [
                    ("allowed_hosts", vec!["b.example", "a.example"]),
                    ("base_currency", vec!["USD"]),
                    ("tag", vec!["first", "second"]),
                ]
                .into_iter()
                .map(|(key, values)| (key.to_owned(), values.into_iter().map(str::to_owned).collect()))
                .collect(),
            }
        );
        // the exact bytes are ABI: field order module, args, meta; sorted meta keys; values in source order
        assert_eq!(
            declaration.config_with(&[], declaration.host_config()),
            map(&[
                ("base_currency", "USD"),
                ("tag", "second"),
                ("zhang.abi", "1"),
                (
                    "zhang.plugin",
                    r#"{"module":"fx-rate.wasm","args":["USD","strict \"mode\""],"meta":{"allowed_hosts":["b.example","a.example"],"base_currency":["USD"],"tag":["first","second"]}}"#
                ),
            ])
        );
    }

    #[test]
    fn should_pass_empty_args_and_meta_for_a_bare_directive() {
        let declaration = PluginDeclaration::parse(&directive(&[]));

        assert_eq!(
            declaration.config_with(&[], declaration.host_config()),
            map(&[("zhang.abi", "1"), ("zhang.plugin", r#"{"module":"fx-rate.wasm","args":[],"meta":{}}"#)])
        );
    }

    #[test]
    fn should_let_the_host_values_win_over_meta_and_options_named_like_them() {
        let declaration = PluginDeclaration::parse(&directive(&[("zhang.plugin", "from meta"), ("zhang.abi", "0")]));
        let options = [option("zhang.plugin", "from option")];

        assert_eq!(
            declaration.config_with(&options, declaration.host_config()),
            map(&[
                ("zhang.abi", "1"),
                (
                    "zhang.plugin",
                    r#"{"module":"fx-rate.wasm","args":[],"meta":{"zhang.abi":["0"],"zhang.plugin":["from meta"]}}"#
                ),
            ])
        );
    }
}
