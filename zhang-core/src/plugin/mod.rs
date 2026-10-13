//! The WASM plugin runtime.
//!
//! Plugins are written against the plugin ABI the host functions and config keys of these modules
//! define; the `zhang-plugin-sdk` crate wraps it for plugin authors. [`PluginType`] lives in
//! `zhang-ast`, so the host and the SDK share its serde representation.

pub use semver::Version;
pub use zhang_ast::PluginType;

pub mod capabilities;
pub mod files;
pub mod host;
pub mod http;
pub mod router;
pub mod runtime;
pub mod stage;
pub mod store;
