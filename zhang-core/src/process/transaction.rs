use std::collections::HashMap;
use std::ops::{Add, AddAssign, Mul, Neg};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use bigdecimal::{BigDecimal, One, Signed, Zero};
use itertools::Itertools;
use log::trace;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Flag, SpanInfo, Transaction};

use crate::constants::TXN_ID;
use crate::domains::schemas::MetaType;
use crate::domains::AccountAmount;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

impl DirectiveProcess for Transaction {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        let mut operations = ledger.operations();
        let id = Uuid::from_span(span);
        let txn_error = operations.check_transaction(self)?;
        if let Some(txn_error) = txn_error {
            let meta = HashMap::of(TXN_ID, id.to_string());
            match txn_error {
                e @ (ErrorKind::TransactionHasMultipleImplicitPosting
                | ErrorKind::TransactionCannotInferTradeAmount
                | ErrorKind::TransactionExplicitPostingHaveMultipleCommodity) => {
                    operations.new_error(e, span, meta)?;
                    return Ok(false);
                }
                ErrorKind::UnbalancedTransaction => {
                    // double check in process method
                }
                e => {
                    operations.new_error(e, span, meta)?;
                }
            }
        }

        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let id = Uuid::from_span(span);
        let txn_error = operations.check_transaction(self)?;

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

        let mut balance_checker = BigDecimal::zero();
        trace!("new balance checker starting with {}", &balance_checker);

        for (posting_idx, txn_posting) in self.txn_postings().into_iter().enumerate() {
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
                posting_idx,
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
            let lot_meta = txn_posting.lot_meta();
            let booking_method = operations
                .typed_meta_value(MetaType::AccountMeta, txn_posting.account_name(), "booking_method")?
                .unwrap_or(ledger.options.default_booking_method);

            // handle implicit posting cost
            if let Some(cost) = lot_meta.cost {
                let mut accr_amount = amount.number.clone();
                loop {
                    let target_lot_record = operations.account_lot_by_meta(
                        &txn_posting.account_name(),
                        &amount.currency,
                        &cost,
                        txn_posting.txn.date.naive_date(),
                        booking_method,
                    )?;
                    let calculated = (&target_lot_record.amount).add(&accr_amount);
                    if !calculated.is_negative() {
                        // the calculated amount is positive, means it is normal case
                        operations.update_account_lot(&txn_posting.account_name(), &target_lot_record, &calculated)?;

                        balance_checker.add_assign(accr_amount.mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one())));
                        trace!("balance checker current value is {}", &balance_checker);
                        break;
                    } else if target_lot_record.amount.is_zero() {
                        // insert error no enough lot record
                        operations.new_error(
                            ErrorKind::NoEnoughCommodityLot,
                            span,
                            HashMap::of(
                                // "original_amount",
                                // target_lot_record.amount.to_string(),
                                "transaction_amount",
                                amount.number.to_string(),
                            ),
                        )?;
                        // persist the calculated result even if there is an error
                        operations.update_account_lot(&txn_posting.account_name(), &target_lot_record, &calculated)?;
                        balance_checker.add_assign(accr_amount.mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one())));
                        trace!("balance checker current value is {}", &balance_checker);
                        break;
                    } else {
                        // if calculated amount is negative, means the matched lots record has no enough amount to do reduction
                        // then set lots record's amount to zero( delete it)
                        operations.update_account_lot(&txn_posting.account_name(), &target_lot_record, &BigDecimal::zero())?;

                        balance_checker.add_assign(
                            (&target_lot_record.amount)
                                .mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one()))
                                .neg(),
                        );
                        trace!("balance checker current value is {}", &balance_checker);
                        // subtract the accr amount
                        accr_amount.add_assign(&target_lot_record.amount);
                    }
                }
            } else {
                // reduction in default lot
                let target_lot_record = operations.default_account_lot(&txn_posting.account_name(), &amount.currency)?;

                operations.update_account_lot(
                    &txn_posting.account_name(),
                    &target_lot_record,
                    &(&target_lot_record.amount).add(&amount.number),
                )?;

                balance_checker.add_assign(&amount.number);
                trace!("balance checker current value is {}", &balance_checker);
            }
        }
        trace!("final balance checker current value is {}, txn_error is {:?}", &balance_checker, &txn_error);
        if txn_error == Some(ErrorKind::UnbalancedTransaction) && !balance_checker.is_zero() {
            operations.new_error(ErrorKind::UnbalancedTransaction, span, HashMap::of(TXN_ID, id.to_string()))?;
        }

        // extract documents from meta
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
