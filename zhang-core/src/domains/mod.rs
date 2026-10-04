use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::{Add, Sub};
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;
use indexmap::IndexMap;
use itertools::Itertools;
use log::debug;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Currency, Date, Flag, Meta, PostingCost, Rounding, SpanInfo};

use crate::domains::schemas::{
    AccountBalanceDomain, AccountDomain, AccountStatus, CommodityDomain, ErrorDomain, MetaDomain, MetaType, OptionDomain, PriceDomain, QueryDomain,
    TransactionInfoDomain,
};
use crate::store::{
    BalanceAssertionDomain, BudgetDomain, BudgetEvent, BudgetEventType, BudgetIntervalDetail, DocumentDomain, DocumentType, PostingDomain, PostingMetaDomain,
    Store, TransactionDomain,
};
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
    /// insert or update account
    /// if account exists, then update its status only
    pub(crate) fn insert_or_update_account(&mut self, datetime: DateTime<Tz>, account: Account, status: AccountStatus, alias: Option<&str>) -> ZhangResult<()> {
        let mut store = self.write();
        let account_domain = store.accounts.entry(account.name().to_owned()).or_insert_with(|| AccountDomain {
            date: datetime.naive_local(),
            r#type: account.account_type.to_string(),
            name: account.name().to_owned(),
            status,
            alias: alias.map(|it| it.to_owned()),
        });

        // if account exists, the property only can be changed is status;
        account_domain.status = status;

        Ok(())
    }

    /// insert new transaction
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_transaction(
        &mut self, id: &Uuid, sequence: i32, datetime: DateTime<Tz>, flag: Flag, payee: Option<&str>, narration: Option<&str>, tags: Vec<String>,
        links: Vec<String>, span: &SpanInfo,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        store.transactions.insert(
            *id,
            TransactionDomain {
                id: *id,
                sequence,
                datetime,
                flag,
                payee: payee.map(|it| it.to_owned()),
                narration: narration.map(|it| it.to_owned()),
                span: span.clone(),
                tags,
                links,
                postings: vec![],
            },
        );

        Ok(())
    }

    /// check a booked transaction's residual (the sum of its weights per commodity), returning the
    /// problem to report:
    /// - [`ErrorKind::CommodityDoesNotDefine`] if a weight commodity is not defined. It goes first:
    ///   the residual of an undefined commodity has no precision to round with;
    /// - else [`ErrorKind::UnbalancedTransaction`] if the residual of a commodity does not round to
    ///   zero under the commodity's precision and rounding.
    ///
    /// Commodities are checked in commodity order, so the result is deterministic (#441)
    pub(crate) fn check_transaction_balance(&self, residual: &BTreeMap<Currency, BigDecimal>) -> ZhangResult<Option<ErrorKind>> {
        let mut commodities = Vec::with_capacity(residual.len());
        for currency in residual.keys() {
            let Some(commodity) = self.commodity(currency)? else {
                return Ok(Some(ErrorKind::CommodityDoesNotDefine));
            };
            commodities.push(commodity);
        }
        for (commodity, amount) in commodities.iter().zip(residual.values()) {
            let rounded = amount.with_scale_round(commodity.precision as i64, commodity.rounding.to_mode());
            if !rounded.is_zero() {
                return Ok(Some(ErrorKind::UnbalancedTransaction));
            }
        }
        Ok(None)
    }

    /// insert transaction postings
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_transaction_posting(
        &mut self, trx_id: &Uuid, posting_idx: usize, flag: Option<Flag>, account_name: &str, unit: Option<Amount>, cost: Option<PostingCost>,
        inferred_amount: Amount, previous_amount: Amount, after_amount: Amount, meta: Meta,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        let trx = store
            .transactions
            .get(trx_id)
            .cloned()
            .expect("invalid context: cannot find txn header when inserting postings");
        let posting = PostingDomain {
            id: Uuid::from_txn_posting(trx_id, posting_idx),
            trx_id: *trx_id,
            trx_sequence: trx.sequence,
            trx_datetime: trx.datetime,
            flag,
            account: Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?,
            unit,
            cost: cost.and_then(|it| it.base),
            inferred_amount,
            previous_amount,
            after_amount,
            metas: PostingMetaDomain::of(meta),
        };
        store.push_posting(posting.clone());
        let txn_header = store
            .transactions
            .get_mut(trx_id)
            .expect("invalid context: cannot find txn header when inserting postings");
        txn_header.postings.push(posting);
        Ok(())
    }

    /// `id`, or if a transaction, one of its postings or a balance assertion has it already, the first id derived
    /// from it ([`FromSpan::derived`]) that none has. Directives can share a span, which ids are derived from: the
    /// padding transactions of a `pad` serving several currencies, a `balance ... with pad`, whose check is kept, and
    /// its padding transaction, and the directives a plugin emits for one of the ledger. A derived id lives apart from
    /// posting ids, so only the postings of the transaction with `id` itself could share one
    pub(crate) fn unused_id(&self, id: Uuid) -> Uuid {
        let store = self.read();
        let postings = store.transactions.get(&id).map(|txn| txn.postings.as_slice()).unwrap_or_default();
        (0..)
            .map(|n| if n == 0 { id } else { Uuid::derived(&id, n) })
            .find(|candidate| {
                !store.transactions.contains_key(candidate)
                    && !store.balance_assertion_ids.contains(candidate)
                    && postings.iter().all(|posting| posting.id != *candidate)
            })
            .expect("an id is free")
    }

    /// record a checked `balance` assertion
    pub(crate) fn insert_balance_assertion(&mut self, assertion: BalanceAssertionDomain) -> ZhangResult<()> {
        let mut store = self.write();
        store.balance_assertion_ids.insert(assertion.id);
        store.balance_assertions.push(assertion);
        Ok(())
    }

    /// insert document
    /// datetime means:
    ///  - for transaction document: transaction datetime
    ///  - for account document: document linking datetime
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

    pub(crate) fn account_target_day_balance(&mut self, account_name: &str, datetime: DateTime<Tz>, currency: &str) -> ZhangResult<Option<Amount>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;

        let posting = store.last_posting_at(account.name(), currency, datetime);

        Ok(posting.map(|it| Amount {
            number: it.after_amount.number.clone(),
            commodity: currency.to_owned(),
        }))
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

    pub fn get_price(&mut self, date: NaiveDateTime, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
        let store = self.read();
        let price = store
            .prices
            .iter()
            .filter(|price| price.commodity.eq(from.as_ref()))
            .filter(|price| price.target_commodity.eq(to.as_ref()))
            .filter(|price| price.datetime.le(&date))
            // `max_by_key` returns the last of several equal maxima, so the last same-day price wins
            .max_by_key(|price| price.datetime)
            .cloned();
        Ok(price)
    }

    pub fn metas(&self, type_: MetaType, type_identifier: impl AsRef<str>) -> ZhangResult<Vec<MetaDomain>> {
        let store = self.read();
        Ok(store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq(type_.as_ref()))
            .filter(|meta| meta.type_identifier.eq(type_identifier.as_ref()))
            .cloned()
            .collect_vec())
    }

    pub fn meta(&self, type_: MetaType, type_identifier: impl AsRef<str>, key: impl AsRef<str>) -> ZhangResult<Option<MetaDomain>> {
        let store = self.read();
        Ok(store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq(type_.as_ref()))
            .filter(|meta| meta.type_identifier.eq(type_identifier.as_ref()))
            .find(|meta| meta.key.eq(key.as_ref()))
            .cloned())
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

    pub fn transaction_span(&mut self, id: &Uuid) -> ZhangResult<Option<TransactionInfoDomain>> {
        let store = self.read();
        Ok(store.transactions.get(id).map(|it| TransactionInfoDomain {
            id: it.id.to_string(),
            source_file: it.span.filename.clone().unwrap_or_default(),
            span_start: it.span.start,
            span_end: it.span.end,
            span: it.span.clone(),
        }))
    }

    /// get target account's latest balance
    /// because the account can have multiple commodities, so the result is the array.
    pub fn single_account_latest_balances(&self, account_name: &str) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;

        let mut ret: IndexMap<Currency, BTreeMap<NaiveDate, Amount>> = IndexMap::new();

        for posting in store
            .postings
            .iter()
            .filter(|posting| posting.account.eq(&account))
            .cloned()
            .sorted_by_key(|posting| posting.trx_datetime)
        {
            let posting: PostingDomain = posting;
            let date = posting.trx_datetime.naive_local().date();

            let dated_amount = ret.entry(posting.after_amount.commodity.clone()).or_default();
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .map(|(_, mut balance)| {
                let (date, amount) = balance.pop_last().expect("");
                AccountBalanceDomain {
                    datetime: date.and_time(NaiveTime::default()),
                    account: account.name().to_owned(),
                    account_status: AccountStatus::Open,
                    balance: amount,
                }
            })
            .collect_vec())
    }

    pub fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store.errors.iter().cloned().collect_vec())
    }

    pub fn account(&mut self, account_name: &str) -> ZhangResult<Option<AccountDomain>> {
        let store = self.read();

        Ok(store.accounts.get(account_name).cloned())
    }
    pub fn all_accounts(&mut self) -> ZhangResult<Vec<String>> {
        let store = self.read();
        Ok(store.accounts.keys().map(|it| it.to_owned()).collect_vec())
    }

    pub fn all_payees(&mut self) -> ZhangResult<Vec<String>> {
        let store = self.read();
        let payees: HashSet<String> = store
            .transactions
            .values()
            .filter_map(|it| it.payee.as_ref())
            .filter(|it| !it.is_empty())
            .map(|it| it.to_owned())
            .collect();

        Ok(payees.into_iter().collect_vec())
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

        for (meta_key, meta_value) in meta.get_flatten() {
            let option = store
                .metas
                .iter_mut()
                .filter(|it| it.type_identifier.eq(type_identifier.as_ref()))
                .filter(|it| it.meta_type.eq(type_.as_ref()))
                .find(|it| it.key.eq(&meta_key));
            if let Some(meta) = option {
                meta.value = meta_value.to_plain_string()
            } else {
                store.metas.push(MetaDomain {
                    meta_type: type_.as_ref().to_string(),
                    type_identifier: type_identifier.as_ref().to_owned(),
                    key: meta_key,
                    value: meta_value.to_plain_string(),
                });
            }
        }
        Ok(())
    }

    pub fn close_account(&mut self, account_name: &str) -> ZhangResult<()> {
        let mut store = self.write();

        let option = store.accounts.get_mut(account_name);

        if let Some(account) = option {
            account.status = AccountStatus::Close
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

/// Budget Related Operations
impl Operations {
    /// check if budget exists
    pub fn contains_budget(&self, name: impl AsRef<str>) -> bool {
        let store = self.read();
        store.budgets.contains_key(name.as_ref())
    }

    /// init or create a new budget
    pub fn init_budget(
        &mut self, name: impl Into<String>, commodity: impl Into<String>, date: DateTime<Tz>, alias: Option<String>, category: Option<String>,
    ) -> ZhangResult<()> {
        let mut store = self.write();
        let name = name.into();
        let commodity = commodity.into();
        let interval = (date.year() as u32) * 100 + date.month();

        let budget_domain = store.budgets.entry(name.clone()).or_insert(BudgetDomain {
            name,
            commodity: commodity.clone(),
            alias,
            category,
            closed: false,
            detail: Default::default(),
        });
        budget_domain.detail.entry(interval).or_insert(BudgetIntervalDetail {
            date: interval,
            events: vec![],
            assigned_amount: Amount::zero(&commodity),
            activity_amount: Amount::zero(&commodity),
        });
        Ok(())
    }

    /// get target month's detail. The budget must exist (check with [Self::contains_budget])
    pub fn budget_month_detail(&self, name: impl Into<String>, interval: u32) -> ZhangResult<Option<BudgetIntervalDetail>> {
        let store = self.read();
        let name = name.into();
        let target_budget = store.budgets.get(&name).expect("budget does not exist");

        Ok(target_budget
            .detail
            .iter()
            .filter(|item| item.0 <= &interval)
            .max_by_key(|item| item.0)
            .map(|item| item.1.clone())
            .map(|fetched_detail| {
                if fetched_detail.date == interval {
                    fetched_detail
                } else {
                    BudgetIntervalDetail {
                        date: interval,
                        events: vec![],
                        assigned_amount: fetched_detail.assigned_amount.sub(fetched_detail.activity_amount.number),
                        activity_amount: Amount::zero(&target_budget.commodity),
                    }
                }
            }))
    }

    /// add amount to target month's budget. The budget must exist (check with [Self::contains_budget])
    pub fn budget_add_assigned_amount(&mut self, name: impl Into<String>, date: DateTime<Tz>, event_type: BudgetEventType, amount: Amount) -> ZhangResult<()> {
        self.update_budget_month(name.into(), date, |detail| {
            detail.assigned_amount = detail.assigned_amount.add(amount.number.clone());
            detail.events.push(BudgetEvent {
                datetime: date,
                timestamp: date.timestamp(),
                amount,
                event_type,
            });
        })
    }

    /// transfer amount between budgets
    pub fn budget_transfer(&mut self, date: DateTime<Tz>, from: impl Into<String>, to: impl Into<String>, amount: Amount) -> ZhangResult<()> {
        self.budget_add_assigned_amount(from, date, BudgetEventType::Transfer, amount.neg())?;
        self.budget_add_assigned_amount(to, date, BudgetEventType::Transfer, amount)?;
        Ok(())
    }

    /// close budget
    pub fn budget_close(&mut self, name: impl AsRef<str>, _date: Date) -> ZhangResult<()> {
        let mut store = self.write();
        let name = name.as_ref();
        if let Some(budget) = store.budgets.get_mut(name) {
            budget.closed = true;
        }
        Ok(())
    }

    /// add activity to target month's budget. The budget must exist (check with [Self::contains_budget])
    pub fn budget_add_activity(&mut self, name: impl Into<String>, date: DateTime<Tz>, amount: Amount) -> ZhangResult<()> {
        self.update_budget_month(name.into(), date, |detail| {
            detail.activity_amount = detail.activity_amount.add(amount.number);
        })
    }

    /// apply `update` to the detail of budget `name` in the month of `date`, which is created on first use from the
    /// latest earlier month (see [Self::budget_month_detail]), or empty. The budget must exist
    fn update_budget_month(&mut self, name: String, date: DateTime<Tz>, update: impl FnOnce(&mut BudgetIntervalDetail)) -> ZhangResult<()> {
        let interval = (date.year() as u32) * 100 + date.month();

        let previous_budget_detail = self.budget_month_detail(&name, interval)?;

        let mut store = self.write();
        let target_budget = store.budgets.get_mut(&name).expect("budget does not exist");

        let detail = target_budget
            .detail
            .entry(interval)
            .or_insert(previous_budget_detail.unwrap_or(BudgetIntervalDetail {
                date: interval,
                events: vec![],
                assigned_amount: Amount::zero(&target_budget.commodity),
                activity_amount: Amount::zero(&target_budget.commodity),
            }));

        update(detail);
        Ok(())
    }

    pub fn get_account_budget(&self, account_name: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let metas = self.metas(MetaType::AccountMeta, account_name)?;
        Ok(metas.into_iter().filter(|meta| meta.key.eq("budget")).map(|meta| meta.value).collect_vec())
    }
}
