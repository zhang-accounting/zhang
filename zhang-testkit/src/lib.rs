//! Shared test support for the zhang workspace.
//!
//! The crate is a dev-dependency of the crates with integration tests (`tests/`). It holds what those tests used to
//! copy from one another: where the fixture ledgers are and how to load them once ([`fixtures`]), how to build a
//! ledger from text or in a scratch directory ([`ledger`]), and how a golden file is compared and rewritten
//! ([`golden`]).
//!
//! # Rules
//!
//! - **Only `tests/` use it.** A `#[cfg(test)]` module inside a crate must not import `zhang_testkit`: the library
//!   under test and the copy this crate links against are two different crates to the compiler, so their types do
//!   not match. Unit tests keep their own small helpers.
//! - **No features of the crates under test.** `zhang-core` is depended on without features, so `cargo test -p
//!   zhang-core` stays free of the plugin runtime; `zhang-server`, which enables it, is not a dependency. The
//!   query engine is behind the `query` feature, which only the crates that test it enable.
//! - **Nothing here asserts a behaviour of zhang.** The crate loads and compares; what a test expects stays in the
//!   test.
//!
//! # Where things are
//!
//! | need | use |
//! |---|---|
//! | a ledger from text | [`ledger::load_text`], [`ledger::load_text_at`] (with a clock), [`ledger::load_transformed`] |
//! | a ledger on disk the test writes to or reloads | [`ledger::Scratch`] |
//! | the fava demo ledger | [`fixtures::fava_demo`] (loaded once per process) or [`ledger::fava_demo_ledger`] (a fresh load) |
//! | every ledger of `integration-tests/` and `examples/` | [`fixtures::every_fixture_ledger`] |
//! | one of them by name | [`fixtures::fixture_ledger`], [`fixtures::fixture_dir`] |
//! | the beancount oracle ledgers (pads, balances, document paths) | [`fixtures::oracle_ledgers`], [`fixtures::oracle_ledger_dir`] |
//! | a ledger of either format from a directory | [`fixtures::load_dir`] |
//! | a reproducible random source | [`XorShift`] |
//! | a golden file (`UPDATE_GOLDEN=1` rewrites it) | [`golden::assert_text`], [`golden::assert_json`] |
//! | a beanquery oracle case set, run and compared under one set of rules (feature `query`) | [`oracle::load_case_files`] and the other loaders, [`oracle::run_case`] |

pub mod fixtures;
pub mod golden;
pub mod ledger;
#[cfg(feature = "query")]
pub mod oracle;

pub use ledger::XorShift;
