use crate::constants::KEY_DEFAULT_COMMODITY_PRECISION;
use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use crate::domains::schemas::{
    AccountBalanceDomain, AccountDailyBalanceDomain, AccountDomain, AccountJournalDomain, AccountStatus, CommodityDomain, ErrorDomain, ErrorType, MetaDomain,
    MetaType, OptionDomain, PriceDomain, TransactionInfoDomain,
};
use crate::store::{CommodityLotRecord, DocumentDomain, DocumentType, PostingDomain, Store, TransactionHeaderDomain};
use crate::{ZhangError, ZhangResult};
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use itertools::Itertools;
use serde::Deserialize;
use sqlx::pool::PoolConnection;
use sqlx::{Acquire, FromRow, Sqlite};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, Meta, SpanInfo};

pub mod schemas;

#[derive(FromRow)]
struct ValueRow {
    value: String,
}

#[derive(Debug, Deserialize, FromRow)]
pub struct AccountAmount {
    pub number: ZhangBigDecimal,
    pub commodity: String,
}

#[derive(Debug, Deserialize, FromRow)]
pub struct LotRow {
    pub amount: f64,
    pub price_amount: Option<f64>,
    pub price_commodity: Option<String>,
}

#[derive(FromRow)]
pub struct StaticRow {
    pub date: NaiveDate,
    pub account_type: String,
    pub amount: ZhangBigDecimal,
    pub commodity: String,
}

pub struct Operations {
    #[cfg(feature = "sqlite")]
    pub(crate) pool: PoolConnection<Sqlite>,
    pub timezone: Tz,
    pub store: Arc<RwLock<Store>>,
}

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
        &mut self, id: &Uuid, datetime: DateTime<Tz>, flag: Flag, payee: Option<&str>, narration: Option<&str>, tags: Vec<String>, links: Vec<String>,
        span: &SpanInfo,
    ) -> ZhangResult<()> {
        let mut store = self.write();

        store.transactions.insert(
            id.clone(),
            TransactionHeaderDomain {
                id: id.clone(),
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

        let time = store.transactions.get(trx_id).map(|it| it.datetime.clone()).expect("cannot find trx");
        store.postings.push(PostingDomain {
            id: Uuid::new_v4(),
            trx_id: trx_id.clone(),
            trx_datetime: time,
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
        let conn = self.pool.acquire().await?;
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
            amount: ZhangBigDecimal(amount.clone()),
            target_commodity: target_commodity.to_owned(),
        });
        Ok(())
    }

    pub(crate) async fn account_target_day_balance(
        &mut self, account_name: &str, datetime: DateTime<Tz>, currency: &str,
    ) -> ZhangResult<Option<AccountAmount>> {
        let conn = self.pool.acquire().await?;

        let option: Option<AccountAmount> = sqlx::query_as(
            r#"select account_after_number as number, account_after_commodity as commodity from transaction_postings
                                join transactions on transactions.id = transaction_postings.trx_id
                                where account = $1 and "datetime" <=  $2 and account_after_commodity = $3
                                order by "datetime" desc, transactions.sequence desc limit 1"#,
        )
        .bind(account_name)
        .bind(datetime)
        .bind(currency)
        .fetch_optional(conn)
        .await?;
        Ok(option)
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
            .filter(|lot| lot.price.as_ref().map(|it|it.currency.as_str()).eq(&Some(price_commodity)))
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
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountDailyBalanceDomain>(
            r#"
                SELECT
                    date(datetime) AS date,
                    account,
                    balance_number,
                    balance_commodity
                FROM
                    account_daily_balance
                GROUP BY
                    account
                HAVING
                    max(datetime)
            "#,
        )
        .fetch_all(conn)
        .await?)
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
        let conn = self.pool.acquire().await?;
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
        let conn = self.pool.acquire().await?;
        let store = self.read();
        Ok(store.transactions.len() as i64)
    }

    pub async fn transaction_span(&mut self, id: &str) -> ZhangResult<TransactionInfoDomain> {
        let conn = self.pool.acquire().await?;

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

    pub async fn account_balances(&mut self) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountBalanceDomain>(
            r#"
                        select datetime, account, account_status, balance_number, balance_commodity
                        from account_balance
            "#,
        )
        .fetch_all(conn)
        .await?)
    }

    pub async fn single_account_balances(&mut self, account_name: &str) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountBalanceDomain>(
            r#"
                select datetime, account, account_status, balance_number, balance_commodity
                from account_balance
                where account = $1
            "#,
        )
        .bind(account_name)
        .fetch_all(conn)
        .await?)
    }

    pub async fn account_journals(&mut self, account: &str) -> ZhangResult<Vec<AccountJournalDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountJournalDomain>(
            r#"
                    select datetime,
                           trx_id,
                           account,
                           payee,
                           narration,
                           inferred_unit_number,
                           inferred_unit_commodity,
                           account_after_number,
                           account_after_commodity
                    from transaction_postings
                             join transactions on transactions.id = transaction_postings.trx_id
                    where account = $1
                    order by datetime desc, transactions.sequence desc
            "#,
        )
        .bind(account)
        .fetch_all(conn)
        .await?)
    }
    pub async fn account_dated_journals(&mut self, account_type: &str, from: NaiveDateTime, to: NaiveDateTime) -> ZhangResult<Vec<AccountJournalDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountJournalDomain>(
            r#"
                select datetime,
                       trx_id,
                       account,
                       payee,
                       narration,
                       inferred_unit_number,
                       inferred_unit_commodity,
                       account_after_number,
                       account_after_commodity
                from transaction_postings
                         join transactions on transactions.id = transaction_postings.trx_id
                         join accounts on accounts.name = transaction_postings.account
                where datetime >= $1
                  and datetime <= $2
                  and accounts.type = $3
            "#,
        )
        .bind(from)
        .bind(to)
        .bind(account_type)
        .fetch_all(conn)
        .await?)
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
        let conn = self.pool.acquire().await?;
        Ok(
            sqlx::query_as::<_, AccountDomain>(r#"select date, type, name, status, alias from accounts WHERE status = 'Open'"#)
                .fetch_all(conn)
                .await?,
        )
    }
    pub async fn all_accounts(&mut self) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;
        let accounts = sqlx::query_as::<_, ValueRow>("select name as value from accounts")
            .fetch_all(conn)
            .await?
            .into_iter()
            .map(|it| it.value)
            .collect_vec();
        Ok(accounts)
    }

    pub async fn all_payees(&mut self) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;

        #[derive(FromRow)]
        struct PayeeRow {
            payee: String,
        }
        let payees = sqlx::query_as::<_, PayeeRow>(
            r#"
        select distinct payee from transactions
        "#,
        )
        .fetch_all(conn)
        .await?;
        Ok(payees.into_iter().map(|it| it.payee).filter(|it| !it.is_empty()).collect_vec())
    }

    pub async fn static_duration(&mut self, from: NaiveDateTime, to: NaiveDateTime) -> ZhangResult<Vec<StaticRow>> {
        let conn = self.pool.acquire().await?;
        let rows = sqlx::query_as::<_, StaticRow>(
            r#"
        SELECT
            date(datetime) AS date,
            accounts.type AS account_type,
            sum(inferred_unit_number) AS amount,
            inferred_unit_commodity AS commodity
        FROM
            transaction_postings
            JOIN transactions ON transactions.id = transaction_postings.trx_id
            JOIN accounts ON accounts.name = transaction_postings.account
            where transactions.datetime >= $1 and transactions.datetime <= $2
        GROUP BY
            date(datetime),
            accounts.type,
            inferred_unit_commodity
    "#,
        )
        .bind(from)
        .bind(to)
        .fetch_all(conn)
        .await?;

        Ok(rows)
    }
}

// for insert and new operations
impl Operations {
    pub async fn new_error(&mut self, error_type: ErrorType, span: &SpanInfo, metas: HashMap<String, String>) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
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
