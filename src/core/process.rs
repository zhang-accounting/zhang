use std::ops::{Add, Sub};
use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use bigdecimal::{BigDecimal, FromPrimitive, Zero};
use chrono::NaiveDateTime;
use serde::Deserialize;
use sqlx::{Acquire, FromRow, SqliteConnection};

use crate::core::amount::Amount;
use crate::core::data::{Balance, Close, Commodity, Document, Open, Options, Posting, Price, Transaction};
use crate::core::database::type_ext::big_decimal::ZhangBigDecimal;
use crate::core::ledger::{Ledger, LedgerError, LedgerErrorType};
use crate::core::models::{Flag, Rounding, ZhangString};
use crate::core::utils::inventory::LotInfo;
use crate::core::utils::span::SpanInfo;
use crate::core::AccountName;
use crate::error::ZhangResult;

#[derive(Debug, Deserialize, FromRow)]
struct AccountAmount {
    number: ZhangBigDecimal,
    commodity: String,
}

pub(crate) struct ProcessContext {
    pub(crate) connection: SqliteConnection,
}

#[async_trait]
pub(crate) trait DirectiveProcess {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()>;
}

async fn check_account_existed(
    account_name: &str, ledger: &mut Ledger, span: &SpanInfo, conn: &mut SqliteConnection,
) -> ZhangResult<()> {
    let existed = sqlx::query("select 1 from accounts where name = $1")
        .bind(account_name)
        .fetch_optional(conn)
        .await?
        .is_some();

    if !existed {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::AccountDoesNotExist {
                account_name: account_name.to_string(),
            },
        })
    }
    Ok(())
}

async fn check_account_closed(
    account_name: &str, ledger: &mut Ledger, span: &SpanInfo, conn: &mut SqliteConnection,
) -> ZhangResult<()> {
    #[derive(FromRow)]
    struct Row {
        status: String,
    }
    let is_account_closed = sqlx::query_as::<_, Row>(r#"select status from accounts where name = $1"#)
        .bind(account_name)
        .fetch_optional(conn)
        .await?
        .map(|it| it.status)
        .map(|status| status.eq("Close"));
    if let Some(true) = is_account_closed {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::AccountClosed {
                account_name: account_name.to_string(),
            },
        })
    }
    Ok(())
}

async fn check_commodity_define(
    commodity_name: &str, ledger: &mut Ledger, span: &SpanInfo, conn: &mut SqliteConnection,
) -> ZhangResult<()> {
    let existed = sqlx::query("select 1 from commodities where name = $1")
        .bind(commodity_name)
        .fetch_optional(conn)
        .await?
        .is_some();
    if !existed {
        ledger.errors.push(LedgerError {
            span: span.clone(),
            error: LedgerErrorType::CommodityDoesNotDefine {
                commodity_name: commodity_name.to_string(),
            },
        })
    }
    Ok(())
}

#[async_trait]
impl DirectiveProcess for Options {
    async fn process(
        &mut self, ledger: &mut Ledger, context: &mut ProcessContext, _span: &SpanInfo,
    ) -> ZhangResult<()> {
        ledger
            .options
            .parse(self.key.as_str(), self.value.as_str(), &mut context.connection)
            .await?;
        ledger
            .configs
            .insert(self.key.clone().to_plain_string(), self.value.clone().to_plain_string());

        sqlx::query(r#"INSERT OR REPLACE INTO options VALUES ($1, $2);"#)
            .bind(self.key.as_str())
            .bind(self.value.as_str())
            .execute(&mut context.connection)
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
            check_commodity_define(currency, ledger, span, &mut _context.connection).await?;
        }

        sqlx::query(r#"INSERT OR REPLACE INTO accounts(date, type, name, status, alias) VALUES ($1, $2, $3, $4, $5);"#)
            .bind(self.date.naive_datetime())
            .bind(self.account.account_type.to_string())
            .bind(self.account.name())
            .bind("Open")
            .bind(self.meta.get_one(&"alias".to_string()).map(|it| it.as_str()))
            .execute(&mut _context.connection)
            .await?;

        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            sqlx::query(r#"INSERT OR REPLACE INTO metas VALUES ($1, $2, $3, $4);"#)
                .bind("AccountMeta")
                .bind(self.account.name())
                .bind(meta_key)
                .bind(meta_value.as_str())
                .execute(&mut _context.connection)
                .await?;
        }

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Close {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        // check if account exist
        check_account_existed(self.account.name(), ledger, span, &mut context.connection).await?;
        check_account_closed(self.account.name(), ledger, span, &mut context.connection).await?;

        sqlx::query(r#"update accounts set status = 'Close' where name = $1"#)
            .bind(self.account.name())
            .execute(&mut context.connection)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Commodity {
    async fn process(
        &mut self, _ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo,
    ) -> ZhangResult<()> {
        let precision = self
            .meta
            .get_one(&"precision".to_string())
            .map(|it| it.as_str().parse::<i32>())
            .transpose()
            .unwrap_or(None);
        let prefix = self.meta.get_one(&"prefix".to_string()).map(|it| it.as_str());
        let suffix = self.meta.get_one(&"suffix".to_string()).map(|it| it.as_str());
        let rounding = self
            .meta
            .get_one(&"rounding".to_string())
            .map(|it| Rounding::from_str(it.as_str()))
            .transpose()
            .unwrap_or(None);

        sqlx::query(
            r#"INSERT OR REPLACE INTO commodities (name, precision, prefix, suffix, rounding)
                        VALUES ($1, $2, $3, $4, $5);"#,
        )
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

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Transaction {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        let id = uuid::Uuid::new_v4().to_string();
        if !ledger.is_transaction_balanced(self).await? {
            ledger.errors.push(LedgerError {
                span: span.clone(),
                error: LedgerErrorType::TransactionDoesNotBalance,
            });
        }

        sqlx::query(r#"INSERT INTO transactions (id, datetime, type, payee, narration, source_file, span_start, span_end)VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#)
            .bind(&id)
            .bind(self.date.naive_datetime())
            .bind(self.flag.clone().unwrap_or(Flag::Okay).to_string())
            .bind(self.payee.as_ref().map(|it| it.as_str()))
            .bind(self.narration.as_ref().map(|it| it.as_str()))
            .bind(span.filename.as_ref().and_then(|it|it.to_str()).map(|it|it.to_string()))
            .bind(span.start as i64)
            .bind(span.end as i64)
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

        for txn_posting in self.txn_postings() {
            let inferred_amount = txn_posting.infer_trade_amount().unwrap();

            let option: Option<AccountAmount> = sqlx::query_as(
                r#"select account_after_number as number, account_after_commodity as commodity from transaction_postings
                                join transactions on transactions.id = transaction_postings.trx_id
                                where account = $1 and "datetime" <=  $2 and account_after_commodity = $3
                                order by "datetime" desc, transactions.sequence desc limit 1"#,
            )
            .bind(txn_posting.posting.account.name())
            .bind(self.date.naive_datetime())
            .bind(&inferred_amount.currency)
            .fetch_optional(&mut context.connection)
            .await?;
            let previous = option.unwrap_or(AccountAmount {
                number: ZhangBigDecimal(BigDecimal::zero()),
                commodity: inferred_amount.currency.clone(),
            });

            let after_number = (&previous.number.0).add(&inferred_amount.number);

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
            let amount = txn_posting
                .units()
                .unwrap_or_else(|| txn_posting.infer_trade_amount().unwrap());
            let lot_info = txn_posting.lots().unwrap_or(LotInfo::Fifo);
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
            sqlx::query(
                r#"INSERT INTO documents (datetime, filename, path, extension, trx_id) VALUES ($1, $2, $3, $4, $5);"#,
            )
            .bind(self.date.naive_datetime())
            .bind(document_pathbuf.file_name().and_then(|it| it.to_str()).unwrap())
            .bind(&document_path)
            .bind(extension)
            .bind(&id)
            .execute(&mut context.connection)
            .await?;
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
                let option: Option<AccountAmount> = sqlx::query_as(
                    r#"
                    select account_after_number as number, account_after_commodity as commodity
                    from transactions
                             join transaction_postings on transactions.id = transaction_postings.trx_id
                    where account = $1
                      and datetime <= $2 and account_after_commodity = $3
                    order by datetime desc
                "#,
                )
                .bind(balance_check.account.name())
                .bind(balance_check.date.naive_datetime())
                .bind(&balance_check.amount.currency)
                .fetch_optional(&mut context.connection)
                .await?;
                let current_balance_amount = option.map(|it| it.number.0).unwrap_or_else(BigDecimal::zero);

                    let distance = Amount::new(
                        (&balance_check.amount.number).sub(&current_balance_amount),
                        balance_check.amount.currency.clone(),
                    );
                if !distance.is_zero() {

                    ledger.errors.push(LedgerError {
                        span: span.clone(),
                        error: LedgerErrorType::AccountBalanceCheckError {
                            account_name: balance_check.account.name().to_string(),
                            target: balance_check.amount.clone(),

                            current: Amount::new(current_balance_amount, balance_check.amount.currency.clone()),
                            distance: distance.clone(),
                        },
                    });
                }

                check_account_existed(balance_check.account.name(), ledger, span, &mut context.connection).await?;
                check_account_closed(balance_check.account.name(), ledger, span, &mut context.connection).await?;


                let mut transformed_trx = Transaction {
                    date: balance_check.date.clone(),
                    flag: Some(Flag::BalancePad),
                    payee: Some(ZhangString::quote("Balance Check")),
                    narration: Some(ZhangString::quote(format!(
                        "Check balance of {}",
                        balance_check.account.name()
                    ))),
                    tags: Default::default(),
                    links: Default::default(),
                    postings: vec![
                        Posting {
                            flag: None,
                            account: balance_check.account.clone(),
                            units: Some(distance),
                            cost: None,
                            cost_date: None,
                            price: None,
                            meta: Default::default(),
                        }
                    ],
                    meta: Default::default(),
                };

                transformed_trx.process(ledger, context, span).await?;

            }
            Balance::BalancePad(balance_pad) => {
                check_account_existed(balance_pad.account.name(), ledger, span, &mut context.connection).await?;
                check_account_existed(balance_pad.pad.name(), ledger, span, &mut context.connection).await?;
                check_account_closed(balance_pad.account.name(), ledger, span, &mut context.connection).await?;
                check_account_closed(balance_pad.pad.name(), ledger, span, &mut context.connection).await?;

                let option: Option<AccountAmount> = sqlx::query_as(
                    r#"
                    select account_after_number as number, account_after_commodity as commodity
                    from transactions
                             join transaction_postings on transactions.id = transaction_postings.trx_id
                    where account = $1
                      and datetime <= $2 and account_after_commodity = $3
                    order by datetime desc
                "#,
                )
                .bind(balance_pad.account.name())
                .bind(balance_pad.date.naive_datetime())
                .bind(&balance_pad.amount.currency)
                .fetch_optional(&mut context.connection)
                .await?;
                let current_balance_amount = option.map(|it| it.number.0).unwrap_or_else(BigDecimal::zero);

                let distance = Amount::new(
                    (&balance_pad.amount.number).sub(&current_balance_amount),
                    balance_pad.amount.currency.clone(),
                );
                let mut transformed_trx = Transaction {
                    date: balance_pad.date.clone(),
                    flag: Some(Flag::BalancePad),
                    payee: Some(ZhangString::quote("Balance Pad")),
                    narration: Some(ZhangString::quote(format!(
                        "pad {} to {}",
                        balance_pad.account.name(),
                        balance_pad.pad.name()
                    ))),
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
                            meta: Default::default(),
                        },
                        Posting {
                            flag: None,
                            account: balance_pad.pad.clone(),
                            units: None,
                            cost: None,
                            cost_date: None,
                            price: None,
                            meta: Default::default(),
                        },
                    ],
                    meta: Default::default(),
                };

                transformed_trx.process(ledger, context, span).await?;

                let _neg_distance = distance.neg();
            }
        }

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Document {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        check_account_existed(self.account.name(), ledger, span, &mut context.connection).await?;
        check_account_closed(self.account.name(), ledger, span, &mut context.connection).await?;

        let path = self.filename.clone().to_plain_string();

        let document_pathbuf = PathBuf::from(&path);
        let extension = document_pathbuf.extension().and_then(|it| it.to_str());
        sqlx::query(
            r#"INSERT INTO documents (datetime, filename, path, extension, account) VALUES ($1, $2, $3, $4, $5);"#,
        )
        .bind(self.date.naive_datetime())
        .bind(document_pathbuf.file_name().and_then(|it| it.to_str()).unwrap())
        .bind(&path)
        .bind(extension)
        .bind(self.account.name())
        .execute(&mut context.connection)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Price {
    async fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        check_commodity_define(&self.currency, ledger, span, &mut context.connection).await?;
        check_commodity_define(&self.amount.currency, ledger, span, &mut context.connection).await?;
        sqlx::query(r#"INSERT INTO prices (datetime, commodity, amount, target_commodity)VALUES ($1, $2, $3, $4)"#)
            .bind(self.date.naive_datetime())
            .bind(&self.currency)
            .bind(&self.amount.number.to_string())
            .bind(&self.amount.currency)
            .execute(&mut context.connection)
            .await?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, FromRow)]
struct LotRow {
    amount: f64,
    price_amount: Option<f64>,
    price_commodity: Option<String>,
}

async fn lot_add(
    account_name: AccountName, amount: Amount, lot_info: LotInfo, conn: &mut SqliteConnection,
) -> ZhangResult<()> {
    let mut trx = conn.begin().await?;
    match lot_info {
        LotInfo::Lot(target_currency, lot_number) => {
            let lot: Option<LotRow> = sqlx::query_as(
                r#"
            select amount, price_amount, price_commodity
            from commodity_lots
            where account = $1 and commodity = $2 and price_amount = $3 and price_commodity = $4"#,
            )
            .bind(&account_name)
            .bind(&amount.currency)
            .bind(lot_number.to_string())
            .bind(&target_currency)
            .fetch_optional(&mut trx)
            .await?;

            if let Some(lot_row) = lot {
                sqlx::query(
                    r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount = $4 and price_commodity = $5"#,
                )
                .bind(
                    BigDecimal::from_f64(lot_row.amount)
                        .expect("error")
                        .add(&amount.number)
                        .to_string(),
                )
                .bind(&account_name)
                .bind(&amount.currency)
                .bind(lot_number.to_string())
                .bind(&target_currency)
                .execute(&mut trx)
                .await?;
            } else {
                sqlx::query(
                    r#"INSERT INTO commodity_lots (account, commodity, datetime, amount, price_amount, price_commodity)
                                    VALUES ($1, $2, $3, $4, $5, $6)"#,
                )
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
            let lot: Option<LotRow> = sqlx::query_as(
                r#"
                select amount, price_amount, price_commodity
                from commodity_lots
                where account = $1 and commodity = $2
                  and (price_commodity = $3 or price_commodity is null)
                  and ((amount != 0 and price_amount is not null) or price_amount is null)
                order by datetime desc
            "#,
            )
            .bind(&account_name)
            .bind(&amount.currency)
            .bind(&amount.currency)
            .fetch_optional(&mut trx)
            .await?;
            if let Some(lot) = lot {
                if lot.price_amount.is_some() {
                    // target lot
                    sqlx::query(
                        r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount = $4 and price_commodity = $5"#,
                    )
                    .bind(
                        BigDecimal::from_f64(lot.amount)
                            .expect("error")
                            .add(&amount.number)
                            .to_string(),
                    )
                    .bind(&account_name)
                    .bind(&amount.currency)
                    .bind(lot.price_amount)
                    .bind(&lot.price_commodity)
                    .execute(&mut trx)
                    .await?;

                    // todo check negative
                } else {
                    // default lot
                    sqlx::query(
                        r#"update commodity_lots
                        set amount = $1
                        where account = $2 and commodity = $3  and price_amount is NULL and price_commodity is NULL"#,
                    )
                    .bind(
                        BigDecimal::from_f64(lot.amount)
                            .expect("error")
                            .add(&amount.number)
                            .to_string(),
                    )
                    .bind(&account_name)
                    .bind(&amount.currency)
                    .execute(&mut trx)
                    .await?;
                }
            } else {
                sqlx::query(
                    r#"INSERT INTO commodity_lots (account, commodity, datetime, amount, price_amount, price_commodity)
                                    VALUES ($1, $2, $3, $4, $5, $6)"#,
                )
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
