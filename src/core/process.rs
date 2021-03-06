use crate::core::amount::Amount;
use crate::core::data::{Balance, Close, Commodity, Document, Open, Options, Price, Transaction};
use crate::core::ledger::{
    AccountInfo, AccountStatus, CurrencyInfo, DocumentType, Ledger, LedgerError, LedgerErrorType,
};
use crate::core::utils::inventory::{DailyAccountInventory, Inventory};
use crate::core::utils::price_grip::DatedPriceGrip;
use crate::core::utils::span::SpanInfo;
use crate::core::AccountName;
use crate::error::ZhangResult;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::ops::{Neg, Sub};
use std::sync::{Arc, RwLock as StdRwLock};

pub(crate) struct ProcessContext {
    pub(crate) target_day: Option<NaiveDate>,
    pub(crate) prices: Arc<StdRwLock<DatedPriceGrip>>,
}

impl ProcessContext {
    pub fn default_account_snapshot(&self) -> Inventory {
        Inventory {
            inner: Default::default(),
            prices: self.prices.clone(),
        }
    }
}

pub(crate) trait DirectiveProcess {
    fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()>;
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

impl DirectiveProcess for Options {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo) -> ZhangResult<()> {
        ledger
            .configs
            .insert(self.key.clone().to_plain_string(), self.value.clone().to_plain_string());
        Ok(())
    }
}

impl DirectiveProcess for Open {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo) -> ZhangResult<()> {
        let account_info = ledger
            .accounts
            .entry(self.account.content.to_string())
            .or_insert_with(|| AccountInfo {
                currencies: Default::default(),
                status: AccountStatus::Open,
                meta: Default::default(),
            });
        account_info.status = AccountStatus::Open;
        for (meta_key, meta_value) in self.meta.clone().get_flatten() {
            account_info.meta.insert(meta_key, meta_value.to_plain_string());
        }
        for currency in &self.commodities {
            account_info.currencies.insert(currency.to_string());
        }
        Ok(())
    }
}

impl DirectiveProcess for Close {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo) -> ZhangResult<()> {
        // check if account exist
        check_account_existed(ledger, _span, self.account.name());
        check_account_closed(ledger, _span, self.account.name());
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

impl DirectiveProcess for Commodity {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo) -> ZhangResult<()> {
        let _target_currency = ledger
            .currencies
            .entry(self.currency.to_string())
            .or_insert_with(|| CurrencyInfo {
                commodity: self.clone(),
                prices: HashMap::new(),
            });
        Ok(())
    }
}

impl DirectiveProcess for Transaction {
    fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        if !self.is_balance() {
            ledger.errors.push(LedgerError {
                span: span.clone(),
                error: LedgerErrorType::TransactionDoesNotBalance,
            });
        }
        let date = self.date.naive_date();
        record_daily_snapshot(
            &mut ledger.account_inventory,
            &mut ledger.daily_inventory,
            &mut context.target_day,
            date,
        );
        for txn_posting in self.txn_postings() {
            check_account_existed(ledger, span, txn_posting.posting.account.name());
            check_account_closed(ledger, span, txn_posting.posting.account.name());
            let target_account_snapshot = ledger
                .account_inventory
                .entry(txn_posting.account_name())
                .or_insert_with(|| context.default_account_snapshot());
            target_account_snapshot.add_amount(txn_posting.units());
        }
        for document in self
            .meta
            .clone()
            .get_flatten()
            .into_iter()
            .filter(|(key, _)| key.eq("document"))
        {
            let (_, document_file_name) = document;
            ledger.documents.push(DocumentType::TransactionDocument {
                date: self.date.clone(),
                trx: self.clone(),
                filename: document_file_name.to_plain_string(),
            })
        }
        Ok(())
    }
}

impl DirectiveProcess for Balance {
    fn process(&mut self, ledger: &mut Ledger, context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        match self {
            Balance::BalanceCheck(balance_check) => {
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

                let target_account_balance = target_account_snapshot.get(&balance_check.amount.currency);
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
                    target_account_snapshot.add_amount(distance);
                }
            }
            Balance::BalancePad(balance_pad) => {
                check_account_existed(ledger, span, balance_pad.account.name());
                check_account_existed(ledger, span, balance_pad.pad.name());
                check_account_closed(ledger, span, balance_pad.account.name());
                check_account_closed(ledger, span, balance_pad.pad.name());
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

                let source_amount = target_account_snapshot.get(&balance_pad.amount.currency);
                let source_target_amount = &balance_pad.amount.number;
                // source account
                let distance = source_target_amount.sub(source_amount);
                let neg_distance = (&distance).neg();
                target_account_snapshot.add_amount(Amount::new(distance, balance_pad.amount.currency.clone()));

                // add to pad
                let pad_account_snapshot = ledger
                    .account_inventory
                    .entry(balance_pad.pad.name().to_string())
                    .or_insert_with(|| context.default_account_snapshot());
                pad_account_snapshot.add_amount(Amount::new(neg_distance, balance_pad.amount.currency.clone()));
            }
        }

        Ok(())
    }
}

impl DirectiveProcess for Document {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, span: &SpanInfo) -> ZhangResult<()> {
        check_account_existed(ledger, span, self.account.name());
        check_account_closed(ledger, span, self.account.name());
        ledger.documents.push(DocumentType::AccountDocument {
            date: self.date.clone(),
            account: self.account.clone(),
            filename: self.filename.clone().to_plain_string(),
        });
        Ok(())
    }
}

impl DirectiveProcess for Price {
    fn process(&mut self, ledger: &mut Ledger, _context: &mut ProcessContext, _span: &SpanInfo) -> ZhangResult<()> {
        let mut result = ledger.prices.write().unwrap();
        result.insert(
            self.date.naive_datetime(),
            self.currency.clone(),
            self.amount.currency.clone(),
            self.amount.number.clone(),
        );
        Ok(())
    }
}
