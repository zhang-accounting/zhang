use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use itertools::Itertools;
use log::debug;
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Meta, Rounding, SpanInfo};

use crate::domains::schemas::{AccountDomain, AccountStatus, CommodityDomain, ErrorDomain, MetaDomain, MetaType, OptionDomain, PriceDomain, QueryDomain};
use crate::store::{BalanceAssertionDomain, DocumentDomain, DocumentType, Store, TransactionDomain};
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

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
    /// insert an account at its first `open`; a later `open` changes nothing. Its status is the account
    /// lifecycle's, set once the ledger is processed ([`crate::ledger::Ledger::account_status`])
    pub(crate) fn insert_account(&mut self, datetime: DateTime<Tz>, account: Account, alias: Option<&str>) -> ZhangResult<()> {
        let mut store = self.write();
        store.accounts.entry(account.name().to_owned()).or_insert_with(|| AccountDomain {
            date: datetime.naive_local(),
            r#type: account.account_type.to_string(),
            name: account.name().to_owned(),
            status: AccountStatus::Open,
            alias: alias.map(|it| it.to_owned()),
        });
        Ok(())
    }

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

    /// insert single price
    pub(crate) fn insert_price(&mut self, datetime: DateTime<Tz>, commodity: &str, amount: &BigDecimal, target_commodity: &str) -> ZhangResult<()> {
        let mut store = self.write();
        store.prices.push(PriceDomain {
            datetime: datetime.naive_local(),
            commodity: commodity.to_owned(),
            amount: amount.clone(),
            target_commodity: target_commodity.to_owned(),
        });
        Ok(())
    }

    /// insert a saved query; queries with the same name are all kept
    pub(crate) fn insert_query(&mut self, date: NaiveDate, name: String, query: String) -> ZhangResult<()> {
        let mut store = self.write();
        store.queries.push(QueryDomain { date, name, query });
        Ok(())
    }

    /// all saved queries, in ledger order (by date, then source order)
    pub fn queries(&self) -> ZhangResult<Vec<QueryDomain>> {
        let store = self.read();
        Ok(store.queries.clone())
    }
}

impl Operations {
    pub fn options(&mut self) -> ZhangResult<Vec<OptionDomain>> {
        let store = self.read();

        Ok(store.options.clone().into_iter().map(|(key, value)| OptionDomain { key, value }).collect_vec())
    }

    /// fetch option's value given option key,
    /// the [T] means the type of option's value
    pub fn option<T>(&self, key: impl AsRef<str>) -> ZhangResult<Option<T>>
    where
        T: FromStr,
    {
        let store = self.read();

        store
            .options
            .get(key.as_ref())
            .map(|value| T::from_str(value).map_err(|_| ZhangError::InvalidOptionValue))
            .transpose()
    }

    /// the metas of `type_identifier` of `type_`, in store order
    pub fn metas(&self, type_: MetaType, type_identifier: impl AsRef<str>) -> ZhangResult<Vec<MetaDomain>> {
        let store = self.read();
        Ok(store.metas_of(type_.as_ref(), type_identifier.as_ref()).cloned().collect_vec())
    }

    /// the first meta of `type_identifier` of `type_` with `key`
    pub fn meta(&self, type_: MetaType, type_identifier: impl AsRef<str>, key: impl AsRef<str>) -> ZhangResult<Option<MetaDomain>> {
        let store = self.read();
        let meta = store
            .metas_of(type_.as_ref(), type_identifier.as_ref())
            .find(|meta| meta.key.eq(key.as_ref()))
            .cloned();
        Ok(meta)
    }

    pub fn commodity(&self, name: &str) -> ZhangResult<Option<CommodityDomain>> {
        let store = self.read();
        Ok(store.commodities.get(name).cloned())
    }

    pub fn exist_commodity(&mut self, name: &str) -> ZhangResult<bool> {
        Ok(self.commodity(name)?.is_some())
    }

    pub fn exist_account(&mut self, name: &str) -> ZhangResult<bool> {
        Ok(self.account(name)?.is_some())
    }

    pub fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store.errors.iter().cloned().collect_vec())
    }

    pub fn account(&mut self, account_name: &str) -> ZhangResult<Option<AccountDomain>> {
        let store = self.read();

        Ok(store.accounts.get(account_name).cloned())
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

    pub fn insert_or_update_options(&mut self, key: &str, value: &str) -> ZhangResult<()> {
        let mut store = self.write();

        store.options.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub fn insert_meta(&mut self, type_: MetaType, type_identifier: impl AsRef<str>, meta: Meta) -> ZhangResult<()> {
        let mut store = self.write();

        // a key the owner already has is updated in place (its first entry, as the scan found it); a new one is
        // appended
        for (meta_key, meta_value) in meta.get_flatten() {
            match store.meta_position(type_.as_ref(), type_identifier.as_ref(), &meta_key) {
                Some(position) => store.metas[position].value = meta_value.to_plain_string(),
                None => store.push_meta(MetaDomain {
                    meta_type: type_.as_ref().to_string(),
                    type_identifier: type_identifier.as_ref().to_owned(),
                    key: meta_key,
                    value: meta_value.to_plain_string(),
                }),
            }
        }
        Ok(())
    }

    pub fn insert_commodity(&mut self, name: &String, precision: i32, prefix: Option<String>, suffix: Option<String>, rounding: Rounding) -> ZhangResult<()> {
        let mut store = self.write();
        store.commodities.insert(
            name.to_owned(),
            CommodityDomain {
                name: name.to_owned(),
                precision,
                prefix,
                suffix,
                rounding,
            },
        );
        Ok(())
    }
}
