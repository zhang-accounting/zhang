//! Shared test support for the zhang workspace.
//!
//! The crate is a dev-dependency of the crates with integration tests (`tests/`). It holds what those tests used to
//! copy from one another: where the fixture ledgers are and how to load them once ([`fixtures`]), how to build a
//! ledger from text or in a scratch directory ([`ledger`]), how a golden file is compared and rewritten
//! ([`golden`]), and the scenarios the parser and exporter tests of both dialects share ([`dialect`]).
//!
//! # Rules
//!
//! - **Only `tests/` use it.** A `#[cfg(test)]` module inside a crate must not import `zhang_testkit`: the library
//!   under test and the copy this crate links against are two different crates to the compiler, so their types do
//!   not match. Unit tests keep their own small helpers.
//! - **No features of the crates under test.** `zhang-core` is depended on without features, so `cargo test -p
//!   zhang-core` stays free of the plugin runtime; `zhang-server`, which enables it, is not a dependency.
//! - **Nothing here asserts a behaviour of zhang.** The crate loads and compares; what a test expects stays in the
//!   test.
//!
//! # Where things are
//!
//! | need | use |
//! |---|---|
//! | a ledger from text | [`ledger::load_text`], [`ledger::try_load_text`] (its error), [`ledger::load_text_at`] (with a clock), [`ledger::load_transformed`] |
//! | a ledger on disk the test writes to or reloads | [`ledger::Scratch`] |
//! | the fava demo ledger | [`fixtures::fava_demo`] (loaded once per process) or [`ledger::fava_demo_ledger`] (a fresh load) |
//! | every ledger of `integration-tests/` and `examples/` | [`fixtures::every_fixture_ledger`] |
//! | one of them by name | [`fixtures::fixture_ledger`], [`fixtures::fixture_dir`] |
//! | the beancount oracle ledgers (pads, balances, document paths) | [`fixtures::oracle_ledgers`], [`fixtures::oracle_ledger_dir`] |
//! | a ledger of either format from a directory | [`fixtures::load_dir`] |
//! | a reproducible random source | [`XorShift`] |
//! | a golden file (`UPDATE_GOLDEN=1` rewrites it) | [`golden::assert_text`], [`golden::assert_json`] |
//! | where a transaction's metadata landed, random transactions and text layouts per dialect | [`dialect::Shape`], [`dialect::random_transaction`], [`dialect::random_layout`] |
//! | a text through one dialect's parser and exporter, with the shape and round-trip checks | [`dialect::Scenario`] |
//! | the string-escaping scenarios of both dialects | [`dialect::escaping`] |
//! | a handler's `State`, a response as JSON, the server's router (feature `server`) | [`http`] |

pub mod dialect;
pub mod fixtures;
pub mod golden;
#[cfg(feature = "server")]
pub mod http;
pub mod ledger;

pub use ledger::XorShift;
