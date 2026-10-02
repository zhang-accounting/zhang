use std::collections::HashMap;
use std::ops::{Add, Mul};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use bigdecimal::{BigDecimal, Zero};
use itertools::Itertools;
use log::trace;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Flag, SpanInfo, Transaction};

use crate::booking::BookOutcome;
use crate::constants::TXN_ID;
use crate::domains::schemas::MetaType;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::{ZhangError, ZhangResult};

impl DirectiveProcess for Transaction {
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let id = Uuid::from_span(span);
        let txn_meta = || HashMap::of(TXN_ID, id.to_string());
        // balance-check transactions (flag `C`) are exempt from validation
        let exempt = self.flag == Some(Flag::BalanceCheck);

        // booking first: the lots decide the weights the implicit posting is interpolated from (E4)
        let booked = match ledger.booker_mut().book(self) {
            BookOutcome::Booked(booked) => booked,
            // nothing to book an exempt transaction's implicit posting with: the load fails
            BookOutcome::Unbookable { kind, .. } if exempt => return Err(ZhangError::ProcessError { span: span.clone(), kind }),
            // the transaction is rejected: it never reaches the store, nor its lots
            BookOutcome::Unbookable { kind, errors } => {
                let mut operations = ledger.operations();
                for error in errors {
                    operations.new_error(error.kind, span, error.metas)?;
                }
                operations.new_error(kind, span, txn_meta())?;
                return Ok(());
            }
        };

        let mut operations = ledger.operations();
        let balance_error = if exempt {
            None
        } else {
            operations.check_transaction_balance(&booked.residual)?
        };
        if balance_error == Some(ErrorKind::CommodityDoesNotDefine) {
            operations.new_error(ErrorKind::CommodityDoesNotDefine, span, txn_meta())?;
        }

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

        for (posting_idx, (posting, inferred_amount)) in self.postings.iter().zip(booked.units).enumerate() {
            let option = operations.account_target_day_balance(
                posting.account.name(),
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
                posting.account.name(),
                posting.units.clone(),
                posting.cost.clone(),
                inferred_amount.clone(),
                Amount::new(previous.number, previous.commodity.clone()),
                Amount::new(after_number, previous.commodity),
            )?;

            // budget related: like `budget-add`, activity on a budget the stream has not defined
            // (yet) is skipped. It is reported once per (account, budget), on the first
            // transaction that loses activity, instead of once per posting
            let budgets_name = operations.get_account_budget(posting.account.name())?;
            for budget in budgets_name {
                if !operations.contains_budget(&budget) {
                    let account_name = posting.account.name().to_owned();
                    if ledger.reported_undefined_budgets.insert((account_name.clone(), budget.clone())) {
                        let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                        operations.new_error(ErrorKind::BudgetDoesNotExist, span, metas)?;
                    }
                    continue;
                }
                let budget_activity_amount = inferred_amount.mul(BigDecimal::from(posting.account.get_account_sign()));
                operations.budget_add_activity(budget, self.date.to_timezone_datetime(&ledger.options.timezone), budget_activity_amount)?;
            }
        }
        for error in booked.errors {
            operations.new_error(error.kind, span, error.metas)?;
        }
        trace!("residual of transaction {}: {:?}", id, booked.residual);
        if balance_error == Some(ErrorKind::UnbalancedTransaction) {
            operations.new_error(ErrorKind::UnbalancedTransaction, span, txn_meta())?;
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
