use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::{Add, AddAssign, Sub};
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use indexmap::IndexMap;
use itertools::Itertools;
use log::debug;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, AccountType, Currency, Date, Flag, Meta, PostingCost, Rounding, SpanInfo};

use crate::constants::BALANCE_CHECK_PAYEE;
use crate::domains::schemas::{
    AccountBalanceDomain, AccountDailyBalanceDomain, AccountDomain, AccountJournalDomain, AccountStatus, BalanceWithSubAccounts, CommodityDomain, ErrorDomain,
    MetaDomain, MetaType, OptionDomain, PriceDomain, QueryDomain, TransactionInfoDomain,
};
use crate::store::{
    BalanceAssertionDomain, BudgetDomain, BudgetEvent, BudgetEventType, BudgetIntervalDetail, DocumentDomain, DocumentType, PostingDomain, PostingMetaDomain,
    Store, TransactionDomain,
};
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

pub mod schemas;

pub struct AccountCommodityLot {
    pub account: Account,
    pub amount: BigDecimal,
    pub cost: Option<Amount>,
    pub price: Option<Amount>,
    pub acquisition_date: Option<NaiveDate>,
}

pub struct Operations {
    pub timezone: Tz,
    pub store: Arc<RwLock<Store>>,
}

impl Operations {
    /// single commodity prices
    pub fn commodity_prices(&self, commodity: impl AsRef<str>) -> ZhangResult<Vec<PriceDomain>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        Ok(store.prices.iter().filter(|price| price.commodity.eq(commodity)).cloned().collect_vec())
    }
}

impl Operations {
    /// single commodity lots
    pub fn commodity_lots(&self, commodity: impl AsRef<str>) -> ZhangResult<Vec<AccountCommodityLot>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        let mut ret = vec![];
        for (account, lots) in store.commodity_lots.iter() {
            for lot in lots.iter() {
                if lot.commodity.eq(commodity) {
                    let lot = lot.clone();
                    ret.push(AccountCommodityLot {
                        account: Account::from_str(account).map_err(|_| ZhangError::InvalidAccount)?,
                        amount: lot.amount,
                        cost: lot.cost,
                        acquisition_date: lot.acquisition_date,
                        price: None,
                    })
                }
            }
        }
        Ok(ret)
    }
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
        &mut self, trx_id: &Uuid, posting_idx: usize, account_name: &str, unit: Option<Amount>, cost: Option<PostingCost>, inferred_amount: Amount,
        previous_amount: Amount, after_amount: Amount, meta: Meta,
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
            account: Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?,
            unit,
            cost: cost.and_then(|it| it.base),
            inferred_amount,
            previous_amount,
            after_amount,
            metas: PostingMetaDomain::of(meta),
        };
        store.postings.push(posting.clone());
        let txn_header = store
            .transactions
            .get_mut(trx_id)
            .expect("invalid context: cannot find txn header when inserting postings");
        txn_header.postings.push(posting);
        Ok(())
    }

    /// record a checked `balance` assertion
    pub(crate) fn insert_balance_assertion(&mut self, assertion: BalanceAssertionDomain) -> ZhangResult<()> {
        let mut store = self.write();
        store.balance_assertions.push(assertion);
        Ok(())
    }

    /// insert document
    /// datetime means:
    ///  - for transaction document: transaction datetime
    ///  - for account document: document linking datetime
    pub(crate) fn insert_document(&mut self, datetime: DateTime<Tz>, filename: Option<&str>, path: String, document_type: DocumentType) -> ZhangResult<()> {
        let mut store = self.write();

        store.documents.push(DocumentDomain {
            datetime,
            document_type,
            filename: filename.map(|it| it.to_owned()),
            path,
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

        let posting: Option<&PostingDomain> = store
            .postings
            .iter()
            .filter(|posting| posting.account.eq(&account))
            .filter(|posting| posting.after_amount.commodity.eq(&currency))
            .filter(|posting| posting.trx_datetime.le(&datetime))
            .sorted_by_key(|posting| posting.trx_datetime)
            .next_back();

        Ok(posting.map(|it| Amount {
            number: it.after_amount.number.clone(),
            commodity: currency.to_owned(),
        }))
    }

    pub fn get_latest_price(&self, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
        let store = self.read();
        let option = store
            .prices
            .iter()
            .filter(|price| price.commodity.eq(from.as_ref()))
            .filter(|price| price.target_commodity.eq(to.as_ref()))
            .sorted_by_key(|it| it.datetime)
            .next_back()
            .cloned();
        Ok(option)
    }
    pub fn get_commodity_balances(&self, commodity: impl AsRef<str>) -> ZhangResult<BigDecimal> {
        let mut total = BigDecimal::zero();
        let store = self.read();
        let commodity = commodity.as_ref();
        for (account, lots) in store.commodity_lots.iter() {
            let account = Account::from_str(account).map_err(|_| ZhangError::InvalidAccount)?;
            if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
                let account_sum: BigDecimal = lots.iter().filter(|lot| lot.commodity.eq(commodity)).map(|it| &it.amount).sum();
                total.add_assign(account_sum);
            }
        }
        Ok(total)
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

    pub fn accounts_latest_balance(&mut self) -> ZhangResult<Vec<AccountDailyBalanceDomain>> {
        let store = self.read();

        let mut ret: HashMap<Account, IndexMap<Currency, BTreeMap<NaiveDate, Amount>>> = HashMap::new();

        for posting in store.postings.iter().cloned().sorted_by_key(|posting| posting.trx_datetime) {
            let posting: PostingDomain = posting;
            let date = posting.trx_datetime.naive_local().date();

            let account_inventory = ret.entry(posting.account).or_default();
            let dated_amount = account_inventory.entry(posting.after_amount.commodity.clone()).or_default();
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .flat_map(|(account, account_inventory)| {
                account_inventory
                    .into_iter()
                    .map(|(_, mut dated)| {
                        let (date, amount) = dated.pop_last().expect("");
                        AccountDailyBalanceDomain {
                            date,
                            account: account.name().to_owned(),
                            balance: amount,
                        }
                    })
                    .collect_vec()
            })
            .collect_vec())
    }

    /// the price of one `from` in `to` as of `date`: the latest `price` directive for the pair
    /// dated on or before `date`, like beancount.
    ///
    /// Prices with the same datetime replace each other: the last one in stream order wins
    /// (the stream is sorted stably, so that is source order). Only the direct pair is looked
    /// up: no inverse rate, and no identity rate for `from == to`.
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
    pub fn typed_meta_value<T>(&self, type_: MetaType, type_identifier: impl AsRef<str>, key: impl AsRef<str>) -> Result<Option<T>, ErrorKind>
    where
        T: FromStr<Err = ErrorKind>,
    {
        let store = self.read();

        store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq(type_.as_ref()))
            .filter(|meta| meta.type_identifier.eq(type_identifier.as_ref()))
            .find(|meta| meta.key.eq(key.as_ref()))
            .map(|it| T::from_str(&it.value))
            .transpose()
    }

    pub fn trx_tags(&mut self, trx_id: &Uuid) -> ZhangResult<Vec<String>> {
        let store = self.read();
        let tags = store.transactions.get(trx_id).map(|it| it.tags.clone()).unwrap_or_default();

        Ok(tags)
    }

    pub fn trx_links(&mut self, trx_id: &Uuid) -> ZhangResult<Vec<String>> {
        let store = self.read();
        let tags = store.transactions.get(trx_id).map(|it| it.links.clone()).unwrap_or_default();

        Ok(tags)
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

    pub fn transaction_counts(&mut self) -> ZhangResult<i64> {
        let store = self.read();
        Ok(store.transactions.len() as i64)
    }

    pub fn single_transaction(&mut self, id: &Uuid) -> ZhangResult<Option<TransactionDomain>> {
        let store = self.read();
        Ok(store.transactions.get(id).cloned())
    }

    pub fn transaction_span(&mut self, id: &Uuid) -> ZhangResult<Option<TransactionInfoDomain>> {
        let store = self.read();
        Ok(store.transactions.get(id).map(|it| TransactionInfoDomain {
            id: it.id.to_string(),
            source_file: it.span.filename.clone().unwrap_or_default(),
            span_start: it.span.start,
            span_end: it.span.end,
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

    /// get target account's all balance
    /// because the account can have multiple commodities, so the result is the array.
    pub fn single_account_all_balances(&self, account_name: &str) -> ZhangResult<HashMap<Currency, HashMap<NaiveDate, Amount>>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;

        let mut ret: HashMap<Currency, HashMap<NaiveDate, Amount>> = HashMap::new();

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

        Ok(ret)
    }

    /// the journal of one account, newest first: a row per posting and a row per balance assertion on the
    /// account itself. Every row's `account_after` is the account's own running balance, that of its own
    /// postings: an assertion changes it in no way. An assertion row also has the amount it asserted, the
    /// balance it was checked against, which includes the sub-accounts, and whether it passed
    pub fn account_journals(&mut self, account: &str) -> ZhangResult<Vec<AccountJournalDomain>> {
        let store = self.read();
        let account = Account::from_str(account).map_err(|_| ZhangError::InvalidAccount)?;

        // both in ledger order: postings and assertions share one sequence
        let postings = store.postings.iter().filter(|posting| posting.account.eq(&account)).collect_vec();
        let assertions = store.balance_assertions.iter().filter(|assertion| assertion.account.eq(&account)).collect_vec();

        // the account's own balance per currency where each assertion stands: after its last posting before it
        let mut own_balance: HashMap<&str, &Amount> = HashMap::new();
        let mut before = postings.iter().peekable();
        let mut assertion_rows = Vec::with_capacity(assertions.len());
        for assertion in assertions {
            while let Some(posting) = before.next_if(|posting| posting.trx_sequence < assertion.sequence) {
                own_balance.insert(posting.after_amount.commodity.as_str(), &posting.after_amount);
            }
            let own = own_balance
                .get(assertion.amount.commodity.as_str())
                .map(|it| (*it).clone())
                .unwrap_or_else(|| Amount::new(BigDecimal::zero(), assertion.amount.commodity.clone()));
            assertion_rows.push((assertion.datetime, assertion.sequence, assertion_journal_row(assertion, own)));
        }

        let posting_rows = postings.into_iter().map(|posting| {
            let trx_header = store.transactions.get(&posting.trx_id);
            let row = AccountJournalDomain {
                datetime: posting.trx_datetime.naive_local(),
                timestamp: posting.trx_datetime.timestamp(),
                account: posting.account.name().to_owned(),
                trx_id: posting.id.to_string(),
                payee: trx_header.and_then(|it| it.payee.clone()),
                narration: trx_header.and_then(|it| it.narration.clone()),
                inferred_unit: posting.inferred_amount.clone(),
                account_after: posting.after_amount.clone(),
                asserted: None,
                checked_balance: None,
                passed: None,
            };
            (posting.trx_datetime, posting.trx_sequence, row)
        });
        Ok(posting_rows
            .chain(assertion_rows)
            .sorted_by(|(a_datetime, a_sequence, _), (b_datetime, b_sequence, _)| {
                a_datetime.cmp(b_datetime).reverse().then(a_sequence.cmp(b_sequence).reverse())
            })
            .map(|(_, _, row)| row)
            .collect_vec())
    }

    /// the balance of every account per currency including its sub-accounts: the sum of the postings of the
    /// account and all its sub-accounts, which a balance assertion on the account is checked against
    pub fn balances_with_sub_accounts(&self) -> ZhangResult<HashMap<String, BalanceWithSubAccounts>> {
        let store = self.read();
        // the units of each account's own postings, by account name, so that an account's sub-accounts are a range
        let mut own: BTreeMap<&str, BTreeMap<&str, BigDecimal>> = BTreeMap::new();
        for account in store.accounts.keys() {
            own.entry(account.as_str()).or_default();
        }
        for posting in &store.postings {
            let units = own
                .entry(posting.account.name())
                .or_default()
                .entry(posting.inferred_amount.commodity.as_str())
                .or_insert_with(BigDecimal::zero);
            *units += &posting.inferred_amount.number;
        }
        Ok(store
            .accounts
            .keys()
            .map(|name| {
                // `Assets:Bank:` up to `Assets:Bank;` (`;` follows `:`) holds exactly the sub-accounts of `Assets:Bank`
                let (from, to) = (format!("{name}:"), format!("{name};"));
                let sub_accounts = own.range(from.as_str()..to.as_str());
                let has_sub_accounts = sub_accounts.clone().next().is_some();
                let mut balance: BTreeMap<Currency, BigDecimal> = BTreeMap::new();
                for (_, held) in own.get_key_value(name.as_str()).into_iter().chain(sub_accounts) {
                    for (currency, units) in held {
                        *balance.entry((*currency).to_owned()).or_insert_with(BigDecimal::zero) += units;
                    }
                }
                (name.clone(), BalanceWithSubAccounts { balance, has_sub_accounts })
            })
            .collect())
    }

    pub fn dated_journals(&mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> ZhangResult<Vec<PostingDomain>> {
        let store = self.read();
        Ok(store
            .postings
            .iter()
            .filter(|posting| posting.trx_datetime.ge(&from))
            .filter(|posting| posting.trx_datetime.le(&to))
            .cloned()
            .collect_vec())
    }
    pub fn account_type_dated_journals(&mut self, account_type: AccountType, from: DateTime<Utc>, to: DateTime<Utc>) -> ZhangResult<Vec<AccountJournalDomain>> {
        let store = self.read();

        let mut ret = vec![];
        for posting in store
            .postings
            .iter()
            .filter(|posting| posting.trx_datetime.ge(&from))
            .filter(|posting| posting.trx_datetime.le(&to))
            .filter(|posting| posting.account.account_type == account_type)
            .cloned()
        {
            let trx = store.transactions.get(&posting.trx_id).cloned().expect("cannot find trx");

            ret.push(AccountJournalDomain {
                datetime: posting.trx_datetime.naive_local(),
                timestamp: posting.trx_datetime.timestamp(),
                account: posting.account.name().to_owned(),
                trx_id: posting.trx_id.to_string(),
                payee: trx.payee,
                narration: trx.narration,
                inferred_unit: posting.inferred_amount,
                account_after: posting.after_amount,
                asserted: None,
                checked_balance: None,
                passed: None,
            })
        }
        Ok(ret)
    }
    pub fn accounts_dated_journals(&self, accounts: &[String], from: DateTime<Tz>, to: DateTime<Tz>) -> ZhangResult<Vec<AccountJournalDomain>> {
        let store = self.read();

        let mut ret = vec![];
        for posting in store
            .postings
            .iter()
            .filter(|posting| posting.trx_datetime.ge(&from))
            .filter(|posting| posting.trx_datetime.le(&to))
            .filter(|posting| accounts.contains(&posting.account.content))
            .cloned()
        {
            let trx = store.transactions.get(&posting.trx_id).cloned().expect("cannot find trx");

            ret.push(AccountJournalDomain {
                datetime: posting.trx_datetime.naive_local(),
                timestamp: posting.trx_datetime.timestamp(),
                account: posting.account.name().to_owned(),
                trx_id: posting.trx_id.to_string(),
                payee: trx.payee,
                narration: trx.narration,
                inferred_unit: posting.inferred_amount,
                account_after: posting.after_amount,
                asserted: None,
                checked_balance: None,
                passed: None,
            })
        }
        Ok(ret)
    }

    pub fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store.errors.iter().cloned().collect_vec())
    }
    pub fn errors_by_meta(&mut self, key: &str, value: &str) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store
            .errors
            .iter()
            .filter(|error| error.metas.get(key).map(|v| v.eq(value)).unwrap_or(false))
            .cloned()
            .collect_vec())
    }

    pub fn account(&mut self, account_name: &str) -> ZhangResult<Option<AccountDomain>> {
        let store = self.read();

        Ok(store.accounts.get(account_name).cloned())
    }
    pub fn all_open_accounts(&mut self) -> ZhangResult<Vec<AccountDomain>> {
        let store = self.read();
        Ok(store
            .accounts
            .values()
            .filter(|account| account.status == AccountStatus::Open)
            .cloned()
            .collect_vec())
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

    pub fn account_target_date_balance(&self, account_name: impl AsRef<str>, date: DateTime<Utc>) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let store = self.read();

        let account = Account::from_str(account_name.as_ref()).map_err(|_| ZhangError::InvalidAccount)?;

        let mut ret: IndexMap<Currency, BTreeMap<NaiveDate, Amount>> = IndexMap::new();

        for posting in store
            .postings
            .iter()
            .filter(|posting| posting.account.eq(&account))
            .filter(|positing| positing.trx_datetime.le(&date))
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
    /// list all budgets
    pub fn all_budgets(&self) -> ZhangResult<Vec<BudgetDomain>> {
        let store = self.read();
        Ok(store.budgets.values().cloned().collect_vec())
    }

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
        let name = name.into();
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

        detail.assigned_amount = detail.assigned_amount.add(amount.number.clone());
        detail.events.push(BudgetEvent {
            datetime: date,
            timestamp: date.timestamp(),
            amount,
            event_type,
        });
        Ok(())
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
        let name = name.into();
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

        detail.activity_amount = detail.activity_amount.add(amount.number);
        Ok(())
    }

    pub fn get_account_budget(&self, account_name: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let metas = self.metas(MetaType::AccountMeta, account_name)?;
        Ok(metas.into_iter().filter(|meta| meta.key.eq("budget")).map(|meta| meta.value).collect_vec())
    }
}

/// the row of a balance assertion in its account's journal: it adds nothing, `account_after` is `own`, the
/// account's own balance where the assertion stands, and `checked_balance` the balance the assertion was
/// checked against, which includes the sub-accounts. Its id follows the posting rows' ids, derived from
/// the assertion id like the id of a single posting
fn assertion_journal_row(assertion: &BalanceAssertionDomain, own: Amount) -> AccountJournalDomain {
    // zero, written with the decimals of the asserted amount and the balance
    let difference = (&assertion.amount.number).sub(&assertion.balance.number);
    let nothing = BigDecimal::zero().with_scale(difference.fractional_digit_count());
    AccountJournalDomain {
        datetime: assertion.datetime.naive_local(),
        timestamp: assertion.datetime.timestamp(),
        account: assertion.account.name().to_owned(),
        trx_id: Uuid::from_txn_posting(&assertion.id, 0).to_string(),
        payee: Some(BALANCE_CHECK_PAYEE.to_owned()),
        narration: Some(assertion.account.name().to_owned()),
        inferred_unit: Amount::new(nothing, assertion.amount.commodity.clone()),
        account_after: own,
        asserted: Some(assertion.amount.clone()),
        checked_balance: Some(assertion.balance.clone()),
        passed: Some(assertion.passed),
    }
}
