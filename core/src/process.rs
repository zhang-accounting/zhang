use std::collections::HashMap;
use std::ops::{Add, Sub};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::constants::{DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING};
use crate::database::type_ext::big_decimal::ZhangBigDecimal;
use crate::domains::schemas::{AccountStatus, ErrorType, MetaType};
use crate::domains::{AccountAmount, Operations};
use crate::ledger::Ledger;
use crate::store::DocumentType;
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::ZhangResult;
use async_trait::async_trait;
use bigdecimal::{BigDecimal, FromPrimitive, Zero};
use itertools::Itertools;
use log::debug;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::utils::inventory::LotInfo;
use zhang_ast::*;

#[async_trait]
pub(crate) trait DirectiveProcess {
    async fn handler(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let start_time = Instant::now();
        let result = DirectiveProcess::process(self, ledger, span).await;
        let duration = start_time.elapsed();
        debug!("directive process is done in {:?}", duration);
        result
    }
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()>;
}

async fn check_account_existed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations().await;
    let existed = operations.exist_account(account_name).await?;

    if !existed {
        operations
            .new_error(ErrorType::AccountDoesNotExist, span, HashMap::of("account_name", account_name.to_string()))
            .await?;
    }
    Ok(())
}

async fn check_account_closed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations().await;

    let account = operations.account(account_name).await?;
    if let Some(true) = account.map(|it| it.status == AccountStatus::Close) {
        operations
            .new_error(ErrorType::AccountClosed, span, HashMap::of("account_name", account_name.to_string()))
            .await?;
    }
    Ok(())
}

async fn check_commodity_define(commodity_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations().await;
    let existed = operations.exist_commodity(commodity_name).await?;
    if !existed {
        operations
            .new_error(
                ErrorType::CommodityDoesNotDefine,
                span,
                HashMap::of("commodity_name", commodity_name.to_string()),
            )
            .await?;
    }
    Ok(())
}

#[async_trait]
impl DirectiveProcess for Options {
    async fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        let option_value = ledger.options.parse(self.key.as_str(), self.value.as_str(), &mut operations).await?;
        operations.insert_or_update_options(self.key.as_str(), option_value.as_str()).await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Open {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        for currency in &self.commodities {
            check_commodity_define(currency, ledger, span).await?;
        }

        operations
            .insert_or_update_account(
                self.date.to_timezone_datetime(&ledger.options.timezone),
                self.account.clone(),
                AccountStatus::Open,
                self.meta.get_one("alias").map(|it| it.as_str()),
            )
            .await?;

        operations.insert_meta(MetaType::AccountMeta, self.account.name(), self.meta.clone()).await?;

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Close {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        // check if account exist
        check_account_existed(self.account.name(), ledger, span).await?;
        check_account_closed(self.account.name(), ledger, span).await?;

        let balances = operations.single_account_balances(self.account.name()).await?;
        let has_non_zero_balance = balances.into_iter().any(|balance| !balance.balance_number.is_zero());
        if has_non_zero_balance {
            operations.new_error(ErrorType::CloseNonZeroAccount, span, HashMap::default()).await?;
        }
        operations.close_account(self.account.name()).await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Commodity {
    async fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;

        let default_precision = operations.option(KEY_DEFAULT_COMMODITY_PRECISION).await?.map(|it| it.value);
        let default_rounding = operations.option(KEY_DEFAULT_ROUNDING).await?.map(|it| it.value);

        let precision = self
            .meta
            .get_one("precision")
            .map(|it| it.as_str().to_owned())
            .or(default_precision)
            .map(|it| it.as_str().parse::<i32>())
            .transpose()
            .unwrap_or(None)
            .unwrap_or(DEFAULT_COMMODITY_PRECISION);
        let prefix = self.meta.get_one("prefix").map(|it| it.clone().to_plain_string());
        let suffix = self.meta.get_one("suffix").map(|it| it.clone().to_plain_string());
        let rounding = self
            .meta
            .get_one("rounding")
            .map(|it| it.as_str().to_owned())
            .or(default_rounding)
            .map(|it| Rounding::from_str(it.as_str()))
            .transpose()
            .unwrap_or(None);

        operations
            .insert_commodity(&self.currency, precision, prefix, suffix, rounding.map(|it| it.to_string()))
            .await?;
        operations.insert_meta(MetaType::CommodityMeta, &self.currency, self.meta.clone()).await?;

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Transaction {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;

        if self.flag != Some(Flag::BalancePad) && self.flag != Some(Flag::BalanceCheck) && !ledger.is_transaction_balanced(self).await? {
            operations.new_error(ErrorType::TransactionDoesNotBalance, span, HashMap::default()).await?;
        }
        let id = Uuid::from_span(span);
        let sequence = ledger.trx_counter.fetch_add(1, Ordering::Relaxed);
        operations
            .insert_transaction(
                &id,
                sequence,
                self.date.to_timezone_datetime(&ledger.options.timezone),
                self.flag.clone().unwrap_or(Flag::Okay),
                self.payee.as_ref().map(|it| it.as_str()),
                self.narration.as_ref().map(|it| it.as_str()),
                self.tags.iter().cloned().collect_vec(),
                self.links.iter().cloned().collect_vec(),
                &span,
            )
            .await?;

        for txn_posting in self.txn_postings() {
            let inferred_amount = txn_posting.infer_trade_amount().unwrap();

            let option = operations
                .account_target_day_balance(
                    txn_posting.posting.account.name(),
                    self.date.to_timezone_datetime(&ledger.options.timezone),
                    &inferred_amount.currency,
                )
                .await?;

            let previous = option.unwrap_or(AccountAmount {
                number: ZhangBigDecimal(BigDecimal::zero()),
                commodity: inferred_amount.currency.clone(),
            });
            let after_number = (&previous.number.0).add(&inferred_amount.number);

            operations
                .insert_transaction_posting(
                    &id,
                    txn_posting.posting.account.name(),
                    txn_posting.posting.units.clone(),
                    txn_posting.posting.cost.clone(),
                    inferred_amount,
                    Amount::new(previous.number.0, previous.commodity.clone()),
                    Amount::new(after_number, previous.commodity),
                )
                .await?;

            let amount = txn_posting.units().unwrap_or_else(|| txn_posting.infer_trade_amount().unwrap());
            let lot_info = txn_posting.lots().unwrap_or(LotInfo::Fifo);
            lot_add(txn_posting.account_name(), amount, lot_info, &mut operations).await?;
        }
        for document in self.meta.clone().get_flatten().into_iter().filter(|(key, _)| key.eq("document")) {
            let (_, document_file_name) = document;
            let document_path = document_file_name.to_plain_string();
            let document_pathbuf = PathBuf::from(&document_path);
            operations
                .insert_document(
                    self.date.to_timezone_datetime(&ledger.options.timezone),
                    document_pathbuf.file_name().and_then(|it| it.to_str()),
                    document_path,
                    DocumentType::Trx(id.clone()),
                )
                .await?;
        }
        operations.insert_meta(MetaType::TransactionMeta, id.to_string(), self.meta.clone()).await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Balance {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        match self {
            Balance::BalanceCheck(balance_check) => {
                let option = operations
                    .account_target_day_balance(
                        balance_check.account.name(),
                        balance_check.date.to_timezone_datetime(&ledger.options.timezone),
                        &balance_check.amount.currency,
                    )
                    .await?;

                let current_balance_amount = option.map(|it| it.number.0).unwrap_or_else(BigDecimal::zero);

                let distance = Amount::new(
                    (&balance_check.amount.number).sub(&current_balance_amount),
                    balance_check.amount.currency.clone(),
                );
                if !distance.is_zero() {
                    operations
                        .new_error(
                            ErrorType::AccountBalanceCheckError,
                            span,
                            HashMap::of("account_name", balance_check.account.name().to_string()),
                        )
                        .await?;
                }

                check_account_existed(balance_check.account.name(), ledger, span).await?;
                check_account_closed(balance_check.account.name(), ledger, span).await?;

                let mut transformed_trx = Transaction {
                    date: balance_check.date.clone(),
                    flag: Some(Flag::BalanceCheck),
                    payee: Some(ZhangString::quote("Balance Check")),
                    narration: Some(ZhangString::quote(balance_check.account.name())),
                    tags: Default::default(),
                    links: Default::default(),
                    postings: vec![Posting {
                        flag: None,
                        account: balance_check.account.clone(),
                        units: Some(distance),
                        cost: None,
                        cost_date: None,
                        price: None,
                        meta: Default::default(),
                    }],
                    meta: Default::default(),
                };

                transformed_trx.process(ledger, span).await?;
            }
            Balance::BalancePad(balance_pad) => {
                check_account_existed(balance_pad.account.name(), ledger, span).await?;
                check_account_existed(balance_pad.pad.name(), ledger, span).await?;
                check_account_closed(balance_pad.account.name(), ledger, span).await?;
                check_account_closed(balance_pad.pad.name(), ledger, span).await?;

                let option = operations
                    .account_target_day_balance(
                        balance_pad.account.name(),
                        balance_pad.date.to_timezone_datetime(&ledger.options.timezone),
                        &balance_pad.amount.currency,
                    )
                    .await?;

                let current_balance_amount = option.map(|it| it.number.0).unwrap_or_else(BigDecimal::zero);

                let distance = Amount::new((&balance_pad.amount.number).sub(&current_balance_amount), balance_pad.amount.currency.clone());
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

                transformed_trx.process(ledger, span).await?;

                let _neg_distance = distance.neg();
            }
        }

        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Document {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        check_account_existed(self.account.name(), ledger, span).await?;
        check_account_closed(self.account.name(), ledger, span).await?;

        let path = self.filename.clone().to_plain_string();

        let document_pathbuf = PathBuf::from(&path);
        operations
            .insert_document(
                self.date.to_timezone_datetime(&ledger.options.timezone),
                document_pathbuf.file_name().and_then(|it| it.to_str()),
                path,
                DocumentType::Account(self.account.clone()),
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl DirectiveProcess for Price {
    async fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations().await;
        check_commodity_define(&self.currency, ledger, span).await?;
        check_commodity_define(&self.amount.currency, ledger, span).await?;
        operations
            .insert_price(
                self.date.to_timezone_datetime(&ledger.options.timezone),
                &self.currency,
                &self.amount.number,
                &self.amount.currency,
            )
            .await?;

        Ok(())
    }
}

async fn lot_add(account_name: AccountName, amount: Amount, lot_info: LotInfo, operations: &mut Operations) -> ZhangResult<()> {
    match lot_info {
        LotInfo::Lot(target_currency, lot_number) => {
            let price = Amount::new(lot_number, target_currency);

            let lot = operations.account_lot(&account_name, &amount.currency, Some(price.clone())).await?;

            if let Some(lot_row) = lot {
                operations
                    .update_account_lot(&account_name, &amount.currency, Some(price), &lot_row.amount.add(&amount.number))
                    .await?;
            } else {
                operations
                    .insert_account_lot(&account_name, &amount.currency, Some(price.clone()), &amount.number)
                    .await?;
            }
        }
        LotInfo::Fifo => {
            let lot = operations.account_lot_fifo(&account_name, &amount.currency, &amount.currency).await?;
            if let Some(lot) = lot {
                if lot.price.is_some() {
                    // target lot
                    operations
                        .update_account_lot(&account_name, &amount.currency, lot.price, &lot.amount.add(&amount.number))
                        .await?;

                    // todo check negative
                } else {
                    // default lot
                    operations
                        .update_account_lot(&account_name, &amount.currency, None, &lot.amount.add(&amount.number))
                        .await?;
                }
            } else {
                operations.insert_account_lot(&account_name, &amount.currency, None, &amount.number).await?;
            }
        }
        LotInfo::Filo => {
            unimplemented!()
        }
    }

    Ok(())
}
