use std::collections::HashMap;
use std::ops::{Add, AddAssign, Mul};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use bigdecimal::{BigDecimal, Zero};
use itertools::Itertools;
use log::trace;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Flag, SpanInfo, Transaction};

use crate::constants::TXN_ID;
use crate::domains::schemas::MetaType;
use crate::inventory::TransactionInference;
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
        trace!("new balance checker starting with {}", balance_checker);

        for (posting_idx, txn_posting) in self.txn_postings().into_iter().enumerate() {
            let inferred_amount = txn_posting.units().unwrap_or(
                txn_posting
                    .infer_trade_amount()
                    .map_err(|kind| ZhangError::ProcessError { span: span.clone(), kind })?,
            );

            let option = operations.account_target_day_balance(
                txn_posting.posting.account.name(),
                self.date.to_timezone_datetime(&ledger.options.timezone),
                &inferred_amount.commodity,
            )?;

            let previous = option.unwrap_or(Amount {
                number: BigDecimal::zero(),
                commodity: inferred_amount.commodity.clone(),
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

            // booking: augment or reduce the account's lots
            let amount = txn_posting.units().unwrap_or(inferred_amount);
            let booking = ledger.booker_mut().book_posting(&txn_posting, &amount);
            for error in booking.errors {
                operations.new_error(error.kind, span, error.metas)?;
            }
            balance_checker.add_assign(booking.weight);
            trace!("balance checker current value is {}", balance_checker);
        }
        trace!("final balance checker current value is {}, txn_error is {:?}", balance_checker, txn_error);
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
