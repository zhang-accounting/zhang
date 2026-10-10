//! The API contracts of zhang-server, one module per area: the handlers called with the `State` of a loaded ledger
//! and, where only the router shows the behaviour (authentication, path encoding, error bodies, paging), the router
//! the server runs. The harness is `zhang_testkit::http`.
//!
//! Three suites keep a binary of their own because they change process-wide state: `account_journal_limit` (an
//! environment variable read once per process), `document_cache` (the working directory) and `sdk_plugins` (it
//! builds the example plugins with cargo).

mod account_totals;
mod auth;
mod balance_assertions;
mod base64_paths;
mod day_order;
mod error_bodies;
mod error_lines;
mod journals_engine;
mod open_commodities;
mod paging_contract;
mod plain_numbers;
mod posting_metadata;
mod query_zhang_tables;
mod string_escaping;
mod transaction_preview;
mod wall_clock_times;
