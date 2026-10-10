//! The integration tests of the beancount extension as one binary: every module under `tests/beancount/` is a
//! suite, and nextest still runs their tests in parallel. One suite runs with
//! `cargo nextest run -p beancount -E 'binary(beancount) & test(balance_assertions::)'`. The oracle fixtures stay in
//! `tests/<suite>/` next to their `oracle.json` and `generate.py`.

mod active_accounts;
mod balance_assertions;
mod beancount_compat;
mod budget_syntax;
mod conformance;
mod cost_specs;
mod dialect;
mod division_by_zero;
mod meta_values;
mod open_commodities;
mod posting_flags;
mod posting_meta;
mod posting_metadata;
mod string_escaping;
mod string_round_trip;
