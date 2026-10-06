use std::collections::{HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, SpanInfo};

use crate::domains::schemas::ErrorDomain;

/// What the load decides about the ledger's directives that they do not carry themselves: the id and place of each
/// transaction it accepted, the outcome of each balance assertion, the path of each document, and the errors
#[derive(Default)]
pub struct Store {
    pub transactions: HashMap<Uuid, TransactionDomain>,

    /// the `balance` assertions, in ledger order. They are not transactions and have no postings:
    /// an assertion changes no balance
    pub balance_assertions: Vec<BalanceAssertionDomain>,
    /// the ids of [`Store::balance_assertions`], which an id given to a transaction or an assertion avoids
    pub(crate) balance_assertion_ids: HashSet<Uuid>,

    pub documents: Vec<DocumentDomain>,

    pub errors: Vec<ErrorDomain>,
}

/// A transaction zhang accepted: its id and its place. Everything else (flag, payee, narration, tags, links, the
/// booked postings and their metadata) is its directive's, `Ledger::directives[directive]`
#[derive(Clone, serde::Serialize, Debug)]
pub struct TransactionDomain {
    pub id: Uuid,
    pub sequence: i32,
    /// the index in [`Ledger::directives`](crate::ledger::Ledger::directives) of the directive it was folded from
    pub directive: usize,
    pub datetime: DateTime<Tz>,
}

/// A `balance` assertion as the load checked it: the asserted amount next to the account's balance
/// where the assertion stands.
///
/// It records a check, it is not a transaction: it has no postings, so nothing that sums postings
/// books it, and no balance depends on it. A failing assertion is reported as an
/// `AccountBalanceCheckError` and changes no number; `balance ... with pad` (a `P` transaction)
/// is how a balance is corrected on purpose.
#[derive(Clone, serde::Serialize, Debug)]
pub struct BalanceAssertionDomain {
    /// derived from the span of the `balance` directive, like a transaction id
    pub id: Uuid,
    /// its place in the journal: assertions and transactions share one sequence
    pub sequence: i32,
    /// the index in [`Ledger::directives`](crate::ledger::Ledger::directives) of the `balance` directive it checks
    pub directive: usize,
    pub datetime: DateTime<Tz>,
    pub account: Account,
    /// the asserted amount
    pub amount: Amount,
    /// the explicit tolerance (`~`); `None` asserts the exact amount
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize_option")]
    pub tolerance: Option<BigDecimal>,
    /// the account's balance in the asserted currency where the assertion stands: the sum of the
    /// postings of the account and all its sub-accounts before it, as in beancount
    pub balance: Amount,
    /// whether `balance` is within `tolerance` of `amount`
    pub passed: bool,
    pub span: SpanInfo,
}

/// What a stored document belongs to. The store keeps the `document` directives only, with the path each was resolved
/// to on load; the documents a transaction or a posting names in its `document` metadata are listed by the query
/// engine, in `#documents`.
#[derive(Clone, serde::Serialize)]
pub enum DocumentType {
    Account(Account),
}

impl DocumentType {
    pub fn match_account(&self, account_name: &str) -> bool {
        let DocumentType::Account(account) = self;
        account.name().eq(account_name)
    }
    pub fn as_account(&self) -> Option<String> {
        let DocumentType::Account(account) = self;
        Some(account.name().to_owned())
    }
}

#[derive(Clone, serde::Serialize)]
pub struct DocumentDomain {
    pub datetime: DateTime<Tz>,
    pub document_type: DocumentType,
    pub filename: Option<String>,
    pub path: String,
    /// where the document may be instead, looked at when nothing is at `path`: a `document` of a beancount ledger on
    /// a remote source, whose files are not looked at on load, may be at its path relative to the ledger's root, as
    /// earlier versions wrote it, rather than relative to its file, as beancount reads it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, PartialEq)]
pub struct CommodityLotRecord {
    pub commodity: String,
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize")]
    pub amount: BigDecimal,

    pub cost: Option<Amount>,

    // acquisition date
    pub acquisition_date: Option<NaiveDate>,

    /// the lot's label, written on the cost that opened it (`{100 USD, "a"}`); lots differing only
    /// by label are distinct. Left out of the serialized store when the lot has none, so a store
    /// without labels serializes as before
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use zhang_ast::Account;

    use crate::store::DocumentType;

    #[test]
    fn should_match_document_type() {
        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());

        assert!(account_type.match_account("Assets:A"));
        assert!(!account_type.match_account("Assets:A:B"));
        assert!(!account_type.match_account("Assets:C"));
    }

    #[test]
    fn should_return_account() {
        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());
        assert_eq!(account_type.as_account(), Some("Assets:A".to_owned()));
    }
}
