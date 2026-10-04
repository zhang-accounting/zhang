use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use itertools::Itertools;
use log::{trace, warn};
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Flag, SpanInfo, Transaction};

use crate::booking::{group_units, is_booked, written_groups, BookOutcome};
use crate::constants::TXN_ID;
use crate::domains::schemas::MetaType;
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::utils::hashmap::HashMapOfExt;
use crate::utils::id::FromSpan;
use crate::ZhangResult;

impl DirectiveProcess for Transaction {
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        // a stage may synthesize several transactions at one place: the paddings of a `pad` serving several currencies
        let id = ledger.operations().unused_id(Uuid::from_span(span));
        let txn_meta = || HashMap::of(TXN_ID, id.to_string());

        // booking first: the lots decide the weights the implicit posting is interpolated from (E4).
        // This is pass 2 of booking (design #423 §5.2, V3): the booking stage booked the transaction
        // before the plugins ran, unless a stage emitted it since or left it unbooked. Booking it
        // again leaves a booked posting as it is and completes an unbooked one, so the stream the
        // ledger keeps is the booked one; the errors are reported here, once. A plugin may have
        // changed the lots a leg depends on since (an earlier sale added, a lot relabelled): this
        // pass is then the booking that counts, and says so in the log
        let before = is_booked(self).then(|| self.postings.clone());
        let outcome = ledger.booker_mut().book(self);
        if let Some(before) = before.filter(|before| *before != self.postings) {
            let leg = before
                .iter()
                .zip(&self.postings)
                .position(|(before, after)| before != after)
                .unwrap_or(before.len().min(self.postings.len()));
            warn!(
                "booking the final stream changed transaction {id} ({} {}), booked before the plugins: a stage changed the lots its legs depend on. Leg {leg}: {:?} before, {:?} now",
                self.date.naive_date(),
                self.narration.as_ref().map(|it| it.as_str()).unwrap_or_default(),
                before.get(leg).map(|it| (&it.account.content, &it.units, &it.cost)),
                self.postings.get(leg).map(|it| (&it.account.content, &it.units, &it.cost)),
            );
        }
        let booked = match outcome {
            BookOutcome::Booked(booked) => booked,
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
        let balance_error = operations.check_transaction_balance(&booked.residual)?;
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

        // one row per posting as written: the legs booking split from it, adjacent and sharing
        // `written.index`, are summed into it, so the rows, their ids and the balances are those
        // of the written postings. A split a stage broke apart (`written_groups`) is one row per
        // leg, as booked, like the exporter shows it
        for (posting_idx, group) in written_groups(&self.postings).into_iter().enumerate() {
            let legs = group.legs;
            let posting = &legs[0];
            let inferred_amount = group_units(legs);
            let (unit, cost) = match group.written {
                Some(written) => (written.units.clone(), written.cost.clone()),
                None => (posting.units.clone(), posting.cost.clone()),
            };
            operations.insert_transaction_posting(
                &id,
                posting_idx,
                posting.flag.clone(),
                posting.account.name(),
                unit,
                cost,
                inferred_amount,
                posting.meta.clone(),
            )?;

            // budget related: like `budget-add`, activity on a budget the stream has not defined
            // (yet) is skipped. It is reported once per (account, budget), on the first
            // transaction that loses activity, instead of once per posting
            let budgets_name = operations.get_account_budget(posting.account.name())?;
            for budget in budgets_name {
                if !super::budget::is_defined(ledger, &budget) {
                    let account_name = posting.account.name().to_owned();
                    if ledger.reported_undefined_budgets.insert((account_name.clone(), budget.clone())) {
                        let metas = HashMap::of2("account_name", account_name, "budget_name", budget);
                        operations.new_error(ErrorKind::BudgetDoesNotExist, span, metas)?;
                    }
                }
            }
        }
        for error in booked.errors {
            operations.new_error(error.kind, span, error.metas)?;
        }
        trace!("residual of transaction {}: {:?}", id, booked.residual);
        if balance_error == Some(ErrorKind::UnbalancedTransaction) {
            operations.new_error(ErrorKind::UnbalancedTransaction, span, txn_meta())?;
        }

        // extract documents from meta. A `document` of a posting is a document of its
        // transaction too: older zhang appended uploaded documents after the postings, which
        // a beancount ledger reads as metadata of the last posting. The legs of a split posting
        // share its meta: counted once
        let documents = std::iter::once(&self.meta)
            .chain(written_groups(&self.postings).into_iter().map(|group| &group.legs[0].meta))
            .flat_map(|meta| meta.get_all("document"))
            .collect_vec();
        for document_file_name in documents {
            let document_path = document_file_name.as_str().to_owned();
            let document_pathbuf = PathBuf::from(&document_path);
            operations.insert_document(
                self.date.to_timezone_datetime(&ledger.options.timezone),
                document_pathbuf.file_name().and_then(|it| it.to_str()),
                document_path,
                None,
                DocumentType::Trx(id),
            )?;
        }
        operations.insert_meta(MetaType::TransactionMeta, id.to_string(), self.meta.clone())?;
        Ok(())
    }
}
