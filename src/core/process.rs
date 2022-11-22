use crate::core::amount::Amount;
use crate::core::data::{Balance, Close, Commodity, Document, Open, Options, Posting, Price, Transaction};
use crate::core::ledger::{
    AccountInfo, AccountStatus, CurrencyInfo, DocumentType, Ledger, LedgerError, LedgerErrorType,
};
use crate::core::models::{Directive, Flag, Rounding, SingleTotalPrice, ZhangString};
use crate::core::utils::inventory::{DailyAccountInventory, Inventory, LotInfo};
use crate::core::utils::latest_map::LatestMap;
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::utils::span::SpanInfo;
use crate::core::AccountName;
use crate::error::ZhangResult;
use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::HashMap;
use std::fmt::format;
use std::ops::{Add, Neg, Sub};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock as StdRwLock};
use bigdecimal::{BigDecimal, FromPrimitive, Num, Zero};
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, SqliteConnection};
use sqlx::FromRow;
use crate::core::models::DirectiveType::Plugin;

#[derive(Debug, Deserialize, FromRow)]
struct AccountAmount {
    number: f64,
    commodity: String,
}

pub(crate) struct ProcessContext {
    pub(crate) target_day: Option<NaiveDate>,
    pub(crate) prices: Arc<StdRwLock<DatedPriceGrip>>,
    pub(crate) connection: SqliteConnection,
}

impl ProcessContext {
    pub fn default_account_snapshot(&self) -> Inventory {
        Inventory {
            currencies: Default::default(),
            prices: self.prices.clone(),
        }
    }
}

#[async_trait]
pub(crate) trait DirectiveProcess {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()>;
}

fn record_daily_snapshot(
    snapshot: &mut HashMap<AccountName, Inventory>, daily_snapshot: &mut DailyAccountInventory,
    target_day: &mut Option<NaiveDate>, date: NaiveDate,
) {
    if let Some(target_day_inner) = target_day {
        if date.ne(target_day_inner) {
            daily_snapshot.insert_account_inventory(*target_day_inner, snapshot.clone());
            *target_day = Some(date);
        }
    } else {
        *target_day = Some(date);
    }
}

fn check_account_existed(ledger: &mut Ledger, span: &SpanInfo, account_name: &str) {
    if !ledger.accounts.contains_key(account_name) {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::AccountDoesNotExist {
                account_name: account_name.to_string(),
            },
        })
    }
}

fn check_account_closed(ledger: &mut Ledger, span: &SpanInfo, account_name: &str) {
    let has_account_closed = ledger
        .accounts
        .get(account_name)
        .map(|account_info| account_info.status)
        .map(|status| status == AccountStatus::Close);
    if let Some(true) = has_account_closed {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::AccountClosed {
                account_name: account_name.to_string(),
            },
        })
    }
}

fn check_commodity_define(ledger: &mut Ledger, span: &SpanInfo, commodity_name: &str) {
    let has_commodity_defined = !ledger.currencies.contains_key(commodity_name);
    if has_commodity_defined {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::CommodityDoesNotDefine {
                commodity_name: commodity_name.to_string(),
            },
        })
    }
}

fn check_commodity_define_for_amount<'a>(ledger: &mut Ledger, span: &SpanInfo, amount: impl Into<Option<&'a Amount>>) {
    if let Some(amount) = amount.into() {
        let has_commodity_defined = !ledger.currencies.contains_key(&amount.currency);
        if has_commodity_defined {
            ledger.errors.push(LedgerError {
                span: span.clone(),
                error: LedgerErrorType::CommodityDoesNotDefine {
                    commodity_name: amount.currency.to_string(),
                },
            })
        }
    }
}

#[async_trait]
impl DirectiveProcess for Options {
    async fn process(
        &mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo,
    ) -> ZhangResult<()> {
        ledger.options.parse(self.key.as_str(), self.value.as_str());
        ledger
            .configs
            .insert(self.key.clone().to_plain_string(), self.value.clone().to_plain_string());
        sqlx::query(r#"INSERT OR REPLACE INTO options VALUES ($1, $2);"#)
            .bind(self.key.as_str())
            .bind(self.value.as_str())
            .execute(&mut _context.connection)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Open {
    async fn process(
        &mut self, ledger: &mut Ledger, _context: &mut ProcessContext, span: &SpanInfo,
    ) -> ZhangResult<()> {
        for currency in &self.commodities {
            check_commodity_define(ledger, span, currency);
        }

        let account_info = ledger
            .accounts
            .entry(self.account.content.to_string())
            .or_insert_with(|| AccountInfo {
                currencies: Default::default(),
                status: AccountStatus::Open,
                meta: Default::default(),
            });

        sqlx::query(r#"INSERT OR REPLACE INTO accounts VALUES ($1, $2, $3, $4);"#)
            .bind(self.date.naive_datetime())
            .bind(self.account.name())
            .bind("Open")
            .bind(self.meta.get_one(&"alias".to_string()).map(|it| it.as_str()))
            .execute(&mut _context.connection)
            .await?;
        account_info.status = AccountStatus::Open;
        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            account_info.meta.insert(meta_key.clone(), meta_value.as_str().to_string());
            sqlx::query(r#"INSERT OR REPLACE INTO metas VALUES ($1, $2, $3, $4);"#)
                .bind("AccountMeta")
                .bind(self.account.name())
                .bind(meta_key)
                .bind(meta_value.as_str())
                .execute(&mut _context.connection)
                .await?;
        }
        for currency in &self.commodities {
            account_info.currencies.insert(currency.to_string());
        }
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Close {
    async fn process(
        &mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo,
    ) -> ZhangResult<()> {
        // check if account exist
        check_account_existed(ledger, _span, self.account.name());
        check_account_closed(ledger, _span, self.account.name());

        sqlx::query(r#"update accounts set status = 'Close' where name = '$1'"#)
            .bind(self.account.name())
            .execute(&mut _context.connection)
            .await?;
        let account_info = ledger
            .accounts
            .entry(self.account.content.to_string())
            .or_insert_with(|| AccountInfo {
                currencies: Default::default(),
                status: AccountStatus::Open,
                meta: Default::default(),
            });
        account_info.status = AccountStatus::Close;
        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            account_info.meta.insert(meta_key, meta_value.to_plain_string());
        }
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Commodity {
    async fn process(
        &mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo,
    ) -> ZhangResult<()> {
        let precision = self
            .meta
            .get_one(&"precision".to_string())
            .map(|it| it.as_str().parse::<i32>())
            .transpose()
            .unwrap_or(None);
        let prefix = self
            .meta
            .get_one(&"prefix".to_string()).map(|it| it.as_str());
        let suffix = self
            .meta
            .get_one(&"suffix".to_string()).map(|it| it.as_str());
        let rounding = self
            .meta
            .get_one(&"rounding".to_string())
            .map(|it| Rounding::from_str(it.as_str()))
            .transpose()
            .unwrap_or(None);

        sqlx::query(r#"INSERT INTO commodities (name, precision, prefix, suffix, rounding)
                        VALUES ($1, $2, $3, $4, $5);"#)
            .bind(&self.currency)
            .bind(precision)
            .bind(prefix)
            .bind(suffix)
            .bind(rounding.map(|it| it.to_string()))
            .execute(&mut _context.connection)
            .await?;

        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            sqlx::query(r#"INSERT OR REPLACE INTO metas VALUES ($1, $2, $3, $4);"#)
                .bind("CommodityMeta")
                .bind(self.currency.as_str())
                .bind(meta_key)
                .bind(meta_value.as_str())
                .execute(&mut _context.connection)
                .await?;
        }
        ledger
            .currencies
            .entry(self.currency.to_string())
            .and_modify(|target| {
                target.commodity = self.clone();
                target.precision = precision;
                target.rounding = rounding;
            })
            .or_insert_with(|| CurrencyInfo {
                commodity: self.clone(),
                precision,
                rounding,
                prices: LatestMap::default(),
            });
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Transaction {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        let id = uuid::Uuid::new_v4().to_string();
        if !ledger.is_transaction_balanced(self) {
            ledger.errors.push(LedgerError {
                span: span.clone(),
                error: LedgerErrorType::TransactionDoesNotBalance,
            });
        }

        sqlx::query(r#"INSERT INTO transactions (id, datetime, type, payee, narration)VALUES ($1, $2, $3, $4, $5)"#)
            .bind(&id)
            .bind(self.date.naive_datetime())
            .bind(self.flag.clone().unwrap_or(Flag::Okay).to_string())
            .bind(self.payee.as_ref().map(|it| it.as_str()))
            .bind(self.narration.as_ref().map(|it| it.as_str()))
            .execute(&mut context.connection)
            .await?;

        for tag in self.tags.iter() {
            sqlx::query(r#"INSERT INTO transaction_tags (trx_id, tag)VALUES ($1, $2)"#)
                .bind(&id)
                .bind(tag)
                .execute(&mut context.connection)
                .await?;
        }
        for link in self.links.iter() {
            sqlx::query(r#"INSERT INTO transaction_links (trx_id, link)VALUES ($1, $2)"#)
                .bind(&id)
                .bind(link)
                .execute(&mut context.connection)
                .await?;
        }

        let date = self.date.naive_date();
        record_daily_snapshot(
            &mut ledger.account_inventory,
            &mut ledger.daily_inventory,
            &mut context.target_day,
            date,
        );
        for txn_posting in self.txn_postings() {
            let inferred_amount = txn_posting.infer_trade_amount().unwrap();


            let option: Option<AccountAmount> = sqlx::query_as(r#"select account_after_number as number, account_after_commodity as commodity from transaction_postings
                                join transactions on transactions.id = transaction_postings.trx_id
                                where account = $1 and "datetime" <=  $2 and account_after_commodity = $3
                                order by "datetime" desc limit 1"#)
                .bind(txn_posting.posting.account.name())
                .bind(self.date.naive_datetime())
                .bind(&inferred_amount.currency)
                .fetch_optional(&mut context.connection)
                .await?;
            let previous = option.unwrap_or(AccountAmount { number: 0f64, commodity: inferred_amount.currency.clone() });

            let after_number = BigDecimal::from_f64(previous.number).unwrap().add(&inferred_amount.number);


            sqlx::query(r#"INSERT INTO transaction_postings
                               (trx_id, account, unit_number, unit_commodity, cost_number, cost_commodity, inferred_unit_number, inferred_unit_commodity,
                                account_before_number, account_before_commodity, account_after_number, account_after_commodity
                               )
                               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#)
                .bind(&id)
                .bind(txn_posting.posting.account.name())
                .bind(txn_posting.posting.units.as_ref().map(|it| it.number.to_string()))
                .bind(txn_posting.posting.units.as_ref().map(|it| &it.currency))
                .bind(txn_posting.posting.cost.as_ref().map(|it| it.number.to_string()))
                .bind(txn_posting.posting.cost.as_ref().map(|it| &it.currency))
                .bind(inferred_amount.number.to_string())
                .bind(&inferred_amount.currency)
                .bind(&previous.number)
                .bind(&previous.commodity)
                .bind(after_number.to_string())
                .bind(&previous.commodity)

                .execute(&mut context.connection)
                .await?;

            check_account_existed(ledger, span, txn_posting.posting.account.name());
            check_account_closed(ledger, span, txn_posting.posting.account.name());
            check_commodity_define_for_amount(ledger, span, &txn_posting.posting.units);
            if let Some(price) = txn_posting.posting.price.as_ref() {
                match price {
                    SingleTotalPrice::Single(single) => check_commodity_define_for_amount(ledger, span, single),
                    SingleTotalPrice::Total(total_price) => {
                        check_commodity_define_for_amount(ledger, span, total_price)
                    }
                }
            }
            check_commodity_define_for_amount(ledger, span, &txn_posting.posting.cost);
            let target_account_snapshot = ledger
                .account_inventory
                .entry(txn_posting.account_name())
                .or_insert_with(|| context.default_account_snapshot());
            let amount = txn_posting
                .units()
                .unwrap_or_else(|| txn_posting.infer_trade_amount().unwrap());
            let lot_info = txn_posting.lots().unwrap_or(LotInfo::Fifo);

            target_account_snapshot.add_lot(amount.clone(), lot_info.clone());
            ledger.inventory.add_lot(amount.clone(), lot_info.clone());


            lot_add(txn_posting.account_name(), amount, lot_info, &mut context.connection).await?;
        }
        for document in self
            .meta
            .clone()
            .get_flatten()
            .into_iter()
            .filter(|(key, _)| key.eq("document"))
        {
            let (_, document_file_name) = document;
            let document_path = document_file_name.to_plain_string();
            let document_pathbuf = PathBuf::from(&document_path);
            let extension = document_pathbuf.extension().and_then(|it| it.to_str());
            sqlx::query(r#"INSERT INTO documents (datetime, filename, path, extension, trx_id) VALUES ($1, $2, $3, $4, $5);"#)
                .bind(self.date.naive_datetime())
                .bind(&document_pathbuf.file_name().and_then(|it| it.to_str()).unwrap())
                .bind(&document_path)
                .bind(&extension)
                .bind(&id)
                .execute(&mut context.connection)
                .await?;
            ledger.documents.push(DocumentType::TransactionDocument {
                date: self.date.clone(),
                trx: self.clone(),
                filename: document_path,
            })
        }

        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            sqlx::query(r#"INSERT OR REPLACE INTO metas VALUES ($1, $2, $3, $4);"#)
                .bind("TransactionMeta")
                .bind(&id)
                .bind(meta_key)
                .bind(meta_value.as_str())
                .execute(&mut context.connection)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Balance {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        match self {
            Balance::BalanceCheck(balance_check) => {
                let option: Option<AccountAmount> = sqlx::query_as(r#"
                    select account_after_number as number, account_after_commodity as commodity
                    from transactions
                             join transaction_postings on transactions.id = transaction_postings.trx_id
                    where account = $1
                      and datetime <= $2 and account_after_commodity = $3
                    order by datetime desc
                "#)
                    .bind(balance_check.account.name())
                    .bind(balance_check.date.naive_datetime())
                    .bind(&balance_check.amount.currency)
                    .fetch_optional(&mut context.connection)
                    .await?;
                let current_balance_amount = option.map(|it| it.number).unwrap_or(0f64);
                let current_balance_amount = BigDecimal::from_f64(current_balance_amount).unwrap();

                if current_balance_amount.ne(&balance_check.amount.number) {
                    let distance = Amount::new(
                        (&balance_check.amount.number).sub(&current_balance_amount),
                        balance_check.amount.currency.clone(),
                    );

                    ledger.errors.push(LedgerError {
                        span: span.clone(),
                        error: LedgerErrorType::AccountBalanceCheckError {
                            account_name: balance_check.account.name().to_string(),
                            target: balance_check.amount.clone(),

                            current: Amount::new(current_balance_amount.clone(), balance_check.amount.currency.clone()),
                            distance: distance.clone(),
                        },
                    });
                }


                check_account_existed(ledger, span, balance_check.account.name());
                check_account_closed(ledger, span, balance_check.account.name());
                record_daily_snapshot(
                    &mut ledger.account_inventory,
                    &mut ledger.daily_inventory,
                    &mut context.target_day,
                    balance_check.date.naive_date(),
                );

                let target_account_snapshot = ledger
                    .account_inventory
                    .entry(balance_check.account.name().to_string())
                    .or_insert_with(|| context.default_account_snapshot());

                let target_account_balance = target_account_snapshot.get_total(&balance_check.amount.currency);
                balance_check.current_amount = Some(Amount::new(
                    target_account_balance.clone(),
                    balance_check.amount.currency.clone(),
                ));
                if target_account_balance.ne(&balance_check.amount.number) {
                    let distance = Amount::new(
                        (&balance_check.amount.number).sub(&target_account_balance),
                        balance_check.amount.currency.clone(),
                    );
                    balance_check.distance = Some(distance.clone());

                    ledger.errors.push(LedgerError {
                        span: span.clone(),
                        error: LedgerErrorType::AccountBalanceCheckError {
                            account_name: balance_check.account.name().to_string(),
                            target: Amount::new(
                                balance_check.amount.number.clone(),
                                balance_check.amount.currency.clone(),
                            ),
                            current: Amount::new(target_account_balance, balance_check.amount.currency.clone()),
                            distance: distance.clone(),
                        },
                    });
                    target_account_snapshot.add_lot(distance.clone(), LotInfo::Fifo);
                    ledger.inventory.add_lot(distance, LotInfo::Fifo);
                }
            }
            Balance::BalancePad(balance_pad) => {
                check_account_existed(ledger, span, balance_pad.account.name());
                check_account_existed(ledger, span, balance_pad.pad.name());
                check_account_closed(ledger, span, balance_pad.account.name());
                check_account_closed(ledger, span, balance_pad.pad.name());


                let option: Option<AccountAmount> = sqlx::query_as(r#"
                    select account_after_number as number, account_after_commodity as commodity
                    from transactions
                             join transaction_postings on transactions.id = transaction_postings.trx_id
                    where account = $1
                      and datetime <= $2 and account_after_commodity = $3
                    order by datetime desc
                "#)
                    .bind(balance_pad.account.name())
                    .bind(balance_pad.date.naive_datetime())
                    .bind(&balance_pad.amount.currency)
                    .fetch_optional(&mut context.connection)
                    .await?;
                let current_balance_amount = option.map(|it| it.number).unwrap_or(0f64);
                let current_balance_amount = BigDecimal::from_f64(current_balance_amount).unwrap();

                let distance = Amount::new(
                    (&balance_pad.amount.number).sub(&current_balance_amount),
                    balance_pad.amount.currency.clone(),
                );
                let mut transformed_trx = Transaction {
                    date: balance_pad.date.clone(),
                    flag: Some(Flag::BalancePad),
                    payee: Some(ZhangString::quote("Balance Pad")),
                    narration: Some(ZhangString::quote(format!("pad {} to {}", balance_pad.account.name(), balance_pad.pad.name()))),
                    tags: Default::default(),
                    links: Default::default(),
                    postings: vec![
                        Posting {
                            flag: None,
                            account: balance_pad.account.clone(),
                            units: Some(distance.clone()),
                            cost: None,
                            cost_date: None,
                            price: None,
                            meta: Default::default()
                        },
                        Posting {
                            flag: None,
                            account: balance_pad.pad.clone(),
                            units: None,
                            cost: None,
                            cost_date: None,
                            price: None,
                            meta: Default::default()
                        }
                    ],
                    meta: Default::default(),
                };

                transformed_trx.process(ledger, context, span).await?;


                record_daily_snapshot(
                    &mut ledger.account_inventory,
                    &mut ledger.daily_inventory,
                    &mut context.target_day,
                    balance_pad.date.naive_date(),
                );

                let target_account_snapshot = ledger
                    .account_inventory
                    .entry(balance_pad.account.name().to_string())
                    .or_insert_with(|| context.default_account_snapshot());

                let source_amount = target_account_snapshot.get_total(&balance_pad.amount.currency);
                let source_target_amount = &balance_pad.amount.number;
                // source account
                let distance = source_target_amount.sub(source_amount);
                let neg_distance = (&distance).neg();
                target_account_snapshot.add_lot(
                    Amount::new(distance.clone(), balance_pad.amount.currency.clone()),
                    LotInfo::Fifo,
                );
                ledger.inventory.add_lot(
                    Amount::new(distance, balance_pad.amount.currency.clone()),
                    LotInfo::Fifo,
                );

                // add to pad
                let pad_account_snapshot = ledger
                    .account_inventory
                    .entry(balance_pad.pad.name().to_string())
                    .or_insert_with(|| context.default_account_snapshot());
                pad_account_snapshot.add_lot(
                    Amount::new(neg_distance.clone(), balance_pad.amount.currency.clone()),
                    LotInfo::Fifo,
                );
                ledger.inventory.add_lot(
                    Amount::new(neg_distance, balance_pad.amount.currency.clone()),
                    LotInfo::Fifo,
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Document {
    async fn process(
        &mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo,
    ) -> ZhangResult<()> {
        check_account_existed(ledger, span, self.account.name());
        check_account_closed(ledger, span, self.account.name());

        let path = self.filename.clone().to_plain_string();

        let document_pathbuf = PathBuf::from(&path);
        let extension = document_pathbuf.extension().and_then(|it| it.to_str());
        sqlx::query(r#"INSERT INTO documents (datetime, filename, path, extension, account) VALUES ($1, $2, $3, $4, $5);"#)
            .bind(self.date.naive_datetime())
            .bind(&document_pathbuf.file_name().and_then(|it| it.to_str()).unwrap())
            .bind(&path)
            .bind(&extension)
            .bind(self.account.name())
            .execute(&mut context.connection)
            .await?;

        ledger.documents.push(DocumentType::AccountDocument {
            date: self.date.clone(),
            account: self.account.clone(),
            filename: path,
        });
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Price {
    async fn process(
        &mut self, ledger: &mut Ledger, _context: &mut ProcessContext, span: &SpanInfo,
    ) -> ZhangResult<()> {
        check_commodity_define(ledger, span, &self.currency);
        check_commodity_define(ledger, span, &self.amount.currency);
        sqlx::query(r#"INSERT INTO prices (datetime, commodity, amount, target_commodity)VALUES ($1, $2, $3, $4)"#)
            .bind(self.date.naive_datetime())
            .bind(&self.currency)
            .bind(&self.amount.number.to_string())
            .bind(&self.amount.currency)
            .execute(&mut _context.connection)
            .await?;

        let mut result = ledger.prices.write().unwrap();
        result.insert(
            self.date.naive_datetime(),
            self.currency.clone(),
            self.amount.currency.clone(),
            self.amount.number.clone(),
        );
        let option = ledger.currencies.get_mut(&self.currency);
        if let Some(currency_info) = option {
            let price_group = currency_info.prices.data.entry(self.date.naive_date()).or_default();
            price_group.insert(self.amount.currency.clone(), self.amount.number.clone());
        }
        Ok(())
    }
}


#[derive(Debug, Deserialize, FromRow)]
struct LotRow {
    amount: f64,
    price_amount: Option<f64>,
    price_commodity: Option<String>,
}

async fn lot_add(account_name: AccountName, amount: Amount, lot_info: LotInfo, conn: &mut SqliteConnection) -> ZhangResult<()> {
    let mut trx = conn.begin().await?;
    match lot_info {
        LotInfo::Lot(target_currency, lot_number) => {
            let lot: Option<LotRow> = sqlx::query_as(r#"
            select amount, price_amount, price_commodity
            from commodity_lots
            where account = $1 and commodity = $2 and price_amount = $3 and price_commodity = $4"#)
                .bind(&account_name)
                .bind(&amount.currency)
                .bind(lot_number.to_string())
                .bind(&target_currency)
                .fetch_optional(&mut trx)
                .await?;

            if let Some(lot_row) = lot {
                sqlx::query(r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount = $4 and price_commodity = $5"#)
                    .bind(BigDecimal::from_f64(lot_row.amount).expect("error").add(&amount.number).to_string())
                    .bind(&account_name)
                    .bind(&amount.currency)
                    .bind(lot_number.to_string())
                    .bind(&target_currency)
                    .execute(&mut trx)
                    .await?;
            } else {
                sqlx::query(r#"INSERT INTO commodity_lots (account, commodity, datetime, amount, price_amount, price_commodity)
                                    VALUES ($1, $2, $3, $4, $5, $6)"#)
                    .bind(&account_name)
                    .bind(&amount.currency)
                    .bind(None::<NaiveDateTime>)
                    .bind(amount.number.to_string())
                    .bind(lot_number.to_string())
                    .bind(&target_currency)
                    .execute(&mut trx)
                    .await?;
            }
        }
        LotInfo::Fifo => {
            let lot: Option<LotRow> = sqlx::query_as(r#"
                select amount, price_amount, price_commodity
                from commodity_lots
                where account = $1 and commodity = $2
                  and (price_commodity = $3 or price_commodity is null)
                  and ((amount != 0 and price_amount is not null) or price_amount is null)
                order by datetime desc
            "#)
                .bind(&account_name)
                .bind(&amount.currency)
                .bind(&amount.currency)
                .fetch_optional(&mut trx)
                .await?;
            if let Some(lot) = lot {
                if let Some(price) = lot.price_amount {
                    // target lot
                    sqlx::query(r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount = $4 and price_commodity = $5"#)
                        .bind(BigDecimal::from_f64(lot.amount).expect("error").add(&amount.number).to_string())
                        .bind(&account_name)
                        .bind(&amount.currency)
                        .bind(&lot.price_amount)
                        .bind(&lot.price_commodity)
                        .execute(&mut trx)
                        .await?;

                    // todo check negative
                } else {
                    // default lot
                    sqlx::query(r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount is NULL and price_commodity is NULL"#)
                        .bind(BigDecimal::from_f64(lot.amount).expect("error").add(&amount.number).to_string())
                        .bind(&account_name)
                        .bind(&amount.currency)
                        .execute(&mut trx)
                        .await?;
                }
            } else {
                sqlx::query(r#"INSERT INTO commodity_lots (account, commodity, datetime, amount, price_amount, price_commodity)
                                    VALUES ($1, $2, $3, $4, $5, $6)"#)
                    .bind(&account_name)
                    .bind(&amount.currency)
                    .bind(None::<NaiveDateTime>)
                    .bind(amount.number.to_string())
                    .bind(None::<f64>)
                    .bind(None::<String>)
                    .execute(&mut trx)
                    .await?;
            }
        }
        LotInfo::Filo => {
            unimplemented!()
        }
    }
    trx.commit().await?;

    Ok(())
}