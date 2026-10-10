//! The integration tests of zhang-core as one binary: every module under `tests/core/` is a suite, and nextest
//! still runs their tests in parallel. One suite runs with `cargo nextest run -p zhang-core -E 'binary(core) &
//! test(booking::)'`. The plugin host suite (`wasm_plugins`) needs the `plugin_runtime` feature, as before.

mod booking;
mod open_commodities;
mod operating_currency_precision;
mod posting_metadata;
mod read_cleanup;
mod string_escaping;
mod wasm_plugins;
