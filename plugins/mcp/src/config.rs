//! The plugin's settings, from the meta of its `plugin` directive:
//!
//! ```zhang
//! plugin "plugins/zhang-mcp.wasm"
//!   timeout: "20s"                                  ; the host's: how long one request may run
//!   allowed_origins: "https://ledger.example.com"   ; comma-separated, or the key repeated; absent = same host only
//!   max_rows: "500"                                 ; the cap of every tool's `limit`; default 1000
//! ```

use zhang_plugin_sdk::config::Config;

/// the `limit` a tool runs with when the call gives none
pub const DEFAULT_LIMIT: usize = 200;
/// the cap of `limit` when the directive sets no `max_rows`
pub const DEFAULT_MAX_ROWS: usize = 1000;

/// the name the plugin exports, and so the route it serves: `/api/plugins/zhang-mcp`
pub const PLUGIN_NAME: &str = "zhang-mcp";
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    /// the full origins (`scheme://host[:port]`) a browser client may send, lower case, besides the request's own host
    pub allowed_origins: Vec<String>,
    /// the most rows any tool returns
    pub max_rows: usize,
}

impl Default for McpConfig {
    fn default() -> McpConfig {
        McpConfig {
            allowed_origins: vec![],
            max_rows: DEFAULT_MAX_ROWS,
        }
    }
}

impl McpConfig {
    /// the settings of the plugin's directive; a value zhang cannot use falls back to the default
    pub fn from_sdk(config: &Config) -> McpConfig {
        let values = |key: &str| {
            config
                .meta(key)
                .or_else(|| config.get(key))
                .map(|values| values.iter().map(str::to_owned).collect::<Vec<_>>())
        };
        let allowed_origins = values("allowed_origins")
            .unwrap_or_default()
            .iter()
            .flat_map(|value| value.split(','))
            .filter_map(normalize_origin)
            .collect();
        let max_rows = values("max_rows")
            .and_then(|values| values.last().cloned())
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|rows| *rows > 0)
            .unwrap_or(DEFAULT_MAX_ROWS);
        McpConfig { allowed_origins, max_rows }
    }

    /// `limit` as a tool runs with it: the default when absent, never above `max_rows`
    pub fn limit(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(DEFAULT_LIMIT).min(self.max_rows)
    }
}

/// an origin as it is compared: trimmed, without a trailing slash, lower case; `None` when empty
pub fn normalize_origin(origin: &str) -> Option<String> {
    let origin = origin.trim().trim_end_matches('/').to_ascii_lowercase();
    (!origin.is_empty()).then_some(origin)
}

#[cfg(test)]
mod test {
    use zhang_plugin_sdk::config::Config;

    use super::{McpConfig, DEFAULT_MAX_ROWS};

    fn config(meta: &[(&str, &[&str])]) -> Config {
        let meta: serde_json::Map<String, serde_json::Value> = meta.iter().map(|(key, values)| ((*key).to_owned(), serde_json::json!(values))).collect();
        let plugin = serde_json::json!({"module": "plugins/zhang-mcp.wasm", "args": [], "meta": meta}).to_string();
        Config::from_map([("zhang.abi", "1".to_owned()), ("zhang.plugin", plugin)])
    }

    #[test]
    fn reads_origins_and_row_caps_from_the_directive() {
        let config = McpConfig::from_sdk(&config(&[
            ("allowed_origins", &["https://Ledger.example.com/, http://localhost:5173", "https://b.example"]),
            ("max_rows", &["250"]),
        ]));
        assert_eq!(
            config.allowed_origins,
            vec!["https://ledger.example.com", "http://localhost:5173", "https://b.example"]
        );
        assert_eq!(config.max_rows, 250);
        assert_eq!((config.limit(None), config.limit(Some(10)), config.limit(Some(9999))), (200, 10, 250));
    }

    #[test]
    fn falls_back_to_the_defaults() {
        assert_eq!(McpConfig::from_sdk(&config(&[])), McpConfig::default());
        let bad = McpConfig::from_sdk(&config(&[("max_rows", &["soon"]), ("allowed_origins", &[" , "])]));
        assert_eq!(bad.max_rows, DEFAULT_MAX_ROWS);
        assert!(bad.allowed_origins.is_empty());
        assert_eq!(McpConfig::from_sdk(&config(&[("max_rows", &["0"])])).max_rows, DEFAULT_MAX_ROWS);
        // a native build has no directive at all
        assert_eq!(McpConfig::from_sdk(&Config::load()), McpConfig::default());
    }
}
