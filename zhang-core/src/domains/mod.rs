use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::DateTime;
use chrono_tz::Tz;
use itertools::Itertools;
use log::debug;
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::SpanInfo;

use crate::domains::schemas::ErrorDomain;
use crate::store::{BalanceAssertionDomain, DocumentDomain, DocumentType, Store, TransactionDomain};
use crate::utils::id::FromSpan;
use crate::ZhangResult;

pub mod schemas;

pub struct Operations {
    pub timezone: Tz,
    pub store: Arc<RwLock<Store>>,
}

impl Operations {
    pub fn read(&self) -> RwLockReadGuard<'_, Store> {
        self.store.read().expect("poison lock detect")
    }
    pub fn write(&self) -> RwLockWriteGuard<'_, Store> {
        self.store.write().expect("poison lock detect")
    }
}

impl Operations {
    /// insert new transaction, folded from the directive with index `directive` in the ledger's directives
    pub(crate) fn insert_transaction(&mut self, id: &Uuid, sequence: i32, directive: usize, datetime: DateTime<Tz>) -> ZhangResult<()> {
        let mut store = self.write();
        store.transactions.insert(
            *id,
            TransactionDomain {
                id: *id,
                sequence,
                directive,
                datetime,
            },
        );
        Ok(())
    }

    /// `id`, or if a transaction or a balance assertion has it already, the first id derived from it
    /// ([`FromSpan::derived`]) that none has. Directives can share a span, which ids are derived from: the padding
    /// transactions of a `pad` serving several currencies, a `balance ... with pad`, whose check is kept, and its
    /// padding transaction, and the directives a plugin emits for one of the ledger. A derived id lives apart from
    /// posting ids ([`FromSpan::from_txn_posting`]), and so does `id`, which is no posting id of its own transaction
    pub(crate) fn unused_id(&self, id: Uuid) -> Uuid {
        let store = self.read();
        (0..)
            .map(|n| if n == 0 { id } else { Uuid::derived(&id, n) })
            .find(|candidate| !store.transactions.contains_key(candidate) && !store.balance_assertion_ids.contains(candidate))
            .expect("an id is free")
    }

    /// record a checked `balance` assertion
    pub(crate) fn insert_balance_assertion(&mut self, assertion: BalanceAssertionDomain) -> ZhangResult<()> {
        let mut store = self.write();
        store.balance_assertion_ids.insert(assertion.id);
        store.balance_assertions.push(assertion);
        Ok(())
    }

    /// insert the document of a `document` directive, at the directive's datetime
    pub(crate) fn insert_document(
        &mut self, datetime: DateTime<Tz>, filename: Option<&str>, path: String, alternate: Option<String>, document_type: DocumentType,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        store.documents.push(DocumentDomain {
            datetime,
            document_type,
            filename: filename.map(|it| it.to_owned()),
            path,
            alternate,
        });

        Ok(())
    }
}

impl Operations {
    pub fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store.errors.iter().cloned().collect_vec())
    }
}

// for insert and new operations
impl Operations {
    pub fn new_error(&mut self, error_kind: ErrorKind, span: &SpanInfo, metas: HashMap<String, String>) -> ZhangResult<()> {
        let mut store = self.write();
        debug!("insert a new error [{}] [span: {:?}] [meta:{:?}]", error_kind, span, metas);
        store.errors.push(ErrorDomain {
            id: Uuid::from_span(span).to_string(),
            error_type: error_kind,
            span: Some(span.clone()),
            metas,
        });
        Ok(())
    }
}
