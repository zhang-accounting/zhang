//! Portable logic shared by zhang's native and WASM crates.
//!
//! Ledger, query-engine and plugin-host integration stays with the callers. The plugin ABI's wire format lives here,
//! so that the plugin host and the plugin SDK share one definition of it.

pub mod decimal;
pub mod plugin_abi;
pub mod prices;
