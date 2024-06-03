use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::{Add, AddAssign, Sub};
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::utils::inventory::{BookingMethod, LotMeta};
use zhang_ast::{Account, AccountType, Currency, Date, Flag, Meta, Rounding, SpanInfo, Transaction};

use crate::domains::schemas::{
    AccountBalanceDomain, AccountDailyBalanceDomain, AccountDomain, AccountJournalDomain, AccountStatus, CommodityDomain, ErrorDomain, MetaDomain, MetaType,
    OptionDomain, PriceDomain, TransactionInfoDomain,
};
use crate::store::{
    BudgetDomain, BudgetEvent, BudgetEventType, BudgetIntervalDetail, CommodityLotRecord, DocumentDomain, DocumentType, PostingDomain, Store, TransactionDomain,
};
use crate::utils::bigdecimal_ext::BigDecimalExt;
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

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
    /// single commodity prices
    pub fn commodity_prices(&self, commodity: impl AsRef<str>) -> ZhangResult<Vec<PriceDomain>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        Ok(store.prices.iter().filter(|price| price.commodity.eq(commodity)).cloned().collect_vec())
    }
}

impl Operations {
    /// single commodity lots
    pub fn commodity_lots(&self, commodity: impl AsRef<str>, tz: &Tz) -> ZhangResult<Vec<AccountCommodityLot>> {
        let store = self.read();
        let commodity = commodity.as_ref();
        let mut ret = vec![];
        for (account, lots) in store.commodity_lots.iter() {
            for lot in lots.iter() {
                if lot.commodity.eq(commodity) {
                    let lot = lot.clone();
                    ret.push(AccountCommodityLot {
                        account: Account::from_str(account).map_err(|_| ZhangError::InvalidAccount)?,
                        datetime: lot
                            .acquisition_date
                            .map(|it| tz.from_local_datetime(&it.and_hms_opt(0, 0, 0).unwrap()).unwrap()),
                        amount: lot.amount,
                        price: lot.price,
                    })
                }
            }
        }
        Ok(ret)
    }
}

impl Operations {
    pub fn read(&self) -> RwLockReadGuard<Store> {
        self.store.read().expect("poison lock detect")
    }
    pub fn write<'a>(&'a self) -> RwLockWriteGuard<'a, Store> {
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

    /// check whether transaction is valid or not, return the ErrorKind of the issue
    pub(crate) fn check_transaction(&self, txn: &Transaction) -> ZhangResult<Option<ErrorKind>> {
        if txn.flag == Some(Flag::BalanceCheck) {
            return Ok(None);
        }
        match txn.get_postings_inventory() {
            Ok(inventory) => {
                for (currency, amount) in inventory.currencies.iter() {
                    let commodity = self.commodity(currency)?;
                    let Some(commodity) = commodity else {
                        return Ok(Some(ErrorKind::CommodityDoesNotDefine));
                    };
                    let precision = commodity.precision;
                    let rounding = commodity.rounding;
                    let decimal = amount.total.round_with(precision as i64, rounding.is_up());
                    if !decimal.is_zero() {
                        return Ok(Some(ErrorKind::UnbalancedTransaction));
                    }
                }
                Ok(None)
            }
            Err(e) => Ok(Some(e)),
        }
    }

    /// insert transaction postings
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_transaction_posting(
        &mut self, trx_id: &Uuid, posting_idx: usize, account_name: &str, unit: Option<Amount>, cost: Option<Amount>, inferred_amount: Amount,
        previous_amount: Amount, after_amount: Amount,
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
            cost,
            inferred_amount,
            previous_amount,
            after_amount,
        };
        store.postings.push(posting.clone());
        let txn_header = store
            .transactions
            .get_mut(trx_id)
            .expect("invalid context: cannot find txn header when inserting postings");
        txn_header.postings.push(posting);
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

    pub(crate) fn account_target_day_balance(&mut self, account_name: &str, datetime: DateTime<Tz>, currency: &str) -> ZhangResult<Option<AccountAmount>> {
        let store = self.read();

        let account = Account::from_str(account_name).map_err(|_| ZhangError::InvalidAccount)?;

        let posting: Option<&PostingDomain> = store
            .postings
            .iter()
            .filter(|posting| posting.account.eq(&account))
            .filter(|posting| posting.after_amount.currency.eq(&currency))
            .filter(|posting| posting.trx_datetime.le(&datetime))
            .sorted_by_key(|posting| posting.trx_datetime)
            .next_back();

        Ok(posting.map(|it| AccountAmount {
            number: it.after_amount.number.clone(),
            commodity: currency.to_owned(),
        }))
    }

    pub(crate) fn account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>) -> ZhangResult<Option<CommodityLotRecord>> {
        let mut store = self.write();
        let entry = store.commodity_lots.entry(account_name.to_owned()).or_default();

        let option = entry.iter().filter(|lot| lot.commodity.eq(currency)).find(|lot| lot.price.eq(&price)).cloned();

        Ok(option)
    }

    pub(crate) fn account_lot_by_meta<'a>(
        &'a mut self, account_name: &str, currency: &str, lot_meta: LotMeta, booking_method: BookingMethod,
    ) -> ZhangResult<CommodityLotRecord> {
        let mut store = self.write();
        let entry = store.commodity_lots.entry(account_name.to_owned()).or_default();

        let mut option = entry
            .iter()
            // match commodity
            .filter(|lot| lot.commodity.eq(currency))
            // match cost, works with empty cost
            .filter(|it| it.cost.eq(&lot_meta.cost))
            // match cost date
            .filter(|it| {
                if let Some(cost_date) = lot_meta.cost_date {
                    // if cost date in lot meta is defined, filter lots with cost date
                    it.acquisition_date.eq(&Some(cost_date))
                } else {
                    // if cost date in meta is null, return all lots
                    true
                }
            })
            .filter(|lot| lot.price.eq(&lot_meta.price));

        let lot_record = match booking_method {
            BookingMethod::FIFO => option.next().cloned(),
            BookingMethod::LIFO => option.rev().next().cloned(),
            BookingMethod::AVERAGE => {
                unimplemented!()
            }
            BookingMethod::AVERAGE_ONLY => {
                unimplemented!()
            }
            BookingMethod::STRICT => {
                unimplemented!()
            }
            BookingMethod::NONE => {
                unimplemented!()
            }
        };
        if let Some(record) = lot_record {
            Ok(record)
        } else {
            // if target lot record does not exist, insert a new one and return it
            let new_lot_record = CommodityLotRecord {
                commodity: currency.to_owned(),
                amount: BigDecimal::zero(),
                cost: lot_meta.cost,
                acquisition_date: lot_meta.cost_date,
                price: lot_meta.price,
            };
            entry.push(new_lot_record.clone());
            Ok(new_lot_record)
        }
    }

    /// todo remove fifo method, and extract the generic method for booking method
    pub fn account_lot_fifo(&mut self, account_name: &str, currency: &str, price_commodity: &str) -> ZhangResult<Option<CommodityLotRecord>> {
        let mut store = self.write();
        let entry = store.commodity_lots.entry(account_name.to_owned()).or_default();

        let option = entry
            .iter()
            .filter(|lot| lot.commodity.eq(currency))
            .find(|lot| lot.price.as_ref().map(|it| it.currency.as_str()).eq(&Some(price_commodity)))
            .cloned();

        Ok(option)
    }

    // todo: remove it
    pub(crate) fn update_account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>, amount: &BigDecimal) -> ZhangResult<()> {
        let mut store = self.write();
        let entry = store.commodity_lots.entry(account_name.to_owned()).or_default();

        let option = entry.iter_mut().find(|lot| lot.price.eq(&price));
        if let Some(lot) = option {
            lot.amount = amount.clone();
        } else {
            entry.push(CommodityLotRecord {
                commodity: currency.to_owned(),
                acquisition_date: None,
                amount: amount.clone(),
                cost: None,
                price,
            })
        }
        Ok(())
    }

    // todo: rename it
    pub(crate) fn update_account_lot_2(&mut self, account_name: &str, lot_record: &CommodityLotRecord, amount: &BigDecimal) -> ZhangResult<()> {
        let mut store = self.write();
        let entry = store.commodity_lots.entry(account_name.to_owned()).or_default();

        let option = entry.iter_mut().find(|lot| lot.eq(&lot_record));

        if let Some(lot) = option {
            lot.amount = amount.clone();
        }
        Ok(())
    }

    pub(crate) fn insert_account_lot(&mut self, account_name: &str, currency: &str, price: Option<Amount>, amount: &BigDecimal) -> ZhangResult<()> {
        let mut store = self.write();
        let lot_records = store.commodity_lots.entry(account_name.to_owned()).or_default();

        lot_records.push(CommodityLotRecord {
            commodity: currency.to_owned(),
            acquisition_date: None,
            amount: amount.clone(),
            cost: None,
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
            let dated_amount = account_inventory.entry(posting.after_amount.currency.clone()).or_default();
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret
            .into_iter()
            .flat_map(|(account, account_invetory)| {
                account_invetory
                    .into_iter()
                    .map(|(_, mut dated)| {
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

    pub fn get_price(&mut self, date: NaiveDateTime, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
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
    pub fn typed_meta_value<T>(&self, type_: MetaType, type_identifier: impl AsRef<str>, key: impl AsRef<str>) -> ZhangResult<Option<T>>
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
            .map_err(|e| ZhangError::ProcessError(e))
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

            let dated_amount = ret.entry(posting.after_amount.currency.clone()).or_default();
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
                    balance_number: amount.number,
                    balance_commodity: amount.currency,
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

            let dated_amount = ret.entry(posting.after_amount.currency.clone()).or_default();
            dated_amount.insert(date, posting.after_amount);
        }

        Ok(ret)
    }

    pub fn account_journals(&mut self, account: &str) -> ZhangResult<Vec<AccountJournalDomain>> {
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
                timestamp: posting.trx_datetime.timestamp(),
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
                inferred_unit_number: posting.inferred_amount.number,
                inferred_unit_commodity: posting.inferred_amount.currency,
                account_after_number: posting.after_amount.number,
                account_after_commodity: posting.after_amount.currency,
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
                inferred_unit_number: posting.inferred_amount.number,
                inferred_unit_commodity: posting.inferred_amount.currency,
                account_after_number: posting.after_amount.number,
                account_after_commodity: posting.after_amount.currency,
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

    pub fn static_duration(&mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> ZhangResult<Vec<StaticRow>> {
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
            let date_store = cal.entry(date).or_default();
            let account_type_store = date_store.entry(posting.account.account_type).or_default();
            let balance = account_type_store.entry(posting.after_amount.currency).or_insert_with(BigDecimal::zero);
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

            let dated_amount = ret.entry(posting.after_amount.currency.clone()).or_default();
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
                    balance_number: amount.number,
                    balance_commodity: amount.currency,
                }
            })
            .collect_vec())
    }
}

// for insert and new operations
impl Operations {
    pub fn new_error(&mut self, error_kind: ErrorKind, span: &SpanInfo, metas: HashMap<String, String>) -> ZhangResult<()> {
        let mut store = self.write();
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

    /// get target month's detail
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

    /// add amount to target month's budget
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

    /// close budget
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
