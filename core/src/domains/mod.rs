use crate::constants::KEY_DEFAULT_COMMODITY_PRECISION;
use crate::domains::schemas::{
    AccountBalanceDomain, AccountDailyBalanceDomain, AccountDomain, AccountJournalDomain, AccountStatus, CommodityDomain, ErrorDomain, ErrorType, MetaDomain,
    MetaType, OptionDomain, PriceDomain, TransactionInfoDomain,
};
use crate::store::{CommodityLotRecord, DocumentDomain, DocumentType, PostingDomain, Store, TransactionHeaderDomain};
use crate::{ZhangError, ZhangResult};
use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hash;
use std::iter::Rev;
use std::ops::AddAssign;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, AccountType, Currency, Flag, Meta, SpanInfo};

pub mod schemas;

#[derive(Debug, Deserialize)]
pub struct AccountAmount {
    pub number: BigDecimal,
    pub commodity: String,
}

#[derive(Debug, Deserialize)]
pub struct LotRow {
    pub amount: f64,
    pub price_amount: Option<f64>,
    pub price_commodity: Option<String>,
}

pub struct StaticRow {
    pub date: NaiveDate,
    pub account_type: String,
    pub amount: BigDecimal,
    pub commodity: String,
}

pub struct AccountCommodityLot {
    pub account: Account,
    pub datetime: Option<DateTime<Tz>>,
    pub amount: BigDecimal,
    pub price: Option<Amount>,
}

pub struct Operations {
    pub timezone: Tz,
    pub store: Arc<RwLock<Store>>,
}

impl Operations {
    pub fn commodity_prices(&self, commodity: impl AsRef<str>) -> ZhangResult<Vec<PriceDomain>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        Ok(store.prices.iter().filter(|price| price.commodity.eq(commodity)).cloned().collect_vec())
    }
}

impl Operations {
    pub fn commodity_lots(&self, commodity: impl AsRef<str>) -> ZhangResult<Vec<AccountCommodityLot>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        let mut ret = vec![];
        for (account, lots) in store.commodity_lots.iter() {
            for lot in lots.iter() {
                if lot.commodity.eq(commodity) {
                    let lot = lot.clone();
                    ret.push(AccountCommodityLot {
                        account: account.clone(),
                        datetime: lot.datetime,
                        amount: lot.amount,
                        price: lot.price,
                    })
                }
            }
        }
        Ok(ret)
    }
}

impl Operations {}

impl Operations {
    pub fn read(&self) -> RwLockReadGuard<Store> {
        self.store.read().unwrap()
    }
    pub fn write(&self) -> RwLockWriteGuard<Store> {
        self.store.write().unwrap()
    }
}

impl Operations {
    pub(crate) async fn insert_or_update_account(
        &mut self, datetime: DateTime<Tz>, account: Account, status: AccountStatus, alias: Option<&str>,
    ) -> ZhangResult<()> {
        let mut store = self.write();
        let account_domain = store.accounts.entry(account.clone()).or_insert_with(|| AccountDomain {
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
    pub(crate) async fn insert_transaction(
        &mut self, id: &Uuid, sequence: i32, datetime: DateTime<Tz>, flag: Flag, payee: Option<&str>, narration: Option<&str>, tags: Vec<String>,
        links: Vec<String>, span: &SpanInfo,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        store.transactions.insert(
            id.clone(),
            TransactionHeaderDomain {
                id: id.clone(),
                sequence,
                datetime,
                flag,
                payee: payee.map(|it| it.to_owned()),
                narration: narration.map(|it| it.to_owned()),
                span: span.clone(),
                tags,
                links,
            },
        );

        Ok(())
    }

    pub(crate) async fn insert_transaction_posting(
        &mut self, trx_id: &Uuid, account_name: &str, unit: Option<Amount>, cost: Option<Amount>, inferred_amount: Amount, previous_amount: Amount,
        after_amount: Amount,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        let trx = store.transactions.get(trx_id).cloned().expect("cannot find trx");
        store.postings.push(PostingDomain {
            id: Uuid::new_v4(),
            trx_id: trx_id.clone(),
            trx_sequence: trx.sequence,
            trx_datetime: trx.datetime,
            account: Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?,
            unit,
            cost,
            inferred_amount,
            previous_amount,
            after_amount,
        });
        Ok(())
    }

    pub(crate) async fn insert_document(
        &mut self, datetime: DateTime<Tz>, filename: Option<&str>, path: String, document_type: DocumentType,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        store.documents.push(DocumentDomain {
            datetime,
            document_type,
            filename: filename.map(|it| it.to_owned()),
            path,
        });

        Ok(())
    }

    pub(crate) async fn insert_price(&mut self, datetime: DateTime<Tz>, commodity: &str, amount: &BigDecimal, target_commodity: &str) -> ZhangResult<()> {
        let mut store = self.write();
        store.prices.push(PriceDomain {
            datetime: datetime.naive_local(),
            commodity: commodity.to_owned(),
            amount: amount.clone(),
            target_commodity: target_commodity.to_owned(),
        });
        Ok(())
    }

    pub(crate) async fn account_target_day_balance(
        &mut self, account_name: &str, datetime: DateTime<Tz>, currency: &str,
    ) -> ZhangResult<Option<AccountAmount>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;

        let posting: Option<&PostingDomain> = store
            .postings
            .iter()
            .filter(|posting| posting.account.eq(&account))
            .filter(|posting| posting.after_amount.currency.eq(&currency))
            .filter(|posting| posting.trx_datetime.le(&datetime))
            .sorted_by_key(|posting| posting.trx_datetime)
            .rev()
            .next();

        Ok(posting.map(|it| AccountAmount {
            number: it.after_amount.number.clone(),
            commodity: currency.to_owned(),
        }))
    }

    pub(crate) async fn account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>) -> ZhangResult<Option<CommodityLotRecord>> {
        let mut store = self.write();
        let entry = store
            .commodity_lots
            .entry(Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?)
            .or_insert_with(|| vec![]);

        let option = entry
            .iter()
            .filter(|lot| lot.commodity.eq(currency))
            .filter(|lot| lot.price.eq(&price))
            .next()
            .cloned();

        Ok(option)
    }

    pub(crate) async fn account_lot_fifo(&mut self, account_name: &str, currency: &str, price_commodity: &str) -> ZhangResult<Option<CommodityLotRecord>> {
        let mut store = self.write();
        let entry = store
            .commodity_lots
            .entry(Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?)
            .or_insert_with(|| vec![]);

        let option = entry
            .iter()
            .filter(|lot| lot.commodity.eq(currency))
            .filter(|lot| lot.price.as_ref().map(|it| it.currency.as_str()).eq(&Some(price_commodity)))
            .next()
            .cloned();

        Ok(option)
    }
    pub(crate) async fn update_account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>, amount: &BigDecimal) -> ZhangResult<()> {
        let mut store = self.write();
        let entry = store
            .commodity_lots
            .entry(Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?)
            .or_insert_with(|| vec![]);

        let option = entry.iter_mut().filter(|lot| lot.price.eq(&price)).next();
        if let Some(lot) = option {
            lot.amount = amount.clone();
        } else {
            entry.push(CommodityLotRecord {
                commodity: currency.to_owned(),
                datetime: None,
                amount: amount.clone(),
                price,
            })
        }
        Ok(())
    }

    pub(crate) async fn insert_account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>, amount: &BigDecimal) -> ZhangResult<()> {
        let mut store = self.write();
        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;
        let lot_records = store.commodity_lots.entry(account).or_insert_with(|| vec![]);

        lot_records.push(CommodityLotRecord {
            commodity: currency.to_owned(),
            datetime: None,
            amount: amount.clone(),
            price,
        });
        Ok(())
    }

    pub fn get_latest_price(&self, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
        let store = self.read();
        let option = store
            .prices
            .iter()
            .filter(|price| price.commodity.eq(from.as_ref()))
            .filter(|price| price.target_commodity.eq(to.as_ref()))
            .sorted_by_key(|it| it.datetime)
            .rev()
            .next()
            .cloned();
        Ok(option)
    }
    pub fn get_commodity_balances(&self, commodity: impl AsRef<str>) -> ZhangResult<BigDecimal> {
        let mut total = BigDecimal::zero();
        let store = self.read();
        for (account, lots) in store.commodity_lots.iter() {
            if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
                let account_sum: BigDecimal = lots.iter().map(|it| &it.amount).sum();
                total.add_assign(account_sum);
            }
        }
        Ok(total)
    }
}

impl Operations {
    pub async fn options(&mut self) -> ZhangResult<Vec<OptionDomain>> {
        let store = self.read();

        Ok(store.options.clone().into_iter().map(|(key, value)| OptionDomain { key, value }).collect_vec())
    }

    pub async fn option(&mut self, key: impl AsRef<str>) -> ZhangResult<Option<OptionDomain>> {
        let store = self.read();

        Ok(store.options.get(key.as_ref()).map(|value| OptionDomain {
            key: key.as_ref().to_string(),
            value: value.to_owned(),
        }))
    }

    pub async fn accounts_latest_balance(&mut self) -> ZhangResult<Vec<AccountDailyBalanceDomain>> {
        let store = self.read();

        let mut ret: HashMap<Account, IndexMap<Currency, BTreeMap<NaiveDate, Amount>>> = HashMap::new();

        for posting in store.postings.iter().cloned().sorted_by_key(|posting| posting.trx_datetime) {
            let posting: PostingDomain = posting;
            let date = posting.trx_datetime.naive_local().date();

            let account_inventory = ret.entry(posting.account).or_insert_with(|| IndexMap::new());
            let dated_amount = account_inventory
                .entry(posting.after_amount.currency.clone())
                .or_insert_with(|| BTreeMap::new());
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .flat_map(|(account, account_invetory)| {
                account_invetory
                    .into_iter()
                    .map(|(currency, mut dated)| {
                        let (date, amount) = dated.pop_last().expect("");
                        AccountDailyBalanceDomain {
                            date,
                            account: account.name().to_owned(),
                            balance_number: amount.number,
                            balance_commodity: amount.currency,
                        }
                    })
                    .collect_vec()
            })
            .collect_vec())
    }

    pub async fn get_price(&mut self, date: NaiveDateTime, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
        let store = self.read();
        let x = store
            .prices
            .iter()
            .filter(|price| price.commodity.eq(from.as_ref()))
            .filter(|price| price.target_commodity.eq(to.as_ref()))
            .filter(|price| price.datetime.le(&date))
            .sorted_by(|a, b| a.datetime.cmp(&b.datetime))
            .next()
            .cloned();
        Ok(x)
    }

    pub async fn metas(&mut self, type_: MetaType, type_identifier: impl AsRef<str>) -> ZhangResult<Vec<MetaDomain>> {
        let store = self.read();
        Ok(store
            .metas
            .iter()
            .filter(|meta| meta.meta_type.eq(type_.as_ref()))
            .filter(|meta| meta.type_identifier.eq(type_identifier.as_ref()))
            .cloned()
            .collect_vec())
    }

    pub async fn trx_tags(&mut self, trx_id: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let store = self.read();
        let tags = store
            .transactions
            .get(&Uuid::from_str(trx_id.as_ref()).unwrap())
            .map(|it| it.tags.clone())
            .unwrap_or_default();

        Ok(tags)
    }

    pub async fn trx_links(&mut self, trx_id: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let store = self.read();
        let tags = store
            .transactions
            .get(&Uuid::from_str(trx_id.as_ref()).unwrap())
            .map(|it| it.links.clone())
            .unwrap_or_default();

        Ok(tags)
    }

    pub async fn commodity(&mut self, name: &str) -> ZhangResult<Option<CommodityDomain>> {
        let store = self.read();
        Ok(store.commodities.get(name).cloned())
    }

    pub async fn exist_commodity(&mut self, name: &str) -> ZhangResult<bool> {
        Ok(self.commodity(name).await?.is_some())
    }

    pub async fn exist_account(&mut self, name: &str) -> ZhangResult<bool> {
        Ok(self.account(name).await?.is_some())
    }

    pub async fn transaction_counts(&mut self) -> ZhangResult<i64> {
        let store = self.read();
        Ok(store.transactions.len() as i64)
    }

    pub async fn transaction_span(&mut self, id: &str) -> ZhangResult<TransactionInfoDomain> {
        let store = self.read();
        Ok(store
            .transactions
            .get(&Uuid::from_str(id).unwrap())
            .map(|it| TransactionInfoDomain {
                id: it.id.to_string(),
                source_file: it.span.filename.clone().unwrap(),
                span_start: it.span.start,
                span_end: it.span.end,
            })
            .unwrap())
    }

    pub async fn single_account_balances(&mut self, account_name: &str) -> ZhangResult<Vec<AccountBalanceDomain>> {
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

            let dated_amount = ret.entry(posting.after_amount.currency.clone()).or_insert_with(|| BTreeMap::new());
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .map(|(date, mut balance)| {
                let (date, amount) = balance.pop_last().expect("");
                AccountBalanceDomain {
                    datetime: date.and_time(NaiveTime::default()),
                    account: account.name().to_owned(),
                    account_status: AccountStatus::Open,
                    balance_number: amount.number,
                    balance_commodity: amount.currency,
                }
            })
            .collect_vec())
    }

    pub async fn account_journals(&mut self, account: &str) -> ZhangResult<Vec<AccountJournalDomain>> {
        let store = self.read();
        let account = Account::from_str(account).map_err(|_| ZhangError::InvalidAccount)?;

        let mut ret = vec![];
        for posting in store.postings.iter().filter(|posting| posting.account.eq(&account)).cloned().sorted_by(|a, b| {
            a.trx_datetime
                .cmp(&b.trx_datetime)
                .reverse()
                .then(a.trx_sequence.cmp(&b.trx_sequence).reverse())
        }) {
            let posting: PostingDomain = posting;
            let trx_header = store.transactions.get(&posting.trx_id);
            ret.push(AccountJournalDomain {
                datetime: posting.trx_datetime.naive_local(),
                account: posting.account.name().to_owned(),
                trx_id: posting.id.to_string(),
                payee: trx_header.and_then(|it| it.payee.clone()),
                narration: trx_header.and_then(|it| it.narration.clone()),
                inferred_unit_number: posting.inferred_amount.number,
                inferred_unit_commodity: posting.inferred_amount.currency,
                account_after_number: posting.after_amount.number,
                account_after_commodity: posting.after_amount.currency,
            })
        }
        Ok(ret)
    }
    pub async fn account_dated_journals(
        &mut self, account_type: AccountType, from: DateTime<Utc>, to: DateTime<Utc>,
    ) -> ZhangResult<Vec<AccountJournalDomain>> {
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
                account: posting.account.name().to_owned(),
                trx_id: posting.trx_id.to_string(),
                payee: trx.payee,
                narration: trx.narration,
                inferred_unit_number: posting.inferred_amount.number,
                inferred_unit_commodity: posting.inferred_amount.currency,
                account_after_number: posting.after_amount.number,
                account_after_commodity: posting.after_amount.currency,
            })
        }
        Ok(ret)
    }

    pub async fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let store = self.read();
        Ok(store.errors.iter().cloned().collect_vec())
    }

    pub async fn account(&mut self, account_name: &str) -> ZhangResult<Option<AccountDomain>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;
        Ok(store.accounts.get(&account).cloned())
    }
    pub async fn all_open_accounts(&mut self) -> ZhangResult<Vec<AccountDomain>> {
        let store = self.read();
        Ok(store
            .accounts
            .values()
            .filter(|account| account.status == AccountStatus::Open)
            .cloned()
            .collect_vec())
    }
    pub async fn all_accounts(&mut self) -> ZhangResult<Vec<String>> {
        let store = self.read();
        Ok(store.accounts.keys().map(|it| it.name().to_owned()).collect_vec())
    }

    pub async fn all_payees(&mut self) -> ZhangResult<Vec<String>> {
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

    pub async fn static_duration(&mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> ZhangResult<Vec<StaticRow>> {
        let store = self.read();
        let mut cal: HashMap<NaiveDate, HashMap<AccountType, HashMap<Currency, BigDecimal>>> = HashMap::new();

        for posting in store
            .postings
            .iter()
            .filter(|posting| posting.trx_datetime.ge(&from))
            .filter(|posting| posting.trx_datetime.le(&to))
            .cloned()
        {
            let date = posting.trx_datetime.naive_local().date();
            let date_store = cal.entry(date).or_insert_with(|| HashMap::default());
            let account_type_store = date_store.entry(posting.account.account_type).or_insert_with(|| HashMap::new());
            let balance = account_type_store.entry(posting.after_amount.currency).or_insert_with(|| BigDecimal::zero());
            balance.add_assign(&posting.after_amount.number);
        }

        let mut ret = vec![];
        for (date, type_store) in cal {
            for (account_type, currency_store) in type_store {
                for (currency, balance) in currency_store {
                    ret.push(StaticRow {
                        date,
                        account_type: account_type.to_string(),
                        amount: balance,
                        commodity: currency,
                    })
                }
            }
        }
        Ok(ret)
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

            let dated_amount = ret.entry(posting.after_amount.currency.clone()).or_insert_with(|| BTreeMap::new());
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .map(|(date, mut balance)| {
                let (date, amount) = balance.pop_last().expect("");
                AccountBalanceDomain {
                    datetime: date.and_time(NaiveTime::default()),
                    account: account.name().to_owned(),
                    account_status: AccountStatus::Open,
                    balance_number: amount.number,
                    balance_commodity: amount.currency,
                }
            })
            .collect_vec())
    }
}

// for insert and new operations
impl Operations {
    pub async fn new_error(&mut self, error_type: ErrorType, span: &SpanInfo, metas: HashMap<String, String>) -> ZhangResult<()> {
        let mut store = self.write();
        store.errors.push(ErrorDomain {
            id: Uuid::new_v4().to_string(),
            error_type,
            span: Some(span.clone()),
            metas,
        });
        Ok(())
    }

    pub async fn insert_or_update_options(&mut self, key: &str, value: &str) -> ZhangResult<()> {
        let mut store = self.write();

        store.options.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub async fn insert_meta(&mut self, type_: MetaType, type_identifier: impl AsRef<str>, meta: Meta) -> ZhangResult<()> {
        let mut store = self.write();

        for (meta_key, meta_value) in meta.get_flatten() {
            let option = store
                .metas
                .iter_mut()
                .filter(|it| it.type_identifier.eq(type_identifier.as_ref()))
                .filter(|it| it.meta_type.eq(type_.as_ref()))
                .filter(|it| it.key.eq(&meta_key))
                .next();
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

    pub async fn close_account(&mut self, account_name: &str) -> ZhangResult<()> {
        let mut store = self.write();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;
        let option = store.accounts.get_mut(&account);

        if let Some(account) = option {
            account.status = AccountStatus::Close
        }

        Ok(())
    }

    pub async fn insert_commodity(
        &mut self, name: &String, precision: i32, prefix: Option<String>, suffix: Option<String>, rounding: Option<String>,
    ) -> ZhangResult<()> {
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
