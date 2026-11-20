//! What the operator of a zhang server lets plugins use, whatever a ledger's `plugin` directives ask for.
//!
//! A ledger is text that more people (and tools) can change than the operator who starts the server: a `PUT
//! /api/files`, a git or S3 data source. The [`PluginPolicy`] is set where the server starts (environment variables
//! and flags, see `zhang serve --help`) and bounds every plugin of every ledger the server loads. It travels with the
//! ledger: a reload uses the policy of the load before it.

use std::fmt::{Display, Formatter};

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

/// What the operator lets every plugin use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPolicy {
    /// the most memory, in bytes, one instance of a plugin may use; an allocation beyond it fails the call, as an
    /// out-of-memory trap. Rounded down to whole WebAssembly pages of 64 KiB
    pub max_memory: u64,
}

impl Default for PluginPolicy {
    fn default() -> Self {
        PluginPolicy {
            max_memory: DEFAULT_MAX_MEMORY,
        }
    }
}

impl PluginPolicy {
    /// [`PluginPolicy::max_memory`] in WebAssembly pages, as extism's manifest takes it: at least one page
    pub fn max_memory_pages(&self) -> u32 {
        u32::try_from(self.max_memory / WASM_PAGE).unwrap_or(u32::MAX).max(1)
    }
}

impl Display for PluginPolicy {
    /// the policy as the server logs it when it starts
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "at most {} of memory per plugin instance ({MAX_MEMORY_ENV})", Size(self.max_memory))
    }
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
    use super::{parse_size, PluginPolicy, Size, DEFAULT_MAX_MEMORY, GIB, KIB, MIB};

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
        let policy = |max_memory| PluginPolicy { max_memory };
        assert_eq!(policy(64 * KIB + 1).max_memory_pages(), 1, "rounded down");
        assert_eq!(policy(1).max_memory_pages(), 1, "at least one page");
        assert_eq!(policy(u64::MAX).max_memory_pages(), u32::MAX);
    }
}
