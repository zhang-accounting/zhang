//! A dry run of one transaction, or of one `open`: what the ledger would report against it once written, found by the
//! stages a load checks it with, without writing it. A batch of directives previewed together is checked one at a
//! time, each with the earlier ones of the batch counted as written ([`Ledger::check_transaction_with`]).

use std::collections::{BTreeSet, HashMap};

use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Open, SpanInfo, Spanned, Transaction};

use super::balance::AccountCommodities;
use super::validate::Watched;
use super::{AccountLifecycle, ActiveAccountsStage, ProcessStage, StageContext, ValidateStage};
use crate::domains::schemas::AccountStatus;
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

/// What an `open` would change once written ([`Ledger::check_open`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCheck {
    /// whether the account is open at the `open`'s date and time with the same commodities already: writing the
    /// `open` would change nothing
    pub already_open: bool,
    /// the commodities the account is open with at the `open`'s date and time, when it is open with others than the
    /// `open` lists (an empty list: any commodity); `None` when it is not open, or open with the same
    pub open_with: Option<Vec<String>>,
}

/// a span no directive of a load has: the candidate of a dry run, and, counting down from it, the directives of a
/// batch that count as written before it
fn dry_run_span(index: usize) -> SpanInfo {
    SpanInfo {
        start: usize::MAX - index,
        end: usize::MAX - index,
        ..SpanInfo::default()
    }
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
        self.check_transaction_with(transaction, replaces, &[])
    }

    /// [`Ledger::check_transaction`], with the directives `with`, which are not in the ledger, counted as written
    /// before the transaction: the earlier entries of a batch previewed together, such as the `open` of an account the
    /// transaction posts to, or a purchase whose lot it sells. Each of them is sorted to its own date and time; among
    /// the directives of the transaction's date and time, they come before it, in their order
    pub fn check_transaction_with(&self, transaction: Transaction, replaces: Option<&SpanInfo>, with: &[Directive]) -> ZhangResult<TransactionCheck> {
        let span = dry_run_span(0);
        let stream = self.stream_until(Spanned::new(Directive::Transaction(transaction), span.clone()), replaces, with);
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

    /// What an `open` would change, once written: nothing, when the account is open at its date and time with the
    /// same commodities already (`already_open`); or it would re-open an account that is open with other commodities
    /// (`open_with`), which the ledger allows but a plugin may not do. `with` are directives not in the ledger that
    /// count as written before it, as for [`Ledger::check_transaction_with`]. Nothing is written
    pub fn check_open(&self, open: &Open, with: &[Directive]) -> OpenCheck {
        let at = open.date.naive_datetime();
        let candidate = Spanned::new(Directive::Open(open.clone()), dry_run_span(0));
        let stream = self.stream_until(candidate, None, with);
        // every directive before the candidate, which is the last of the stream
        let mut lifecycle = AccountLifecycle::default();
        let mut commodities = AccountCommodities::default();
        for directive in &stream[..stream.len() - 1] {
            lifecycle.apply(&directive.data);
            commodities.apply(&directive.data);
        }
        if lifecycle.status(open.account.name(), at) != Some(AccountStatus::Open) {
            return OpenCheck {
                already_open: false,
                open_with: None,
            };
        }
        let open_with = commodities.allowed(open.account.name()).unwrap_or_default();
        let same = open_with.iter().collect::<BTreeSet<_>>() == open.commodities.iter().collect::<BTreeSet<_>>();
        OpenCheck {
            already_open: same,
            open_with: (!same).then(|| open_with.to_vec()),
        }
    }

    /// the dated directives of the ledger and `with`, sorted as a load sorts them, with `candidate` in place of
    /// `replaces` (or after them when it is not there), up to and including `candidate`. The directives of `with` are
    /// sorted to their dates and times too, before the candidate among those of its date and time
    fn stream_until(&self, candidate: Spanned<Directive>, replaces: Option<&SpanInfo>, with: &[Directive]) -> Vec<Spanned<Directive>> {
        let span = candidate.span.clone();
        let mut stream = self.directives.clone();
        let position = replaces.and_then(|replaces| {
            stream
                .iter()
                .position(|it| &it.span == replaces && matches!(it.data, Directive::Transaction(_)))
        });
        stream.extend(
            with.iter()
                .enumerate()
                .map(|(index, directive)| Spanned::new(directive.clone(), dry_run_span(index + 1))),
        );
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

    use super::{OpenCheck, TransactionCheck};
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

    /// the first directive of `text`, as the parser reads it
    fn directive(text: &str) -> Directive {
        ZhangDataType {}.transform(text.to_owned(), None).unwrap().remove(0).data
    }

    #[test]
    fn a_batch_counts_its_earlier_directives_as_written() {
        let ledger = load(LEDGER);
        let coffee = transaction("2024-01-02 * \"coffee\"\n  Assets:Cash -5 USD\n  Expenses:Coffee 5 USD\n");
        assert_eq!(
            ledger.check_transaction(coffee.clone(), None).unwrap().errors,
            vec![(
                ErrorKind::AccountDoesNotExist,
                HashMap::from([("account_name".to_owned(), "Expenses:Coffee".to_owned())])
            )]
        );
        // an `open` earlier in the batch counts, at its own date: the same day, or a year before
        for open in ["2024-01-02 open Expenses:Coffee USD", "2023-01-01 open Expenses:Coffee"] {
            assert_eq!(
                ledger.check_transaction_with(coffee.clone(), None, &[directive(open)]).unwrap(),
                TransactionCheck {
                    errors: vec![],
                    unbalanced: Some(vec![])
                },
                "{open}"
            );
        }
        // and so does a purchase earlier in the batch, whose lot a later sale takes
        let buy = directive("2024-01-01 * \"buy\"\n  Assets:Broker 10 AAPL {150 USD}\n  Assets:Cash -1500 USD\n");
        let sell = transaction("2024-01-02 * \"sell\"\n  Assets:Broker -4 AAPL {}\n  Assets:Cash\n");
        assert_eq!(
            kinds(&ledger.check_transaction(sell.clone(), None).unwrap()),
            [ErrorKind::TransactionCannotInferTradeAmount]
        );
        assert_eq!(ledger.check_transaction_with(sell, None, &[buy]).unwrap().errors, vec![]);
    }

    #[test]
    fn an_open_of_an_open_account_is_already_open_with_the_same_commodities() {
        let ledger = load(&format!("{LEDGER}1970-01-01 open Assets:Bank USD, EUR\n"));
        let open = |text: &str| match directive(text) {
            Directive::Open(open) => open,
            other => panic!("not an open: {other:?}"),
        };
        let check = |text: &str| ledger.check_open(&open(text), &[]);
        let not_open = OpenCheck {
            already_open: false,
            open_with: None,
        };
        assert_eq!(check("2024-01-02 open Expenses:Coffee"), not_open);
        // the commodities as a set: the order does not matter
        assert_eq!(
            check("2024-01-02 open Assets:Bank EUR, USD"),
            OpenCheck {
                already_open: true,
                open_with: None
            }
        );
        assert_eq!(
            check("2024-01-02 open Assets:Bank USD"),
            OpenCheck {
                already_open: false,
                open_with: Some(vec!["USD".to_owned(), "EUR".to_owned()])
            }
        );
        // an account open for any commodity is open with an empty list
        assert_eq!(
            check("2024-01-02 open Assets:Cash USD"),
            OpenCheck {
                already_open: false,
                open_with: Some(vec![])
            }
        );
        assert_eq!(check("2024-01-02 open Assets:Cash").already_open, true);
        // once closed, the account can be opened again; before its open it is not open
        assert_eq!(check("2024-01-02 open Expenses:Food"), not_open);
        assert_eq!(check("1969-12-31 open Assets:Cash"), not_open);
        // an `open` earlier in the batch counts
        let earlier = directive("2024-01-01 open Expenses:Coffee USD");
        assert_eq!(
            ledger.check_open(&open("2024-01-02 open Expenses:Coffee USD"), &[earlier.clone()]),
            OpenCheck {
                already_open: true,
                open_with: None
            }
        );
        assert_eq!(
            ledger.check_open(&open("2024-01-02 open Expenses:Coffee"), &[earlier]).open_with,
            Some(vec!["USD".to_owned()])
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
