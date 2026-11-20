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
//! - `allowed_paths` (multi-valued) grants read-only access to files in the ledger, through the
//!   `zhang_read_file` and `zhang_list_dir` host functions ([`crate::plugin::host`]); no entry means no
//!   file access. Each value is a file or a directory relative to the ledger root, written with `/`;
//!   `"."` grants the whole root. A directory grants everything under it, compared component by
//!   component (`documents` does not grant `documents-private`), except hidden entries: below a grant,
//!   a name starting with `.` is readable only when a value names it, so `"."` does not expose
//!   `.git/config` or `.env`, while `".config"` grants `.config/…`. An empty or absolute value, or one
//!   holding a `..` component, a NUL or a backslash, is a `ParseInvalidMeta` error on the directive and
//!   grants nothing. Files are read from the ledger's own source, so this works for local ledgers and
//!   for ledgers on S3, WebDAV or GitHub alike, and nothing outside the ledger root can be read (see
//!   [`crate::plugin::files`]).
//!
//!   **Warning:** `allowed_paths` together with `allowed_hosts` lets a plugin send what it reads off the
//!   machine. A plugin already receives the whole ledger, so grant files and hosts together only to a
//!   plugin you would trust with both.
//! - `allowed_writes` (multi-valued) lets a router plugin append directives of these kinds to the ledger
//!   through the `zhang_write_preview` and `zhang_write_commit` host functions ([`crate::plugin::write`]):
//!   `transaction` or `open`. No entry means no writes, whatever the plugin declares. Any other value is a
//!   `ParseInvalidMeta` error on the directive and grants nothing.
//!
//!   **Warning:** a plugin granted `allowed_writes` can add entries to the ledger's files. It cannot edit
//!   or delete anything, and a well-behaved client shows every entry first, but grant it only to a plugin
//!   you trust.
//! - `timeout` bounds how long a single call into the plugin may run, [`DEFAULT_TIMEOUT`] without
//!   it. The value is whole seconds (`"90"`) or a whole number with a unit `ms`, `s`, `m` or `h`
//!   (`"500ms"`, `"30s"`, `"2m"`), above zero and at most a day. A repeated key keeps its last
//!   value. An invalid value is a `ParseInvalidMeta` error on the directive, and the plugin gets the
//!   default.
//! - `seed` is mixed into the plugin's seed, [`SEED_CONFIG_KEY`] below; a repeated key keeps its
//!   last value.
//! - `stage` chooses the stream a processor or mapper runs on ([`PluginStage`]): `"booked"`, the
//!   default, runs it after the booking stage, on booked postings; `"raw"` runs it before, on the
//!   transactions as written. A repeated key keeps its last value. Any other value is a
//!   `ParseInvalidMeta` error on the directive, and the plugin runs booked.
//! - every other key reaches the plugin as a flat config entry; a repeated key keeps its last value.
//! - keys starting with [`RESERVED_CONFIG_PREFIX`] are reserved for values the host sets. A meta
//!   entry (or a ledger option) may still use one, but the host's value wins. The host sets
//!   [`ABI_CONFIG_KEY`] (`zhang.abi`) to [`ABI_VERSION`], [`PLUGIN_CONFIG_KEY`] (`zhang.plugin`)
//!   to the directive as written: its positional arguments and every meta value, as JSON (see
//!   [`PluginDirectiveConfig`]), and [`SEED_CONFIG_KEY`] (`zhang.seed`) to the plugin's seed (see
//!   [`plugin_seed`]).
//!
//! What a directive grants is not all a plugin gets: the operator's
//! [`PluginPolicy`](crate::plugin_policy::PluginPolicy) bounds `allowed_hosts`, `timeout` and `allowed_writes`
//! ([`PluginCapabilities::bound_by`]), and the load reports what a directive asked for beyond it.
//!
//! Every other capability key (`allowed_paths`, `allowed_writes`, `timeout`, `seed`, `stage`) is read here *and*
//! still passed through as config, so a plugin that already uses a meta key with that name
//! sees no change, and a plugin can see what it was granted.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use log::warn;
use zhang_ast::error::ErrorKind;
use zhang_ast::Plugin;
pub use zhang_shared::plugin_abi::WriteKind;

use crate::domains::schemas::OptionDomain;
pub use crate::pipeline::PluginStage;
use crate::plugin::files::clean_path;
use crate::plugin_policy::{parse_timeout, PluginPolicy, ALLOWED_HOSTS_ENV, MAX_TIMEOUT_ENV};
use crate::utils::hashmap::HashMapOfExt;

/// meta key on a `plugin` directive granting it HTTP access to the given hosts
const ALLOWED_HOSTS_KEY: &str = "allowed_hosts";

/// meta key on a `plugin` directive granting it read access to the given files and directories
const ALLOWED_PATHS_KEY: &str = "allowed_paths";

/// meta key on a `plugin` directive letting it append directives of the given kinds, a [`WriteKind`] each
pub const ALLOWED_WRITES_KEY: &str = "allowed_writes";

/// meta key on a `plugin` directive bounding how long a single call into the plugin may run
const TIMEOUT_KEY: &str = "timeout";

/// how long a single call into a plugin may run when its directive declares no `timeout`
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// meta key on a `plugin` directive mixed into the plugin's seed, [`SEED_CONFIG_KEY`]
const SEED_KEY: &str = "seed";

/// meta key on a `plugin` directive choosing the stream its processor or mapper runs on, a [`PluginStage`]
const STAGE_KEY: &str = "stage";

pub use zhang_shared::plugin_abi::config::{
    ABI as ABI_CONFIG_KEY, ALLOWED_HOSTS as ALLOWED_HOSTS_CONFIG_KEY, PLUGIN as PLUGIN_CONFIG_KEY, RESERVED_PREFIX as RESERVED_CONFIG_PREFIX,
    SEED as SEED_CONFIG_KEY, TIMEOUT_MS as TIMEOUT_MS_CONFIG_KEY,
};
/// The plugin's directive as written, which the plugin receives as JSON under the config key
/// [`PLUGIN_CONFIG_KEY`] (`zhang.plugin`): its positional arguments and every value of its meta keys.
pub use zhang_shared::plugin_abi::PluginDirective as PluginDirectiveConfig;
pub use zhang_shared::plugin_abi::ABI_VERSION;

/// the domain separating [`plugin_seed`] hashes from any other use of SHA-256; the `v1` is part of the derivation
const SEED_DOMAIN: &[u8] = b"zhang/plugin-seed/v1";

/// what the host grants a plugin, as declared by its directive's meta
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCapabilities {
    /// hosts this plugin may reach over HTTP; empty means no network access
    pub allowed_hosts: Vec<String>,
    /// files and directories this plugin may read, relative to the ledger root and cleaned (empty for the
    /// whole root); empty means no file access
    pub allowed_paths: Vec<PathBuf>,
    /// the kinds of directives this plugin may append, each once, in the order granted; empty means no writes
    pub allowed_writes: Vec<WriteKind>,
    /// how long a single call into the plugin may run before the host stops it
    pub timeout: Duration,
    /// the `seed` meta, mixed into the plugin's seed; `None` without one
    pub seed: Option<String>,
    /// where the plugin's processor and mapper run, the `stage` meta
    pub stage: PluginStage,
}

impl Default for PluginCapabilities {
    fn default() -> Self {
        PluginCapabilities {
            allowed_hosts: vec![],
            allowed_paths: vec![],
            allowed_writes: vec![],
            timeout: DEFAULT_TIMEOUT,
            seed: None,
            stage: PluginStage::Booked,
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
    /// the directive as written, handed to the plugin as [`PLUGIN_CONFIG_KEY`]
    pub directive: PluginDirectiveConfig,
}

impl PluginDeclaration {
    /// whether the directive sets its plugin's timeout: it has a `timeout` meta the host could use
    pub fn declares_timeout(&self) -> bool {
        self.directive.meta.contains_key(TIMEOUT_KEY) && !self.errors.iter().any(|error| error.metas.contains_key(TIMEOUT_KEY))
    }

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
        let mut stage = None;
        // `get_flatten` yields the values of one key together, in source order
        for (key, value) in directive.meta.clone().get_flatten() {
            let value = value.to_plain_string();
            declaration.directive.meta.entry(key.clone()).or_default().push(value.clone());
            match key.as_str() {
                // consumed by the host. A later capability key gets its own arm that reads the value
                // and still inserts it into the config below.
                ALLOWED_HOSTS_KEY => declaration.capabilities.allowed_hosts.push(value),
                ALLOWED_PATHS_KEY => {
                    match clean_path(&value) {
                        Ok(path) => declaration.capabilities.allowed_paths.push(path),
                        Err(_) => declaration.errors.push(DeclarationError {
                            kind: ErrorKind::ParseInvalidMeta,
                            metas: HashMap::of2("plugin", directive.module.as_str(), ALLOWED_PATHS_KEY, value.clone()),
                        }),
                    }
                    declaration.config.insert(key, value);
                }
                ALLOWED_WRITES_KEY => {
                    match WriteKind::parse(value.trim()) {
                        Some(kind) => {
                            if !declaration.capabilities.allowed_writes.contains(&kind) {
                                declaration.capabilities.allowed_writes.push(kind);
                            }
                        }
                        None => declaration.errors.push(DeclarationError {
                            kind: ErrorKind::ParseInvalidMeta,
                            metas: HashMap::of2("plugin", directive.module.as_str(), ALLOWED_WRITES_KEY, value.clone()),
                        }),
                    }
                    declaration.config.insert(key, value);
                }
                TIMEOUT_KEY => {
                    timeout = Some(value.clone());
                    declaration.config.insert(key, value);
                }
                SEED_KEY => {
                    declaration.capabilities.seed = Some(value.clone());
                    declaration.config.insert(key, value);
                }
                STAGE_KEY => {
                    stage = Some(value.clone());
                    declaration.config.insert(key, value);
                }
                _ => {
                    if key.starts_with(RESERVED_CONFIG_PREFIX) {
                        warn!(
                            "plugin {}: the meta key {} is reserved for the host (prefix {RESERVED_CONFIG_PREFIX}); a value the host sets for it wins",
                            crate::plugin::log_safe(directive.module.as_str()),
                            crate::plugin::log_safe(&key)
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
        if let Some(value) = stage {
            match PluginStage::parse(&value) {
                Some(stage) => declaration.capabilities.stage = stage,
                None => declaration.errors.push(DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", directive.module.as_str(), STAGE_KEY, value),
                }),
            }
        }
        declaration
    }

    /// the config values the host sets under [`RESERVED_CONFIG_PREFIX`] for this plugin:
    /// [`ABI_CONFIG_KEY`], [`PLUGIN_CONFIG_KEY`], [`SEED_CONFIG_KEY`], and the capabilities the plugin has, which the
    /// operator's policy may have bounded ([`PluginCapabilities::bound_by`]): [`ALLOWED_HOSTS_CONFIG_KEY`] and
    /// [`TIMEOUT_MS_CONFIG_KEY`]. `occurrence` is the number of `plugin` directives before this one declaring the same
    /// module. Pass them to [`PluginDeclaration::config_with`]
    pub fn host_config(&self, occurrence: usize) -> Vec<(String, String)> {
        let directive = serde_json::to_string(&self.directive).expect("strings, arrays and maps of strings always serialize to JSON");
        let seed = plugin_seed(&self.directive.module, occurrence, self.capabilities.seed.as_deref());
        let allowed_hosts = serde_json::to_string(&self.capabilities.allowed_hosts).expect("strings always serialize to JSON");
        vec![
            (ABI_CONFIG_KEY.to_owned(), ABI_VERSION.to_string()),
            (PLUGIN_CONFIG_KEY.to_owned(), directive),
            (SEED_CONFIG_KEY.to_owned(), seed.to_string()),
            (ALLOWED_HOSTS_CONFIG_KEY.to_owned(), allowed_hosts),
            (TIMEOUT_MS_CONFIG_KEY.to_owned(), self.capabilities.timeout.as_millis().to_string()),
        ]
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

/// The seed of a plugin, which it receives as the decimal string of [`SEED_CONFIG_KEY`] (`zhang.seed`), so the ids
/// and links it generates stay the same on every reload.
///
/// It depends only on the `plugin` directive: `module`, the module as written; `occurrence`, the number of `plugin`
/// directives before this one declaring the same module, so two declarations of one module get different seeds; and
/// `seed`, the directive's `seed` meta if it has one, which changes the seed without moving the directive. It does
/// not depend on other modules, on the order of their directives, on the date or on the ledger's content. Writing
/// the module differently (`./x.wasm` for `x.wasm`) changes it.
///
/// The derivation is part of plugin ABI v1 ([`ABI_VERSION`]), so a seed never changes between zhang versions: the
/// first 8 bytes, read as a big-endian `u64`, of the SHA-256 of
///
/// ```text
/// "zhang/plugin-seed/v1" ‖ len(module) ‖ module ‖ occurrence [‖ len(seed) ‖ seed]
/// ```
///
/// where strings are UTF-8, lengths count bytes, `len(…)` and `occurrence` are big-endian `u64`s, and the bracketed
/// part is present only with a `seed` meta.
///
/// The host cannot stop a plugin targeting WASI from reading the host's own entropy and clock through WASI (extism
/// links them in, and its host functions cannot intercept them). Reproducibility is a contract: derive randomness
/// from `zhang.seed`, and read the time with the `zhang_now` host function.
pub fn plugin_seed(module: &str, occurrence: usize, seed: Option<&str>) -> u64 {
    fn push_str(message: &mut Vec<u8>, value: &str) {
        message.extend_from_slice(&(value.len() as u64).to_be_bytes());
        message.extend_from_slice(value.as_bytes());
    }
    let mut message = SEED_DOMAIN.to_vec();
    push_str(&mut message, module);
    message.extend_from_slice(&(occurrence as u64).to_be_bytes());
    if let Some(seed) = seed {
        push_str(&mut message, seed);
    }
    let hash = sha256::digest(message.as_slice());
    u64::from_str_radix(&hash[..16], 16).expect("a SHA-256 digest is hexadecimal")
}

/// Whether the host pattern `pattern` allows `host`, the way extism has always matched `allowed_hosts`: a glob
/// (`*` any run of characters, `?` one, `[...]` one of a set), or the pattern as written when it is no valid glob.
/// Both are compared in lower case
pub fn host_allowed(pattern: &str, host: &str) -> bool {
    let (pattern, host) = (pattern.to_ascii_lowercase(), host.to_ascii_lowercase());
    match glob::Pattern::new(&pattern) {
        Ok(glob) => glob.matches(&host),
        Err(_) => pattern == host,
    }
}

/// whether every host the pattern `inner` allows is one `outer` allows too. Decided only where that is sure: `outer`
/// is `*`, the two are the same, or `inner` is a plain host (no `*`, `?` or `[`) that `outer` allows. Anything else
/// counts as not within, so a ceiling never lets through more than it allows
fn pattern_within(inner: &str, outer: &str) -> bool {
    let (inner, outer) = (inner.to_ascii_lowercase(), outer.to_ascii_lowercase());
    outer == "*" || inner == outer || (!inner.contains(['*', '?', '[']) && host_allowed(&outer, &inner))
}

/// A capability a `plugin` directive asks for that the operator's [`PluginPolicy`] does not allow in full: the
/// plugin runs with `granted` instead of `requested`, and the load reports it as a
/// [`ErrorKind::PluginCapabilityReduced`] on the directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReduced {
    /// the meta key of the capability, such as `allowed_hosts`
    pub capability: &'static str,
    /// what the directive asks for, as the meta writes it
    pub requested: String,
    /// what the plugin gets, `none` for nothing
    pub granted: String,
    /// the setting of `zhang serve` that would grant more, such as `ZHANG_PLUGIN_ALLOWED_HOSTS`
    pub setting: &'static str,
}

impl CapabilityReduced {
    /// the metas of the error reporting it on the directive of the plugin `module`
    pub fn metas(&self, module: &str) -> HashMap<String, String> {
        [
            ("plugin", module),
            ("capability", self.capability),
            ("requested", &self.requested),
            ("granted", &self.granted),
            ("setting", self.setting),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }
}

/// a list as a [`CapabilityReduced`] shows it: its items separated by `, `, or `none`
fn listed(items: &[String]) -> String {
    match items.is_empty() {
        true => "none".to_owned(),
        false => items.join(", "),
    }
}

impl PluginCapabilities {
    /// Bound these capabilities, as a directive grants them, by the operator's `policy`, and say what they lost.
    ///
    /// - `allowed_hosts` keeps the patterns within one of the policy's, and the policy's patterns within one of the
    ///   directive's (so `*` with a policy of `api.example.com` gives `api.example.com`). A pattern only partly within
    ///   the policy is left out, so the plugin may reach fewer hosts than both allow, never more.
    /// - `timeout` is at most [`PluginPolicy::max_timeout`]; the default, which a directive did not write
    ///   (`timeout_declared` false), is lowered without a word.
    ///
    /// A capability to write to the ledger, once plugins have one, is bounded here too: cleared unless
    /// [`PluginPolicy::allows_writes`], and reported as a [`CapabilityReduced`] naming
    /// [`LEVEL_ENV`](crate::plugin_policy::LEVEL_ENV).
    pub fn bound_by(&mut self, policy: &PluginPolicy, timeout_declared: bool) -> Vec<CapabilityReduced> {
        let mut reduced = vec![];
        let mut hosts: Vec<String> = vec![];
        for granted in &self.allowed_hosts {
            if policy.allowed_hosts.iter().any(|ceiling| pattern_within(granted, ceiling)) {
                hosts.push(granted.to_ascii_lowercase());
            }
            for ceiling in &policy.allowed_hosts {
                if pattern_within(ceiling, granted) {
                    hosts.push(ceiling.clone());
                }
            }
        }
        let mut seen = HashSet::new();
        hosts.retain(|host| seen.insert(host.clone()));
        let fully_granted = self
            .allowed_hosts
            .iter()
            .all(|granted| policy.allowed_hosts.iter().any(|ceiling| pattern_within(granted, ceiling)));
        if !fully_granted {
            reduced.push(CapabilityReduced {
                capability: ALLOWED_HOSTS_KEY,
                requested: listed(&self.allowed_hosts),
                granted: listed(&hosts),
                setting: ALLOWED_HOSTS_ENV,
            });
        }
        self.allowed_hosts = hosts;

        if self.timeout > policy.max_timeout {
            if timeout_declared {
                reduced.push(CapabilityReduced {
                    capability: TIMEOUT_KEY,
                    requested: format!("{:?}", self.timeout),
                    granted: format!("{:?}", policy.max_timeout),
                    setting: MAX_TIMEOUT_ENV,
                });
            }
            self.timeout = policy.max_timeout;
        }
        reduced
    }
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;
    use std::time::Duration;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Meta, Plugin, ZhangString};

    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::{
        host_allowed, pattern_within, plugin_seed, CapabilityReduced, DeclarationError, PluginCapabilities, PluginDeclaration, PluginDirectiveConfig,
        PluginStage, WriteKind, DEFAULT_TIMEOUT,
    };
    use crate::plugin_policy::PluginPolicy;
    use crate::utils::hashmap::HashMapOfExt;

    #[test]
    fn should_match_hosts_as_extism_always_did_in_any_case() {
        assert!(host_allowed("api.example.com", "api.example.com"));
        assert!(host_allowed("API.Example.com", "api.example.com"));
        assert!(host_allowed("*.example.com", "api.example.com"));
        assert!(host_allowed("*", "anything.example"));
        assert!(!host_allowed("*.example.com", "example.com"));
        assert!(!host_allowed("api.example.com", "api.example.com.evil.example"));
        assert!(host_allowed("[", "["), "a pattern that is no glob matches as written");
    }

    #[test]
    fn should_count_a_pattern_within_another_only_when_that_is_sure() {
        for (inner, outer) in [
            ("api.example.com", "*"),
            ("*", "*"),
            ("*.example.com", "*.example.com"),
            ("api.example.com", "*.example.com"),
        ] {
            assert!(pattern_within(inner, outer), "{inner} within {outer}");
        }
        for (inner, outer) in [
            ("*", "api.example.com"),
            ("*.example.com", "*.api.example.com"),
            ("*.example.com", "?.example.com"),
            ("a.com", "b.com"),
        ] {
            assert!(!pattern_within(inner, outer), "{inner} not within {outer}");
        }
    }

    #[test]
    fn should_bound_capabilities_by_the_policy_and_say_what_they_lost() {
        let mut capabilities = PluginCapabilities {
            allowed_hosts: vec!["api.example.com".to_owned(), "*".to_owned()],
            timeout: Duration::from_secs(600),
            ..PluginCapabilities::default()
        };
        let policy = PluginPolicy {
            allowed_hosts: vec!["api.example.com".to_owned(), "rates.example".to_owned()],
            ..PluginPolicy::default()
        };

        let reduced = capabilities.bound_by(&policy, true);

        assert_eq!(capabilities.allowed_hosts, vec!["api.example.com", "rates.example"]);
        assert_eq!(capabilities.timeout, Duration::from_secs(30));
        assert_eq!(
            reduced,
            vec![
                CapabilityReduced {
                    capability: "allowed_hosts",
                    requested: "api.example.com, *".to_owned(),
                    granted: "api.example.com, rates.example".to_owned(),
                    setting: "ZHANG_PLUGIN_ALLOWED_HOSTS",
                },
                CapabilityReduced {
                    capability: "timeout",
                    requested: "600s".to_owned(),
                    granted: "30s".to_owned(),
                    setting: "ZHANG_PLUGIN_MAX_TIMEOUT",
                },
            ]
        );
        assert_eq!(
            reduced[0].metas("fx.wasm"),
            HashMap::from(
                [
                    ("plugin", "fx.wasm"),
                    ("capability", "allowed_hosts"),
                    ("requested", "api.example.com, *"),
                    ("granted", "api.example.com, rates.example"),
                    ("setting", "ZHANG_PLUGIN_ALLOWED_HOSTS"),
                ]
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
            )
        );
    }

    #[test]
    fn should_leave_capabilities_within_the_policy_alone() {
        let granted = PluginCapabilities {
            allowed_hosts: vec!["api.example.com".to_owned()],
            timeout: Duration::from_secs(10),
            ..PluginCapabilities::default()
        };
        let mut capabilities = granted.clone();

        assert_eq!(capabilities.bound_by(&PluginPolicy::unrestricted(), true), vec![]);
        assert_eq!(capabilities, granted);
        // the default timeout is lowered silently when the directive did not write it
        let mut capabilities = PluginCapabilities::default();
        assert_eq!(capabilities.bound_by(&PluginPolicy::default(), false), vec![]);
        assert_eq!(capabilities.timeout, Duration::from_secs(30));
    }

    #[test]
    fn should_know_whether_a_directive_sets_the_timeout() {
        assert!(PluginDeclaration::parse(&directive(&[("timeout", "2m")])).declares_timeout());
        assert!(!PluginDeclaration::parse(&directive(&[])).declares_timeout());
        assert!(
            !PluginDeclaration::parse(&directive(&[("timeout", "soon")])).declares_timeout(),
            "an invalid one counts as none"
        );
    }

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
    fn should_run_a_plugin_without_stage_booked() {
        let declaration = PluginDeclaration::parse(&directive(&[("base_currency", "USD")]));

        assert_eq!(declaration.capabilities.stage, PluginStage::Booked);
        assert_eq!(declaration.errors, vec![]);
    }

    #[test]
    fn should_read_the_stage_and_still_pass_it_through() {
        for (value, stage) in [("raw", PluginStage::Raw), (" raw ", PluginStage::Raw), ("booked", PluginStage::Booked)] {
            let declaration = PluginDeclaration::parse(&directive(&[("stage", value)]));

            assert_eq!(declaration.capabilities.stage, stage, "stage {value:?}");
            assert_eq!(declaration.errors, vec![], "stage {value:?}");
            assert_eq!(declaration.config, map(&[("stage", value)]), "stage {value:?}");
        }
        // the last value counts
        let declaration = PluginDeclaration::parse(&directive(&[("stage", "raw"), ("stage", "booked")]));
        assert_eq!(declaration.capabilities.stage, PluginStage::Booked);
    }

    #[test]
    fn should_report_an_invalid_stage_and_run_the_plugin_booked() {
        for value in ["RAW", "early", "", "before booking"] {
            let declaration = PluginDeclaration::parse(&directive(&[("stage", value)]));

            assert_eq!(declaration.capabilities.stage, PluginStage::Booked, "stage {value:?}");
            assert_eq!(
                declaration.errors,
                vec![DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", "fx-rate.wasm", "stage", value),
                }],
                "stage {value:?}"
            );
            assert_eq!(declaration.config, map(&[("stage", value)]), "stage {value:?}");
        }
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
    fn should_read_allowed_paths_and_still_pass_them_through() {
        let declaration = PluginDeclaration::parse(&directive(&[
            ("allowed_paths", "documents"),
            ("allowed_paths", "./statements//2024.csv"),
            ("allowed_paths", "."),
        ]));

        assert_eq!(
            declaration.capabilities.allowed_paths,
            vec![PathBuf::from("documents"), PathBuf::from("statements/2024.csv"), PathBuf::new()]
        );
        assert_eq!(declaration.errors, vec![]);
        // like any other key: the flat config keeps the last value, `zhang.plugin` every value
        assert_eq!(declaration.config, map(&[("allowed_paths", ".")]));
        assert_eq!(declaration.directive.meta["allowed_paths"], vec!["documents", "./statements//2024.csv", "."]);
    }

    #[test]
    fn should_read_allowed_writes_once_each_and_still_pass_them_through() {
        let declaration = PluginDeclaration::parse(&directive(&[
            ("allowed_writes", "open"),
            ("allowed_writes", " transaction "),
            ("allowed_writes", "open"),
        ]));

        assert_eq!(declaration.capabilities.allowed_writes, vec![WriteKind::Open, WriteKind::Transaction]);
        assert_eq!(declaration.errors, vec![]);
        assert_eq!(declaration.config, map(&[("allowed_writes", "open")]));
        assert_eq!(declaration.directive.meta["allowed_writes"], vec!["open", " transaction ", "open"]);
    }

    #[test]
    fn should_grant_no_writes_without_allowed_writes() {
        let declaration = PluginDeclaration::parse(&directive(&[("allowed_hosts", "api.example.com")]));

        assert_eq!(declaration.capabilities.allowed_writes, Vec::<WriteKind>::new());
    }

    #[test]
    fn should_report_an_invalid_allowed_write_and_grant_nothing_for_it() {
        for value in ["balance", "Transaction", "", "transaction open", "close", "*"] {
            let declaration = PluginDeclaration::parse(&directive(&[("allowed_writes", value), ("allowed_writes", "open")]));

            assert_eq!(declaration.capabilities.allowed_writes, vec![WriteKind::Open], "value {value:?}");
            assert_eq!(
                declaration.errors,
                vec![DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", "fx-rate.wasm", "allowed_writes", value),
                }],
                "value {value:?}"
            );
        }
    }

    #[test]
    fn should_grant_no_file_access_without_allowed_paths() {
        let declaration = PluginDeclaration::parse(&directive(&[("allowed_hosts", "api.example.com")]));

        assert_eq!(declaration.capabilities.allowed_paths, Vec::<PathBuf>::new());
    }

    #[test]
    fn should_report_an_invalid_allowed_path_and_grant_nothing_for_it() {
        for value in ["../outside", "documents/../..", "/etc", "", "documents\\2024", "documents\0"] {
            let declaration = PluginDeclaration::parse(&directive(&[("allowed_paths", value), ("allowed_paths", "documents")]));

            assert_eq!(declaration.capabilities.allowed_paths, vec![PathBuf::from("documents")], "value {value:?}");
            assert_eq!(
                declaration.errors,
                vec![DeclarationError {
                    kind: ErrorKind::ParseInvalidMeta,
                    metas: HashMap::of2("plugin", "fx-rate.wasm", "allowed_paths", value),
                }],
                "value {value:?}"
            );
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
            declaration.config_with(&[], declaration.host_config(0)),
            map(&[
                ("base_currency", "USD"),
                ("tag", "second"),
                ("zhang.abi", "1"),
                (
                    "zhang.plugin",
                    r#"{"module":"fx-rate.wasm","args":["USD","strict \"mode\""],"meta":{"allowed_hosts":["b.example","a.example"],"base_currency":["USD"],"tag":["first","second"]}}"#
                ),
                ("zhang.seed", "1811957226761548848"),
                ("zhang.allowed_hosts", r#"["b.example","a.example"]"#),
                ("zhang.timeout_ms", "60000"),
            ])
        );
    }

    #[test]
    fn should_pass_empty_args_and_meta_for_a_bare_directive() {
        let declaration = PluginDeclaration::parse(&directive(&[]));

        assert_eq!(
            declaration.config_with(&[], declaration.host_config(0)),
            map(&[
                ("zhang.abi", "1"),
                ("zhang.plugin", r#"{"module":"fx-rate.wasm","args":[],"meta":{}}"#),
                ("zhang.seed", "1811957226761548848"),
                ("zhang.allowed_hosts", "[]"),
                ("zhang.timeout_ms", "60000"),
            ])
        );
    }

    #[test]
    fn should_hand_the_plugin_the_capabilities_the_policy_left_it() {
        let mut declaration = PluginDeclaration::parse(&directive(&[("allowed_hosts", "*"), ("timeout", "2m")]));
        let policy = PluginPolicy {
            allowed_hosts: vec!["api.example.com".to_owned()],
            ..PluginPolicy::default()
        };
        declaration.capabilities.bound_by(&policy, declaration.declares_timeout());

        let config = declaration.config_with(&[], declaration.host_config(0));

        assert_eq!(config["zhang.allowed_hosts"], r#"["api.example.com"]"#);
        assert_eq!(config["zhang.timeout_ms"], "30000");
        // what the directive asked for stays where it always was
        assert_eq!(config["timeout"], "2m");
        assert!(config["zhang.plugin"].contains(r#""allowed_hosts":["*"]"#), "{}", config["zhang.plugin"]);
    }

    #[test]
    fn should_let_the_host_values_win_over_meta_and_options_named_like_them() {
        let declaration = PluginDeclaration::parse(&directive(&[("zhang.plugin", "from meta"), ("zhang.abi", "0"), ("zhang.seed", "7")]));
        let options = [option("zhang.plugin", "from option"), option("zhang.seed", "8")];

        assert_eq!(
            declaration.config_with(&options, declaration.host_config(0)),
            map(&[
                ("zhang.abi", "1"),
                (
                    "zhang.plugin",
                    r#"{"module":"fx-rate.wasm","args":[],"meta":{"zhang.abi":["0"],"zhang.plugin":["from meta"],"zhang.seed":["7"]}}"#
                ),
                ("zhang.seed", "1811957226761548848"),
                ("zhang.allowed_hosts", "[]"),
                ("zhang.timeout_ms", "60000"),
            ])
        );
    }

    /// the `zhang.seed` the host hands a plugin declared by `directive`, the `occurrence`-th of its module
    fn seed_config(directive: &Plugin, occurrence: usize) -> String {
        let declaration = PluginDeclaration::parse(directive);
        declaration.config_with(&[], declaration.host_config(occurrence))["zhang.seed"].clone()
    }

    #[test]
    fn should_derive_the_seed_as_specified() {
        // computed independently: the first 8 bytes of SHA-256("zhang/plugin-seed/v1" ‖ len ‖ module ‖ occurrence
        // [‖ len ‖ seed]), big-endian. Changing a value here changes every seed users already rely on
        assert_eq!(plugin_seed("fx-rate.wasm", 0, None), 1811957226761548848);
        assert_eq!(plugin_seed("fx-rate.wasm", 1, None), 14470161454731210553);
        assert_eq!(plugin_seed("fx-rate.wasm", 0, Some("ids")), 3501562816358462189);
        assert_eq!(plugin_seed("fx-rate.wasm", 0, Some("")), 358401822436118058, "an empty seed meta still counts");
    }

    #[test]
    fn should_hand_the_same_seed_to_every_load() {
        assert_eq!(seed_config(&directive(&[]), 0), seed_config(&directive(&[]), 0));
        assert_eq!(seed_config(&directive(&[]), 0), "1811957226761548848");
        // the plugin's other config, args and options do not move it
        let declaration = PluginDeclaration::parse(&directive_with_args(&["USD"], &[("base_currency", "USD")]));
        assert_eq!(
            declaration.config_with(&[option("timezone", "Asia/Shanghai")], declaration.host_config(0))["zhang.seed"],
            "1811957226761548848"
        );
    }

    #[test]
    fn should_give_each_declaration_of_a_module_its_own_seed() {
        assert_ne!(seed_config(&directive(&[]), 0), seed_config(&directive(&[]), 1));
        assert_eq!(seed_config(&directive(&[]), 1), "14470161454731210553");
    }

    #[test]
    fn should_mix_the_seed_meta_into_the_seed_and_still_pass_it_through() {
        let declaration = PluginDeclaration::parse(&directive(&[("seed", "first"), ("seed", "ids")]));

        assert_eq!(declaration.capabilities.seed.as_deref(), Some("ids"), "the last value wins");
        let config = declaration.config_with(&[], declaration.host_config(0));
        assert_eq!(config["seed"], "ids");
        assert_eq!(config["zhang.seed"], "3501562816358462189");
        assert_ne!(seed_config(&directive(&[("seed", "ids")]), 0), seed_config(&directive(&[("seed", "other")]), 0));
    }
}
