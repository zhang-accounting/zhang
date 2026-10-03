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
//!   network. The host consumes it, so it is the only key that never reaches the plugin's config.
//! - `timeout` bounds how long a single call into the plugin may run, [`DEFAULT_TIMEOUT`] without
//!   it. The value is whole seconds (`"90"`) or a whole number with a unit `ms`, `s`, `m` or `h`
//!   (`"500ms"`, `"30s"`, `"2m"`), above zero and at most a day. A repeated key keeps its last
//!   value. An invalid value is a `ParseInvalidMeta` error on the directive, and the plugin gets the
//!   default.
//! - every other key reaches the plugin as a flat config entry; a repeated key keeps its last value.
//! - keys starting with [`RESERVED_CONFIG_PREFIX`] are reserved for values the host sets. A meta
//!   entry (or a ledger option) may still use one, but the host's value wins.
//!
//! Every other capability key (`timeout`, and later `allowed_paths`, `seed`, `stage`) is read here
//! *and* still passed through as config, so a plugin that already uses a meta key with that name
//! sees no change.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use log::warn;
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

/// a parsed `plugin` directive: its capabilities and the config it hands to the plugin
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginDeclaration {
    pub capabilities: PluginCapabilities,
    /// the plugin's own config, taken from its directive's meta
    pub config: BTreeMap<String, String>,
    /// the meta values the host could not use
    pub errors: Vec<DeclarationError>,
}

impl PluginDeclaration {
    /// the single place turning a `plugin` directive's meta into capabilities and config
    pub fn parse(directive: &Plugin) -> PluginDeclaration {
        let mut declaration = PluginDeclaration::default();
        let mut timeout = None;
        for (key, value) in directive.meta.clone().get_flatten() {
            let value = value.to_plain_string();
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
    use crate::plugin::capabilities::{DeclarationError, PluginCapabilities, PluginDeclaration, DEFAULT_TIMEOUT};
    use crate::utils::hashmap::HashMapOfExt;

    fn directive(meta: &[(&str, &str)]) -> Plugin {
        Plugin {
            module: ZhangString::quote("fx-rate.wasm"),
            value: vec![],
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
}
