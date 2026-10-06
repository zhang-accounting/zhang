//! What a load decides about the directives of a ledger that they do not say themselves ([`Outcome`]).

use uuid::Uuid;
use zhang_ast::amount::Amount;

/// What the load decided about one of [`Ledger::directives`](crate::ledger::Ledger::directives), at the same index in
/// [`Ledger::outcomes`](crate::ledger::Ledger::outcomes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// its place, from 0, in the order zhang processed the ledger: the `seq` of `#entries`. `None` for a transaction
    /// the load rejected, which is no entry
    pub seq: Option<u32>,
    pub detail: Detail,
}

/// What the load decided about a directive besides its place.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Detail {
    #[default]
    None,
    /// a transaction the load accepted: its id, which stays the same from one load to the next as URLs name it, and
    /// the errors the load reported at its place (indexes into [`Ledger::errors`](crate::ledger::Ledger::errors))
    Transaction { id: Uuid, errors: Vec<usize> },
    /// a checked `balance` assertion, `balance` or `balance ... with pad`: the id of its check, an id like a
    /// transaction's, the balance of the asserted account and its sub-accounts in the asserted currency where it
    /// stands, as in beancount, and whether it holds. A check books nothing, and a failing one is also an
    /// `AccountBalanceCheckError`
    Assertion { id: Uuid, balance: Amount, passed: bool },
    /// a `document`: the path within the ledger the load resolved it to, and where else it may be. A `document` of a
    /// beancount ledger on a remote source, whose files are not looked at on load, may be at its path relative to the
    /// ledger's root, as earlier versions wrote it, rather than relative to its file, as beancount reads it
    Document { path: String, alternate: Option<String> },
}
