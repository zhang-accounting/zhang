//! Booking as a native pipeline stage: pass 1 of the booking split (design #423, §2 and §4).

use zhang_ast::{Directive, Spanned};

use super::balance::{define_commodity, stage_booker};
use super::{ProcessStage, StageContext};
use crate::ZhangResult;

/// books every transaction of the stream, in place, before the plugins see it: the implicit
/// posting gets its interpolated units, a cost spec becomes the per-unit cost and acquisition date
/// of the lot it books against, and a reduction spanning several lots becomes one posting per lot
/// ([`Booker::book`](crate::booking::Booker::book)). A transaction booking rejects (several
/// implicit postings, unresolved costs, nothing to infer a commodity from, weights in several commodities) is left
/// as written.
///
/// Like beancount's booking, which runs before the plugins and whose balances are dropped, this
/// pass keeps nothing: the lots it builds are stale as soon as a later stage adds a transaction,
/// and its errors are those of a stream the plugins have not seen yet. [`ValidateStage`](super::ValidateStage) books the
/// final stream again (pass 2): it leaves what is booked unchanged, completes what a stage left
/// unbooked, reports every booking error once, and its lots become the store's.
pub struct BookingStage;

impl ProcessStage for BookingStage {
    fn name(&self) -> &str {
        "booking"
    }

    fn process(&self, mut directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut booker = stage_booker(ctx);
        for directive in &mut directives {
            match &mut directive.data {
                Directive::Open(open) => {
                    let _reported_by_validation = booker.apply_open(open);
                }
                Directive::Commodity(commodity) => define_commodity(&mut booker, commodity, ctx.options),
                Directive::Transaction(txn) => {
                    let _reported_by_validation = booker.book(txn);
                }
                _ => {}
            }
        }
        Ok(directives)
    }
}

#[cfg(test)]
mod test {
    use indoc::indoc;

    use super::BookingStage;
    use crate::booking::tests::postings;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;
    use crate::pipeline::{run_pipeline, ProcessStage, StageContext};

    /// the stream after the booking stage, as [`postings`] shows its transactions, and the errors it reported
    fn booked(content: &str) -> (Vec<Vec<String>>, usize) {
        let directives = ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        let mut ctx = StageContext::new(&[]);
        let stages: Vec<Box<dyn ProcessStage>> = vec![Box::new(BookingStage)];
        let out = run_pipeline(&stages, Ledger::sort_directives_datetime(directives), &mut ctx).unwrap();
        (postings(&out), ctx.into_errors().len())
    }

    #[test]
    fn should_book_the_transactions_in_place_and_report_nothing() {
        let (postings, errors) = booked(indoc! {r#"
            1970-01-01 commodity CNY
              precision: "2"
            1970-01-01 open Assets:A
            1970-01-01 open Income:I
            2024-05-16 * "buy"
              Assets:A 10 USD { 10 CNY }
              Income:I
            2024-05-17 * "buy at total cost"
              Assets:A 10 USD {{ 110 CNY }}
              Income:I -110 CNY
            2024-05-18 * "sell across both lots, more than held"
              Assets:A -25 USD {}
              Income:I 260 CNY
        "#});
        assert_eq!(
            postings,
            vec![
                vec!["Assets:A 10 USD {10 CNY, 2024-05-16} <- #0 10 USD {10 CNY}", "Income:I -100 CNY <- #1 ?"],
                vec!["Assets:A 10 USD {11 CNY, 2024-05-17} <- #0 10 USD {{110 CNY}}", "Income:I -110 CNY"],
                vec!["Assets:A -25 USD {}", "Income:I 260 CNY"],
            ]
        );
        // the lots run short on the sale: the error is final validation's to report, not this stage's
        assert_eq!(errors, 0);
    }

    #[test]
    fn should_leave_a_transaction_it_cannot_book_as_written() {
        let (postings, errors) = booked(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Income:I
            2024-05-16 * "two implicit postings"
              Assets:A 10 USD
              Assets:A
              Income:I
            2024-05-17 * "weights in two commodities"
              Assets:A 10 USD
              Assets:A 10 CNY
              Income:I
        "#});
        assert_eq!(
            postings,
            vec![
                vec!["Assets:A 10 USD", "Assets:A ?", "Income:I ?"],
                vec!["Assets:A 10 USD", "Assets:A 10 CNY", "Income:I ?"],
            ]
        );
        assert_eq!(errors, 0);
    }
}
