//! What the operator of a zhang server lets plugins use, whatever a ledger's `plugin` directives ask for.
//!
//! A ledger is text that more people (and tools) can change than the operator who starts the server: a `PUT
//! /api/files`, a git or S3 data source. A `plugin` directive grants capabilities (`allowed_hosts`, `timeout`, and
//! on the way, writes), so whoever can edit the ledger could grant a plugin the network or hours of CPU. The
//! [`PluginPolicy`] is the operator's ceiling over those grants: it is set where the server starts (environment
//! variables and flags, see `zhang serve --help`) and bounds every plugin of every ledger the server loads. A plugin
//! gets what its directive grants *and* the policy allows; where the directive asks for more, the plugin still loads,
//! with less, and the load reports a `PluginCapabilityReduced` error naming the setting to change. The policy travels
//! with the ledger: a reload uses the policy of the load before it.
//!
//! By default plugins run read-only, without network and for at most 30 seconds a call.

use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::time::Duration;

/// a kibibyte, in bytes
pub const KIB: u64 = 1024;
/// a mebibyte, in bytes
pub const MIB: u64 = 1024 * KIB;
/// a gibibyte, in bytes
pub const GIB: u64 = 1024 * MIB;

/// the size of a WebAssembly memory page, the unit a plugin's memory grows by
pub const WASM_PAGE: u64 = 64 * KIB;

/// The most memory one instance of a plugin may use when the operator sets no limit: every linear memory of the
/// instance together, extism's own (which holds the input, the output and what host functions hand over) and the
/// plugin's. A processor receives the whole ledger as JSON and holds it a few times over while it works, so this
/// leaves room for a ledger of some tens of thousands of transactions; a router, which reads the ledger through
/// queries, needs far less. Without a limit, a wasm32 plugin can take 4 GiB per memory, for each request it serves.
pub const DEFAULT_MAX_MEMORY: u64 = 256 * MIB;

/// the environment variable setting [`PluginPolicy::max_memory`]
pub const MAX_MEMORY_ENV: &str = "ZHANG_PLUGIN_MAX_MEMORY";

/// the environment variable setting [`PluginPolicy::level`]; `zhang serve --plugins` wins over it
pub const LEVEL_ENV: &str = "ZHANG_PLUGINS";

/// the environment variable setting [`PluginPolicy::allowed_hosts`], host patterns separated by commas
pub const ALLOWED_HOSTS_ENV: &str = "ZHANG_PLUGIN_ALLOWED_HOSTS";

/// the environment variable setting [`PluginPolicy::max_timeout`]
pub const MAX_TIMEOUT_ENV: &str = "ZHANG_PLUGIN_MAX_TIMEOUT";

/// how long one call into a plugin may run when the operator sets no limit. It bounds router calls too, which hold
/// the ledger's read lock while they run: while a writer waits for that lock no new reader gets it, so a long call
/// stalls every request of the server
pub const DEFAULT_MAX_TIMEOUT: Duration = Duration::from_secs(30);

/// the longest timeout anything may set: a `timeout` meta, or [`MAX_TIMEOUT_ENV`]
pub const LONGEST_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// How much the operator lets plugins do, [`LEVEL_ENV`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PluginLevel {
    /// no plugin is loaded: a ledger's `plugin` directives each report a `PluginsDisabled` error
    Off,
    /// plugins run, and none changes the ledger's files, whatever its directive grants
    #[default]
    Read,
    /// plugins run, and a plugin may write to the ledger where its directive grants it
    Write,
}

impl FromStr for PluginLevel {
    type Err = String;

    /// `off`, `read` or `write`, in any case
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" => Ok(PluginLevel::Off),
            "read" => Ok(PluginLevel::Read),
            "write" => Ok(PluginLevel::Write),
            _ => Err(format!("{text:?} is not a plugin level: write off, read or write")),
        }
    }
}

impl Display for PluginLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PluginLevel::Off => "off",
            PluginLevel::Read => "read",
            PluginLevel::Write => "write",
        })
    }
}

/// What the operator lets every plugin use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPolicy {
    /// whether plugins load at all, and whether they may write to the ledger
    pub level: PluginLevel,
    /// The hosts a plugin may reach at most, as patterns of the `allowed_hosts` meta (`*` matches any run of
    /// characters, so a lone `*` allows every host), lower case. A plugin reaches a host only when both its
    /// `allowed_hosts` and these allow it. Empty, the default, means no plugin has any network. Whatever they allow,
    /// a request never goes to a private, loopback, link-local or otherwise non-public address
    pub allowed_hosts: Vec<String>,
    /// the longest one call into a plugin may run, whatever its `timeout` meta asks for; a shorter `timeout` counts
    pub max_timeout: Duration,
    /// the most memory, in bytes, one instance of a plugin may use; an allocation beyond it fails the call, as an
    /// out-of-memory trap. Rounded down to whole WebAssembly pages of 64 KiB
    pub max_memory: u64,
}

impl Default for PluginPolicy {
    /// read-only, no network, 30 seconds and 256 MiB
    fn default() -> Self {
        PluginPolicy {
            level: PluginLevel::default(),
            allowed_hosts: vec![],
            max_timeout: DEFAULT_MAX_TIMEOUT,
            max_memory: DEFAULT_MAX_MEMORY,
        }
    }
}

impl PluginPolicy {
    /// A policy that holds back nothing a `plugin` directive can grant: writes, every host (public addresses only,
    /// as always), the longest timeout. For tools and tests that trust the ledger as much as the code running them;
    /// a server never uses it unless its operator sets all of it.
    pub fn unrestricted() -> Self {
        PluginPolicy {
            level: PluginLevel::Write,
            allowed_hosts: vec!["*".to_owned()],
            max_timeout: LONGEST_TIMEOUT,
            max_memory: DEFAULT_MAX_MEMORY,
        }
    }

    /// whether plugins load at all (not [`PluginLevel::Off`])
    pub fn loads_plugins(&self) -> bool {
        self.level != PluginLevel::Off
    }

    /// Whether the operator lets plugins write to the ledger ([`PluginLevel::Write`]). This is the one check every
    /// plugin write is to go through: a plugin writes only when this is true *and* its directive grants the write. A
    /// directive granting writes under a policy that does not allow them is to load its plugin without them, and say
    /// so (`PluginCapabilities::bound_by`, where the other capabilities are bounded).
    pub fn allows_writes(&self) -> bool {
        self.level == PluginLevel::Write
    }

    /// [`PluginPolicy::max_memory`] in WebAssembly pages, as extism's manifest takes it: at least one page
    pub fn max_memory_pages(&self) -> u32 {
        u32::try_from(self.max_memory / WASM_PAGE).unwrap_or(u32::MAX).max(1)
    }

    /// the most memory one instance gets, in bytes: [`PluginPolicy::max_memory_pages`] whole pages, which a plugin reads
    /// in its config key `zhang.max_memory`
    pub fn instance_memory(&self) -> u64 {
        u64::from(self.max_memory_pages()) * WASM_PAGE
    }
}

impl Display for PluginPolicy {
    /// the policy as the server logs it when it starts
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let hosts = match self.allowed_hosts.is_empty() {
            true => "no host".to_owned(),
            false => self.allowed_hosts.join(", "),
        };
        write!(
            f,
            "plugins {} ({LEVEL_ENV}), may reach {hosts} ({ALLOWED_HOSTS_ENV}), run for at most {:?} a call ({MAX_TIMEOUT_ENV}) and use at most {} of memory per instance ({MAX_MEMORY_ENV})",
            match self.level {
                PluginLevel::Off => "are off",
                PluginLevel::Read => "run read-only",
                PluginLevel::Write => "run and may write where their directive allows it",
            },
            self.max_timeout,
            Size(self.max_memory)
        )
    }
}

/// The host patterns of [`ALLOWED_HOSTS_ENV`]: separated by commas, each trimmed and lower-cased, empty ones left out
pub fn parse_hosts(text: &str) -> Vec<String> {
    text.split(',').map(|it| it.trim().to_ascii_lowercase()).filter(|it| !it.is_empty()).collect()
}

/// A timeout, as a `timeout` meta and [`MAX_TIMEOUT_ENV`] write it: whole seconds, or a whole number with a unit `ms`,
/// `s`, `m` or `h`. `None` for anything else, for zero and for more than [`LONGEST_TIMEOUT`]
pub fn parse_timeout(value: &str) -> Option<Duration> {
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
    (!timeout.is_zero() && timeout <= LONGEST_TIMEOUT).then_some(timeout)
}

/// a size in bytes, written with the largest binary unit it is a whole number of: `256 MiB`, `1536 KiB`, `10 B`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size(pub u64);

impl Display for Size {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let bytes = self.0;
        match [(GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")]
            .into_iter()
            .find(|(unit, _)| bytes > 0 && bytes.is_multiple_of(*unit))
        {
            Some((unit, name)) => write!(f, "{} {name}", bytes / unit),
            None => write!(f, "{bytes} B"),
        }
    }
}

/// A size as an operator writes it: a whole number of bytes (`4194304`), or a whole number with a binary unit `KiB`,
/// `MiB` or `GiB` (`4MiB`, `4 MiB`; also `K`, `M`, `G`, `KB`, `MB` and `GB`, which mean the same binary units, in any
/// case). Zero, a fraction and anything else is an error saying what is expected.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let trimmed = text.trim();
    let digits = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());
    let (amount, unit) = trimmed.split_at(digits);
    let invalid = || format!("{text:?} is not a size: write a whole number of bytes, or one with a unit KiB, MiB or GiB, such as 4MiB");
    let amount: u64 = amount.parse().map_err(|_| invalid())?;
    let unit = match unit.trim_start().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => KIB,
        "m" | "mb" | "mib" => MIB,
        "g" | "gb" | "gib" => GIB,
        _ => return Err(invalid()),
    };
    match amount.checked_mul(unit) {
        Some(0) => Err(format!("{text:?} is not a size: it must be above zero")),
        Some(size) => Ok(size),
        None => Err(invalid()),
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::{parse_hosts, parse_size, parse_timeout, PluginLevel, PluginPolicy, Size, DEFAULT_MAX_MEMORY, GIB, KIB, MIB};

    #[test]
    fn should_run_plugins_read_only_without_network_for_thirty_seconds_by_default() {
        let policy = PluginPolicy::default();
        assert_eq!(policy.level, PluginLevel::Read);
        assert!(policy.loads_plugins());
        assert!(!policy.allows_writes());
        assert_eq!(policy.allowed_hosts, Vec::<String>::new());
        assert_eq!(policy.max_timeout, Duration::from_secs(30));
    }

    #[test]
    fn should_read_the_level_in_any_case() {
        assert_eq!("off".parse(), Ok(PluginLevel::Off));
        assert_eq!(" Read ".parse(), Ok(PluginLevel::Read));
        assert_eq!("WRITE".parse(), Ok(PluginLevel::Write));
        assert_eq!(
            "sometimes".parse::<PluginLevel>(),
            Err("\"sometimes\" is not a plugin level: write off, read or write".to_owned())
        );
        for (level, loads, writes) in [
            (PluginLevel::Off, false, false),
            (PluginLevel::Read, true, false),
            (PluginLevel::Write, true, true),
        ] {
            let policy = PluginPolicy {
                level,
                ..PluginPolicy::default()
            };
            assert_eq!((policy.loads_plugins(), policy.allows_writes()), (loads, writes), "{level}");
        }
    }

    #[test]
    fn should_read_host_patterns_separated_by_commas() {
        assert_eq!(
            parse_hosts("api.example.com, *.Rates.example ,,*"),
            vec!["api.example.com", "*.rates.example", "*"]
        );
        assert_eq!(parse_hosts(" , "), Vec::<String>::new());
    }

    #[test]
    fn should_read_a_timeout_as_the_meta_writes_it() {
        assert_eq!(parse_timeout("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_timeout("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_timeout("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_timeout("24h"), Some(Duration::from_secs(24 * 3600)));
        for value in ["", "0", "25h", "soon", "1.5s"] {
            assert_eq!(parse_timeout(value), None, "{value:?}");
        }
    }

    #[test]
    fn should_read_a_size_in_bytes_or_binary_units() {
        for (text, size) in [
            ("4194304", 4 * MIB),
            ("4MiB", 4 * MIB),
            (" 4 MiB ", 4 * MIB),
            ("4m", 4 * MIB),
            ("4MB", 4 * MIB),
            ("512KiB", 512 * KIB),
            ("512k", 512 * KIB),
            ("1GiB", GIB),
            ("1g", GIB),
            ("10B", 10),
        ] {
            assert_eq!(parse_size(text), Ok(size), "{text:?}");
        }
    }

    #[test]
    fn should_refuse_what_is_not_a_size() {
        for text in ["", "MiB", "0", "0MiB", "-1", "1.5MiB", "4 TiB", "four", "18446744073709551615GiB"] {
            assert!(parse_size(text).is_err(), "{text:?}");
        }
        assert_eq!(parse_size("0").unwrap_err(), "\"0\" is not a size: it must be above zero");
    }

    #[test]
    fn should_write_a_size_with_the_largest_whole_unit() {
        assert_eq!(Size(256 * MIB).to_string(), "256 MiB");
        assert_eq!(Size(1536 * KIB).to_string(), "1536 KiB");
        assert_eq!(Size(2 * GIB).to_string(), "2 GiB");
        assert_eq!(Size(10).to_string(), "10 B");
        assert_eq!(Size(0).to_string(), "0 B");
    }

    #[test]
    fn should_cap_memory_at_256_mib_by_default_in_whole_pages() {
        assert_eq!(PluginPolicy::default().max_memory, DEFAULT_MAX_MEMORY);
        assert_eq!(PluginPolicy::default().max_memory_pages(), 4096);
        let policy = |max_memory| PluginPolicy {
            max_memory,
            ..PluginPolicy::default()
        };
        assert_eq!(policy(64 * KIB + 1).max_memory_pages(), 1, "rounded down");
        assert_eq!(policy(1).max_memory_pages(), 1, "at least one page");
        assert_eq!(policy(u64::MAX).max_memory_pages(), u32::MAX);
        assert_eq!(PluginPolicy::default().instance_memory(), DEFAULT_MAX_MEMORY);
        assert_eq!(policy(100 * KIB).instance_memory(), 64 * KIB, "whole pages");
        assert_eq!(policy(1).instance_memory(), 64 * KIB);
    }
}
