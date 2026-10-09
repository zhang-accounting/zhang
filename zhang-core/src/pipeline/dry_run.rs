//! A dry run of one transaction: what the ledger would report against it once written, found by the stages a load checks
//! it with, without writing it.

use std::collections::HashMap;

use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Spanned, Transaction};

use super::validate::Watched;
use super::{ActiveAccountsStage, ProcessStage, StageContext, ValidateStage};
use crate::ledger::Ledger;
use crate::ZhangResult;

/// What the ledger would report against a transaction once written ([`Ledger::check_transaction`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionCheck {
    /// the errors a load would report against the transaction, in its order, each with its metas: the accounts it may
    /// not use at its date and time ([`ActiveAccountsStage`]), then those of booking and of final validation
    /// ([`ValidateStage`]), such as [`ErrorKind::UnbalancedTransaction`]
    pub errors: Vec<(ErrorKind, HashMap<String, String>)>,
    /// what the transaction is unbalanced by once booked, each posting weighed by its cost or price, rounded at each
    /// commodity's precision, as final validation checks it: empty when it balances, `None` when booking rejects it
    pub unbalanced: Option<Vec<Amount>>,
}

impl Ledger {
    /// Check `transaction` as a load of the ledger with it written would: [`ActiveAccountsStage`] and
    /// [`ValidateStage`], the stages that report on a transaction once the plugins ran, run over the directives this
    /// load validated up to the transaction, with it where the load sorts it. `replaces` is the span of the transaction
    /// an edit replaces: that one is left out, and the edit takes its place among the directives of its date and time.
    /// A new transaction comes after them. Nothing is written, and the ledger does not change.
    ///
    /// The transaction is checked as written, before booking: booking completes it as a load would.
    pub fn check_transaction(&self, transaction: Transaction, replaces: Option<&SpanInfo>) -> ZhangResult<TransactionCheck> {
        // a span no directive of a load has
        let span = SpanInfo {
            start: usize::MAX,
            end: usize::MAX,
            ..SpanInfo::default()
        };
        let stream = self.stream_until(Spanned::new(Directive::Transaction(transaction), span.clone()), replaces);
        let mut ctx = StageContext::new(&self.options)
            .with_dialect(self.dialect)
            .with_commodities(self.options.operating_currency_commodity().into_iter().collect());
        ctx.validation.watched = Some(Watched {
            span: span.clone(),
            unbalanced: None,
        });
        let stream = ActiveAccountsStage.process(stream, &mut ctx)?;
        ValidateStage.process(stream, &mut ctx)?;
        let (_, validation) = ctx.into_materialize_results();
        let errors = validation
            .errors
            .into_iter()
            .filter(|error| error.span == span)
            .map(|error| (error.kind, error.metas))
            .collect();
        Ok(TransactionCheck {
            errors,
            unbalanced: validation.watched.and_then(|watched| watched.unbalanced),
        })
    }

    /// the dated directives of the ledger, sorted as a load sorts them, with `candidate` in place of `replaces` (or
    /// after them when it is not there), up to and including `candidate`
    fn stream_until(&self, candidate: Spanned<Directive>, replaces: Option<&SpanInfo>) -> Vec<Spanned<Directive>> {
        let span = candidate.span.clone();
        let mut stream = self.directives.clone();
        let position = replaces.and_then(|replaces| {
            stream
                .iter()
                .position(|it| &it.span == replaces && matches!(it.data, Directive::Transaction(_)))
        });
        match position {
            Some(position) => stream[position] = candidate,
            None => stream.push(candidate),
        }
        // the sort is stable, and the directives are sorted already: the candidate moves to its date and time, where
        // its place in the input breaks a tie
        let mut stream = Ledger::sort_directives_datetime(stream, self.dialect);
        let end = stream.iter().position(|it| it.span == span).expect("the candidate is in the stream") + 1;
        stream.truncate(end);
        stream
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;

    use bigdecimal::BigDecimal;
    use indoc::indoc;
    use tempfile::tempdir;
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Directive, SpanInfo, Transaction};

    use super::TransactionCheck;
    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;

    const LEDGER: &str = indoc! {r#"
        option "operating_currency" "USD"
        1970-01-01 commodity USD
        1970-01-01 commodity CNY
          precision: 2
        1970-01-01 commodity AAPL
          precision: 0
        1970-01-01 open Assets:Broker
        1970-01-01 open Assets:Cash
        1970-01-01 open Expenses:Food
        2023-01-01 close Expenses:Food
    "#};

    fn load(content: &str) -> Ledger {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
        // the ledger keeps what it read: the files can go
        drop(dir);
        ledger
    }

    /// the transaction `text` as the parser reads it, unbooked
    fn transaction(text: &str) -> Transaction {
        let mut directives = ZhangDataType {}.transform(text.to_owned(), None).unwrap();
        match directives.remove(0).data {
            Directive::Transaction(transaction) => transaction,
            other => panic!("not a transaction: {other:?}"),
        }
    }

    fn amount(number: &str, commodity: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), commodity)
    }

    fn kinds(check: &TransactionCheck) -> Vec<ErrorKind> {
        check.errors.iter().map(|(kind, _)| kind.clone()).collect()
    }

    #[test]
    fn a_purchase_at_cost_balances_by_its_weight() {
        let ledger = load(LEDGER);
        let check = ledger
            .check_transaction(
                transaction(indoc! {r#"
                    2024-01-02 * "buy"
                      Assets:Broker 10 AAPL {150 USD}
                      Assets:Cash -1500 USD
                "#}),
                None,
            )
            .unwrap();
        assert_eq!(
            check,
            TransactionCheck {
                errors: vec![],
                unbalanced: Some(vec![])
            }
        );
    }

    #[test]
    fn a_residual_within_the_precision_balances_and_one_beyond_is_reported_rounded() {
        let ledger = load(LEDGER);
        let check = |text: &str| ledger.check_transaction(transaction(text), None).unwrap();
        let within = check(indoc! {r#"
            2024-01-02 * "dust"
              Assets:Cash 10.005 CNY
              Assets:Broker -10 CNY
        "#});
        assert_eq!(within.unbalanced, Some(vec![]));
        let beyond = check(indoc! {r#"
            2024-01-02 * "off"
              Assets:Cash 10.5 CNY
              Assets:Broker -10 CNY
              Assets:Cash 1 USD
        "#});
        assert_eq!(beyond.unbalanced, Some(vec![amount("0.50", "CNY"), amount("1", "USD")]));
        assert_eq!(kinds(&beyond), [ErrorKind::UnbalancedTransaction]);
    }

    #[test]
    fn an_unbookable_transaction_has_no_residual() {
        let ledger = load(LEDGER);
        let check = ledger
            .check_transaction(
                transaction(indoc! {r#"
                    2024-01-02 * "two implicit"
                      Assets:Cash
                      Assets:Broker
                "#}),
                None,
            )
            .unwrap();
        assert_eq!(check.unbalanced, None);
        assert_eq!(kinds(&check), [ErrorKind::TransactionHasMultipleImplicitPosting]);
    }

    #[test]
    fn the_accounts_are_checked_at_the_transactions_date() {
        let ledger = load(LEDGER);
        let check = |date: &str| {
            ledger
                .check_transaction(transaction(&format!("{date} * \"lunch\"\n  Assets:Cash -5 USD\n  Expenses:Food 5 USD\n")), None)
                .unwrap()
        };
        assert_eq!(check("2022-12-31").errors, vec![]);
        assert_eq!(
            check("2023-01-02").errors,
            vec![(
                ErrorKind::AccountClosed,
                HashMap::from([("account_name".to_owned(), "Expenses:Food".to_owned())])
            )]
        );
    }

    #[test]
    fn a_sale_books_against_the_lots_held_before_it() {
        let ledger = load(&format!(
            "{LEDGER}{}",
            indoc! {r#"
                2024-01-01 * "buy"
                  Assets:Broker 10 AAPL {150 USD}
                  Assets:Cash -1500 USD
            "#}
        ));
        let sale = |date: &str, units: &str| {
            ledger
                .check_transaction(
                    transaction(&format!("{date} * \"sell\"\n  Assets:Broker -{units} AAPL {{}}\n  Assets:Cash\n")),
                    None,
                )
                .unwrap()
        };
        assert_eq!(
            sale("2024-01-02", "4"),
            TransactionCheck {
                errors: vec![],
                unbalanced: Some(vec![])
            }
        );
        // before the purchase there is no lot to sell, and after it not 11 shares
        assert_eq!(kinds(&sale("2023-12-31", "4")), [ErrorKind::TransactionCannotInferTradeAmount]);
        assert_eq!(
            kinds(&sale("2024-01-02", "11")),
            [ErrorKind::NoEnoughCommodityLot, ErrorKind::TransactionCannotInferTradeAmount]
        );
    }

    #[test]
    fn an_edit_replaces_the_transaction_it_edits() {
        let ledger = load(&format!(
            "{LEDGER}{}",
            indoc! {r#"
                2024-01-01 * "buy"
                  Assets:Broker 10 AAPL {150 USD}
                  Assets:Cash -1500 USD
                2024-01-02 * "sell"
                  Assets:Broker -6 AAPL {}
                  Assets:Cash
            "#}
        ));
        let span = |narration: &str| -> SpanInfo {
            ledger
                .directives
                .iter()
                .find(|it| it.span.content.contains(narration))
                .map(|it| it.span.clone())
                .unwrap()
        };
        // a second sale of 6 shares finds only 4, but the edit of the sale takes its place
        let resale = transaction("2024-01-02 * \"sell\"\n  Assets:Broker -6 AAPL {}\n  Assets:Cash\n");
        assert_eq!(
            kinds(&ledger.check_transaction(resale.clone(), None).unwrap()),
            [ErrorKind::NoEnoughCommodityLot, ErrorKind::TransactionCannotInferTradeAmount]
        );
        assert_eq!(
            ledger.check_transaction(resale, Some(&span("\"sell\""))).unwrap(),
            TransactionCheck {
                errors: vec![],
                unbalanced: Some(vec![])
            }
        );
        // an edit of the purchase to 5 shares leaves the later sale short, which is not the edit's error
        let half = transaction("2024-01-01 * \"buy\"\n  Assets:Broker 5 AAPL {150 USD}\n  Assets:Cash -750 USD\n");
        assert_eq!(
            ledger.check_transaction(half, Some(&span("\"buy\""))).unwrap(),
            TransactionCheck {
                errors: vec![],
                unbalanced: Some(vec![])
            }
        );
    }
}
