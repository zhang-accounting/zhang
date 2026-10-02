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
//! - every other key reaches the plugin as a flat config entry; a repeated key keeps its last value.
//! - keys starting with [`RESERVED_CONFIG_PREFIX`] are reserved for values the host sets. A meta
//!   entry (or a ledger option) may still use one, but the host's value wins.
//!
//! A capability key the host learns to read later (`allowed_paths`, `timeout`, `seed`, `stage`) is
//! read here *and* still passed through as config, so a plugin that already uses a meta key with
//! that name sees no change.

use std::collections::BTreeMap;

use log::warn;
use zhang_ast::Plugin;

use crate::domains::schemas::OptionDomain;

/// meta key on a `plugin` directive granting it HTTP access to the given hosts
const ALLOWED_HOSTS_KEY: &str = "allowed_hosts";

/// prefix of the config keys reserved for values the host sets
pub const RESERVED_CONFIG_PREFIX: &str = "zhang.";

/// what the host grants a plugin, as declared by its directive's meta
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCapabilities {
    /// hosts this plugin may reach over HTTP; empty means no network access
    pub allowed_hosts: Vec<String>,
}

/// a parsed `plugin` directive: its capabilities and the config it hands to the plugin
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginDeclaration {
    pub capabilities: PluginCapabilities,
    /// the plugin's own config, taken from its directive's meta
    pub config: BTreeMap<String, String>,
}

impl PluginDeclaration {
    /// the single place turning a `plugin` directive's meta into capabilities and config
    pub fn parse(directive: &Plugin) -> PluginDeclaration {
        let mut declaration = PluginDeclaration::default();
        for (key, value) in directive.meta.clone().get_flatten() {
            let value = value.to_plain_string();
            match key.as_str() {
                // consumed by the host. A later capability key gets its own arm that reads the value
                // and still inserts it into the config below.
                ALLOWED_HOSTS_KEY => declaration.capabilities.allowed_hosts.push(value),
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

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use zhang_ast::{Meta, Plugin, ZhangString};

    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::{PluginCapabilities, PluginDeclaration};

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
                allowed_hosts: vec!["b.example".to_owned(), "a.example".to_owned()]
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
}
