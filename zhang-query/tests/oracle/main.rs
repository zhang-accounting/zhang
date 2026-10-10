//! The beanquery oracle: every case set under `tests/oracle/cases/<set>` (written by
//! `tests/oracle/generate.py` from beanquery 0.2.0, see `tests/oracle/README.md`) compared with the
//! engine through `zhang_testkit::oracle`, one module per set.
//!
//! Run one set with `cargo nextest run -p zhang-query -E 'binary(oracle) & test(conformance::)'`.

mod conformance;
mod export;
mod golden;
mod having_pivot;
mod period;
mod server_features;
mod statements;
mod tables;
