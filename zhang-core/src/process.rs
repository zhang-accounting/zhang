use std::collections::HashMap;
use std::ops::{Add, Mul, Sub};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::Ordering;

use bigdecimal::{BigDecimal, Zero};
use itertools::Itertools;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::utils::inventory::LotInfo;
use zhang_ast::*;

use crate::constants::{DEFAULT_COMMODITY_PRECISION, DEFAULT_ROUNDING, KEY_DEFAULT_COMMODITY_PRECISION, KEY_DEFAULT_ROUNDING};
use crate::domains::schemas::{AccountStatus, MetaType};
use crate::domains::{AccountAmount, Operations};
use crate::ledger::Ledger;
use crate::store::{BudgetEventType, DocumentType};
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

/// Directive Process is used to handle how a directive be validated, how we process directives and store the result into [Store]
pub(crate) trait DirectiveProcess {
    fn handler(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let should_process = DirectiveProcess::validate(self, ledger, span)?;
        if should_process {
            DirectiveProcess::process(self, ledger, span)
        } else {
            Ok(())
        }
    }

    /// validate method is used to check if the directive is invalid, or should the directive need to emit an error
    /// if the directive is invalid, we need to use [operations::new_error] to emit a new error into [Store]
    /// return `true` if the directive need to execute the [process] method
    fn validate(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<bool> {
        Ok(true)
    }

    /// process method is to handle the directive, and generate the result for storing in [Store]
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()>;
}

fn check_account_existed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();
    let existed = operations.exist_account(account_name)?;

    if !existed {
        operations.new_error(ErrorKind::AccountDoesNotExist, span, HashMap::of("account_name", account_name.to_string()))?;
    }
    Ok(())
}

fn check_account_closed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();

    let account = operations.account(account_name)?;
    if let Some(true) = account.map(|it| it.status == AccountStatus::Close) {
        operations.new_error(ErrorKind::AccountClosed, span, HashMap::of("account_name", account_name.to_string()))?;
    }
    Ok(())
}

fn check_commodity_define(commodity_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();
    let existed = operations.exist_commodity(commodity_name)?;
    if !existed {
        operations.new_error(
            ErrorKind::CommodityDoesNotDefine,
            span,
            HashMap::of("commodity_name", commodity_name.to_string()),
        )?;
    }
    Ok(())
}

impl DirectiveProcess for Options {
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        let option_value = ledger.options.parse(self.key.as_str(), self.value.as_str(), &mut operations, span)?;
        operations.insert_or_update_options(self.key.as_str(), option_value.as_str())?;
        Ok(())
    }
}

impl DirectiveProcess for Open {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        for currency in &self.commodities {
            check_commodity_define(currency, ledger, span)?;
        }
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.insert_or_update_account(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            self.account.clone(),
            AccountStatus::Open,
            self.meta.get_one("alias").map(|it| it.as_str()),
        )?;

        operations.insert_meta(MetaType::AccountMeta, self.account.name(), self.meta.clone())?;

        Ok(())
    }
}

impl DirectiveProcess for Close {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();

        // check if account exist
        check_account_existed(self.account.name(), ledger, span)?;
        check_account_closed(self.account.name(), ledger, span)?;

        let balances = operations.single_account_balances(self.account.name())?;
        let has_non_zero_balance = balances.into_iter().any(|balance| !balance.balance_number.is_zero());
        if has_non_zero_balance {
            operations.new_error(ErrorKind::CloseNonZeroAccount, span, HashMap::default())?;
        }
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.close_account(self.account.name())?;
        Ok(())
    }
}

impl DirectiveProcess for Commodity {
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let ledger_default_precision = operations.option::<i32>(KEY_DEFAULT_COMMODITY_PRECISION)?;
        let ledger_default_rounding = operations.option::<Rounding>(KEY_DEFAULT_ROUNDING)?;

        let precision = self
            .meta
            .get_one("precision")
            .map(|it| it.as_str().to_owned())
            .map(|it| it.as_str().parse::<i32>())
            .transpose()
            .unwrap_or(None)
            .or(ledger_default_precision)
            .unwrap_or(DEFAULT_COMMODITY_PRECISION);
        let prefix = self.meta.get_one("prefix").map(|it| it.clone().to_plain_string());
        let suffix = self.meta.get_one("suffix").map(|it| it.clone().to_plain_string());
        let rounding = self
            .meta
            .get_one("rounding")
            .map(|it| it.as_str().to_owned())
            .map(|it| Rounding::from_str(it.as_str()))
            .transpose()
            .map_err(|_| ZhangError::InvalidOptionValue)?
            .or(ledger_default_rounding)
            .unwrap_or(DEFAULT_ROUNDING);

        operations.insert_commodity(&self.currency, precision, prefix, suffix, rounding)?;
        operations.insert_meta(MetaType::CommodityMeta, &self.currency, self.meta.clone())?;

        Ok(())
    }
}

impl DirectiveProcess for Transaction {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();

        let txn_error = operations.check_transaction(self)?;
        if let Some(txn_error) = txn_error {
            match txn_error {
                e @ (ErrorKind::TransactionHasMultipleImplicitPosting
                | ErrorKind::TransactionCannotInferTradeAmount
                | ErrorKind::TransactionExplicitPostingHaveMultipleCommodity) => {
                    operations.new_error(e, span, HashMap::default())?;
                    return Ok(false);
                }
                e => {
                    operations.new_error(e, span, HashMap::default())?;
                }
            }
        }

        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let id = Uuid::from_span(span);
        let sequence = ledger.trx_counter.fetch_add(1, Ordering::Relaxed);
        operations.insert_transaction(
            &id,
            sequence,
            self.date.to_timezone_datetime(&ledger.options.timezone),
            self.flag.clone().unwrap_or(Flag::Okay),
            self.payee.as_ref().map(|it| it.as_str()),
            self.narration.as_ref().map(|it| it.as_str()),
            self.tags.iter().cloned().collect_vec(),
            self.links.iter().cloned().collect_vec(),
            span,
        )?;

        for txn_posting in self.txn_postings() {
            let inferred_amount = txn_posting.infer_trade_amount().map_err(ZhangError::ProcessError)?;

            let option = operations.account_target_day_balance(
                txn_posting.posting.account.name(),
                self.date.to_timezone_datetime(&ledger.options.timezone),
                &inferred_amount.currency,
            )?;

            let previous = option.unwrap_or(AccountAmount {
                number: BigDecimal::zero(),
                commodity: inferred_amount.currency.clone(),
            });
            let after_number = (&previous.number).add(&inferred_amount.number);

            operations.insert_transaction_posting(
                &id,
                txn_posting.posting.account.name(),
                txn_posting.posting.units.clone(),
                txn_posting.posting.cost.clone(),
                inferred_amount.clone(),
                Amount::new(previous.number, previous.commodity.clone()),
                Amount::new(after_number, previous.commodity),
            )?;

            // budget related
            let budgets_name = operations.get_account_budget(txn_posting.posting.account.name())?;
            for budget in budgets_name {
                let budget_activity_amount = inferred_amount.mul(BigDecimal::from(txn_posting.posting.account.get_account_sign()));
                operations.budget_add_activity(budget, self.date.to_timezone_datetime(&ledger.options.timezone), budget_activity_amount)?;
            }

            let amount = txn_posting.units().unwrap_or(inferred_amount);
            let lot_info = txn_posting.lots().unwrap_or(LotInfo::Fifo);
            lot_add(txn_posting.account_name(), amount, lot_info, &mut operations)?;
        }
        for document in self.meta.clone().get_flatten().into_iter().filter(|(key, _)| key.eq("document")) {
            let (_, document_file_name) = document;
            let document_path = document_file_name.to_plain_string();
            let document_pathbuf = PathBuf::from(&document_path);
            operations.insert_document(
                self.date.to_timezone_datetime(&ledger.options.timezone),
                document_pathbuf.file_name().and_then(|it| it.to_str()),
                document_path,
                DocumentType::Trx(id),
            )?;
        }
        operations.insert_meta(MetaType::TransactionMeta, id.to_string(), self.meta.clone())?;
        Ok(())
    }
}

impl DirectiveProcess for BalancePad {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        check_account_existed(self.account.name(), ledger, span)?;
        check_account_existed(self.pad.name(), ledger, span)?;
        check_account_closed(self.account.name(), ledger, span)?;
        check_account_closed(self.pad.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let option = operations.account_target_day_balance(
            self.account.name(),
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.amount.currency,
        )?;

        let current_balance_amount = option.map(|it| it.number).unwrap_or_else(BigDecimal::zero);

        let distance = Amount::new((&self.amount.number).sub(&current_balance_amount), self.amount.currency.clone());
        let mut transformed_trx = Transaction {
            date: self.date.clone(),
            flag: Some(Flag::BalancePad),
            payee: Some(ZhangString::quote("Balance Pad")),
            narration: Some(ZhangString::quote(format!("pad {} to {}", self.account.name(), self.pad.name()))),
            tags: Default::default(),
            links: Default::default(),
            postings: vec![
                Posting {
                    flag: None,
                    account: self.account.clone(),
                    units: Some(distance.clone()),
                    cost: None,
                    cost_date: None,
                    price: None,
                    comment: None,
                    meta: Default::default(),
                },
                Posting {
                    flag: None,
                    account: self.pad.clone(),
                    units: None,
                    cost: None,
                    cost_date: None,
                    price: None,
                    comment: None,
                    meta: Default::default(),
                },
            ],
            meta: Default::default(),
        };

        transformed_trx.process(ledger, span)?;

        // let neg_distance = distance.neg();
        Ok(())
    }
}

impl DirectiveProcess for BalanceCheck {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        check_account_existed(self.account.name(), ledger, span)?;
        check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        let option = operations.account_target_day_balance(
            self.account.name(),
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.amount.currency,
        )?;

        let current_balance_amount = option.map(|it| it.number).unwrap_or_else(BigDecimal::zero);

        let distance = Amount::new((&self.amount.number).sub(&current_balance_amount), self.amount.currency.clone());
        if !distance.is_zero() {
            operations.new_error(
                ErrorKind::AccountBalanceCheckError,
                span,
                HashMap::of("account_name", self.account.name().to_string()),
            )?;
        }

        let mut transformed_trx = Transaction {
            date: self.date.clone(),
            flag: Some(Flag::BalanceCheck),
            payee: Some(ZhangString::quote("Balance Check")),
            narration: Some(ZhangString::quote(self.account.name())),
            tags: Default::default(),
            links: Default::default(),
            postings: vec![Posting {
                flag: None,
                account: self.account.clone(),
                units: Some(distance),
                cost: None,
                cost_date: None,
                price: None,
                comment: None,
                meta: Default::default(),
            }],
            meta: Default::default(),
        };

        transformed_trx.process(ledger, span)?;
        Ok(())
    }
}

impl DirectiveProcess for Document {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        check_account_existed(self.account.name(), ledger, span)?;
        check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let path = self.filename.clone().to_plain_string();

        let document_pathbuf = PathBuf::from(&path);
        operations.insert_document(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            document_pathbuf.file_name().and_then(|it| it.to_str()),
            path,
            DocumentType::Account(self.account.clone()),
        )?;
        Ok(())
    }
}

impl DirectiveProcess for Price {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        check_commodity_define(&self.currency, ledger, span)?;
        check_commodity_define(&self.amount.currency, ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        operations.insert_price(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.currency,
            &self.amount.number,
            &self.amount.currency,
        )?;

        Ok(())
    }
}

impl DirectiveProcess for Budget {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        if operations.contains_budget(&self.name) {
            operations.new_error(ErrorKind::DefineDuplicatedBudget, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.init_budget(
            &self.name,
            &self.commodity,
            self.date.to_timezone_datetime(&ledger.options.timezone),
            self.meta.get_one("alias").map(|it| it.as_str().to_owned()),
            self.meta.get_one("category").map(|it| it.as_str().to_owned()),
        )?;
        Ok(())
    }
}

impl DirectiveProcess for BudgetAdd {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        if !operations.contains_budget(&self.name) {
            operations.new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.budget_add_assigned_amount(
            &self.name,
            self.date.to_timezone_datetime(&ledger.options.timezone),
            BudgetEventType::AddAssignedAmount,
            self.amount.clone(),
        )?;

        Ok(())
    }
}

impl DirectiveProcess for BudgetTransfer {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        if !operations.contains_budget(&self.from) {
            operations.new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
            return Ok(false);
        };
        if !operations.contains_budget(&self.to) {
            operations.new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
            return Ok(false);
        };

        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.budget_transfer(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.from,
            &self.to,
            self.amount.clone(),
        )?;
        Ok(())
    }
}

impl DirectiveProcess for BudgetClose {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        if !operations.contains_budget(&self.name) {
            operations.new_error(ErrorKind::BudgetDoesNotExist, span, HashMap::default())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        // todo: check if budget is empty

        operations.budget_close(&self.name, self.date.clone())?;
        Ok(())
    }
}

fn lot_add(account_name: AccountName, amount: Amount, lot_info: LotInfo, operations: &mut Operations) -> ZhangResult<()> {
    match lot_info {
        LotInfo::Lot(target_currency, lot_number) => {
            let price = Amount::new(lot_number, target_currency);

            let lot = operations.account_lot(&account_name, &amount.currency, Some(price.clone()))?;

            if let Some(lot_row) = lot {
                operations.update_account_lot(&account_name, &amount.currency, Some(price), &lot_row.amount.add(&amount.number))?;
            } else {
                operations.insert_account_lot(&account_name, &amount.currency, Some(price.clone()), &amount.number)?;
            }
        }
        LotInfo::Fifo => {
            let lot = operations.account_lot(&account_name, &amount.currency, None)?;
            if let Some(lot) = lot {
                if lot.price.is_some() {
                    // target lot
                    operations.update_account_lot(&account_name, &amount.currency, lot.price, &lot.amount.add(&amount.number))?;

                    // todo check negative
                } else {
                    // default lot
                    operations.update_account_lot(&account_name, &amount.currency, None, &lot.amount.add(&amount.number))?;
                }
            } else {
                operations.insert_account_lot(&account_name, &amount.currency, None, &amount.number)?;
            }
        }
        LotInfo::Filo => {
            unimplemented!()
        }
    }

    Ok(())
}
