//! The kit's loaders with the budgets interpreted by zhang's native budget code, through RFC #698's test switch
//! ([`crate::fixtures::native_budgets`]), whatever the kit's features: for the tests of the native code itself, such
//! as the query engine's, which compile their queries against the core catalog and its native provider of the budget
//! tables (`Query::compile`), where a ledger whose budgets the bundled script computes has a catalog of its own.
// TODO(P5): goes with the native budget code, and its tests with it

use std::path::PathBuf;

use zhang_ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::ledger::Ledger;

use crate::fixtures::native_budgets;
use crate::ledger;

/// [`ledger::load_ledger`], natively.
pub fn load_ledger(dir: PathBuf, entry: &str) -> Ledger {
    ledger::load_ledger_with(dir, entry, native_budgets())
}

/// [`ledger::load_text`], natively.
pub fn load_text(content: &str) -> Ledger {
    ledger::load_text_with(content, native_budgets())
}

/// [`ledger::load_text_at`], natively.
pub fn load_text_at(content: &str, clock: Clock) -> Ledger {
    ledger::load_text_at_with(content, clock, native_budgets())
}

/// [`ledger::load_transformed`], natively.
pub fn load_transformed(content: &str, transform: impl FnOnce(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>) -> Ledger {
    ledger::load_transformed_with(content, transform, native_budgets())
}

/// [`ledger::fava_demo_ledger`], natively.
pub fn fava_demo_ledger() -> Ledger {
    load_ledger(crate::fixtures::fixture_dir("fava-demo-ledger"), "main.zhang")
}
