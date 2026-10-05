//! Final booking and transaction validation, after plugins and balance stages.

use std::collections::{HashMap, VecDeque};

use log::{trace, warn};
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Spanned};

use super::balance::{define_commodity, stage_booker};
use super::{ProcessStage, StageContext, StageError};
use crate::booking::{is_booked, BookOutcome};
use crate::constants::TXN_ID;
use crate::store::CommodityLotRecord;
use crate::utils::id::FromSpan;
use crate::ZhangResult;

/// Books the final stream and reports booking, transaction balance and nonzero close errors
/// once. Rejected transactions stay in the stream but contribute no postings or lots to the store.
/// The store consumes these results without booking again.
pub struct ValidateStage;

/// Results consumed by the store fold. Transaction IDs are allocated there, alongside balance
/// assertions and postings; errors are bound to those IDs when their transaction is consumed.
#[derive(Default)]
pub(crate) struct FinalValidation {
    pub errors: Vec<StageError>,
    pub lots: HashMap<String, Vec<CommodityLotRecord>>,
    transactions: HashMap<Uuid, VecDeque<TransactionOutcome>>,
}

struct TransactionOutcome {
    accepted: bool,
    /// Positions in the stage error channel of errors that carry `txn_id`.
    error_indices: Vec<usize>,
}

impl FinalValidation {
    fn record(&mut self, span: &SpanInfo, accepted: bool, error_indices: Vec<usize>) {
        self.transactions
            .entry(Uuid::from_span(span))
            .or_default()
            .push_back(TransactionOutcome { accepted, error_indices });
    }

    /// Consume the next transaction at this span, including a rejected one. A rejected transaction
    /// does not reserve its candidate ID, just as it did when the store booked transactions itself.
    pub fn take_transaction(&mut self, span: &SpanInfo, id: Uuid) -> bool {
        let outcome = self
            .transactions
            .get_mut(&Uuid::from_span(span))
            .and_then(VecDeque::pop_front)
            .expect("every transaction is processed by the final validation stage");
        for index in outcome.error_indices {
            self.errors[index].metas.insert(TXN_ID.to_owned(), id.to_string());
        }
        outcome.accepted
    }
}

impl ProcessStage for ValidateStage {
    fn name(&self) -> &str {
        "validate"
    }

    fn process(&self, mut directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut booker = stage_booker(ctx);
        for directive in &mut directives {
            let span = &directive.span;
            match &mut directive.data {
                Directive::Open(open) => {
                    if let Some(error) = booker.apply_open(open) {
                        ctx.emit_error(error.kind, span.clone(), error.metas);
                    }
                }
                Directive::Commodity(commodity) => define_commodity(&mut booker, commodity, ctx.options),
                Directive::Close(close) => {
                    // Check the account's own units, not its subtree or each lot separately.
                    if booker.has_non_zero_balance(close.account.name()) {
                        ctx.emit_error(ErrorKind::CloseNonZeroAccount, span.clone(), HashMap::new());
                    }
                }
                Directive::Transaction(txn) => {
                    let before = is_booked(txn).then(|| txn.postings.clone());
                    let outcome = booker.book(txn);
                    if let Some(before) = before.filter(|before| *before != txn.postings) {
                        let leg = before
                            .iter()
                            .zip(&txn.postings)
                            .position(|(before, after)| before != after)
                            .unwrap_or(before.len().min(txn.postings.len()));
                        warn!(
                            "booking the final stream changed transaction at {:?}:{} ({} {}), booked before the plugins: a stage changed the lots its legs depend on. Leg {leg}: {:?} before, {:?} now",
                            span.filename, span.start,
                            txn.date.naive_date(),
                            txn.narration.as_ref().map(|it| it.as_str()).unwrap_or_default(),
                            before.get(leg).map(|it| (&it.account.content, &it.units, &it.cost)),
                            txn.postings.get(leg).map(|it| (&it.account.content, &it.units, &it.cost)),
                        );
                    }
                    let mut error_indices = vec![];
                    let accepted = match outcome {
                        BookOutcome::Unbookable { kind, errors } => {
                            for error in errors {
                                ctx.emit_error(error.kind, span.clone(), error.metas);
                            }
                            error_indices.push(ctx.errors.len());
                            ctx.emit_error(kind, span.clone(), HashMap::new());
                            false
                        }
                        BookOutcome::Booked(booked) => {
                            let balance_error = booker.check_transaction_balance(&booked.residual);
                            if balance_error == Some(ErrorKind::CommodityDoesNotDefine) {
                                error_indices.push(ctx.errors.len());
                                ctx.emit_error(ErrorKind::CommodityDoesNotDefine, span.clone(), HashMap::new());
                            }
                            for error in booked.errors {
                                ctx.emit_error(error.kind, span.clone(), error.metas);
                            }
                            trace!("residual of transaction at {:?}:{}: {:?}", span.filename, span.start, booked.residual);
                            if balance_error == Some(ErrorKind::UnbalancedTransaction) {
                                error_indices.push(ctx.errors.len());
                                ctx.emit_error(ErrorKind::UnbalancedTransaction, span.clone(), HashMap::new());
                            }
                            true
                        }
                    };
                    ctx.validation.record(span, accepted, error_indices);
                }
                _ => {}
            }
        }
        ctx.validation.lots = booker.into_lots();
        Ok(directives)
    }
}

#[cfg(test)]
mod test {
    use indoc::indoc;
    use zhang_ast::Rounding;

    use super::*;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::domains::schemas::CommodityDomain;
    use crate::ledger::Ledger;

    #[test]
    fn final_booking_completes_postings_and_rolls_back_a_rejected_cost_sale() {
        let directives = ZhangDataType {}
            .transform(
                indoc! {r#"
                    1970-01-01 commodity USD
                    1970-01-01 commodity STOCK
                    1970-01-01 open Assets:A
                    1970-01-01 open Equity:E
                    2024-01-01 * "buy"
                      Assets:A 5 STOCK {10 USD}
                      Equity:E
                    2024-01-02 * "reject"
                      Assets:A -6 STOCK {}
                      Equity:E 60 USD
                    2024-01-03 * "sell"
                      Assets:A -1 STOCK {}
                      Equity:E
                "#}
                .to_owned(),
                None,
            )
            .unwrap();
        let rejected = directives.iter().find(|it| it.span.content.contains("\"reject\"")).unwrap().clone();
        let mut ctx = StageContext::new(&[]);
        let out = ValidateStage.process(Ledger::sort_directives_datetime(directives), &mut ctx).unwrap();
        let (_, mut result) = ctx.into_materialize_results();
        assert_eq!(
            result.errors.iter().map(|it| &it.kind).collect::<Vec<_>>(),
            [&ErrorKind::NoEnoughCommodityLot, &ErrorKind::TransactionCannotInferTradeAmount]
        );
        let transactions = out.iter().filter(|it| matches!(it.data, Directive::Transaction(_))).collect::<Vec<_>>();
        assert_eq!(transactions[1], &rejected, "a rejected transaction stays in the stream as written");
        let id = Uuid::from_span(&rejected.span);
        assert!(!result.take_transaction(&rejected.span, id));
        assert_eq!(result.errors[1].metas[TXN_ID], id.to_string());
        assert!(!result.errors[0].metas.contains_key(TXN_ID));
        assert_eq!(result.lots["Assets:A"].len(), 1);
        assert_eq!(result.lots["Assets:A"][0].amount.to_string(), "4");
        let Directive::Transaction(sale) = &transactions[2].data else {
            unreachable!()
        };
        assert_eq!(sale.postings[1].units.as_ref().unwrap().to_string(), "10 USD");
    }

    #[test]
    fn final_balance_validation_seeds_operating_currency_and_uses_definitions_in_stream_order() {
        let directives = ZhangDataType {}
            .transform(
                indoc! {r#"
                    1970-01-01 open Assets:A
                    1970-01-01 open Equity:E
                    2024-01-01 * "operating currency rounds to zero"
                      Assets:A 1.004 USD
                      Equity:E -1 USD
                    2024-01-02 * "undefined zero residual precedes known imbalance"
                      Assets:A 10 JPY
                      Equity:E -10 JPY
                      Assets:A 1 USD
                    2024-01-03 commodity JPY
                    2024-01-03 commodity USD
                      precision: 3
                    2024-01-03 * "redefined precision"
                      Assets:A 1.004 USD
                      Equity:E -1 USD
                "#}
                .to_owned(),
                None,
            )
            .unwrap();
        let mut ctx = StageContext::new(&[]).with_commodities(vec![CommodityDomain {
            name: "USD".to_owned(),
            precision: 2,
            prefix: None,
            suffix: None,
            rounding: Rounding::RoundDown,
        }]);
        ValidateStage.process(Ledger::sort_directives_datetime(directives), &mut ctx).unwrap();
        assert_eq!(
            ctx.into_errors().into_iter().map(|it| it.kind).collect::<Vec<_>>(),
            [ErrorKind::CommodityDoesNotDefine, ErrorKind::UnbalancedTransaction]
        );
    }
}
