//! The behaviour tests of the query engine, one module per area: BQL semantics on inline ledgers
//! and on `integration-tests/<name>` by name, the data tables, the columns zhang adds, the
//! statements, the typed API and the limits. The beanquery comparisons are the `oracle` binary.
//!
//! Run one module with `cargo nextest run -p zhang-query -E 'binary(engine) & test(language::)'`.

mod booked_rows;
mod data_tables;
mod evaluator;
mod functions;
mod having_pivot;
mod inline_params;
mod language;
mod ledger_columns;
mod limits;
mod lots;
mod now;
mod period;
mod posting_flags;
mod posting_metadata;
mod server_features;
mod server_features_api;
mod statements;
mod tables;
mod typed_api;
mod zhang_tables;
