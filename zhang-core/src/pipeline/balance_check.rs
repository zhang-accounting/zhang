//! `balance` assertions as a native pipeline stage.

use std::collections::HashMap;

use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{BalanceCheck, Directive, Flag, Posting, Spanned, Transaction, ZhangString};

use super::balance::{exceeds_tolerance, AccountStates, UnitBalances};
use super::{ProcessStage, StageContext};
use crate::ZhangResult;

/// validates every `BalanceCheck` against the account's balance at that point of
/// the stream — including the padding transactions [`PadStage`](crate::pipeline::PadStage)
/// inserted before it — and reports breaches through the stage error channel.
///
/// Every check, passing or not, inserts its correcting transaction (flag `C`,
/// a single posting of the distance) right after itself, so the account sits at
/// the asserted amount from there on; the store fold books it like any other
/// transaction.
pub struct BalanceCheckStage;

impl ProcessStage for BalanceCheckStage {
    fn name(&self) -> &str {
        "balance-check"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut balances = UnitBalances::for_stage(ctx);
        let mut accounts = AccountStates::default();
        let mut ret = Vec::with_capacity(directives.len());

        for directive in directives {
            let correction = match &directive.data {
                Directive::Open(open) => {
                    accounts.apply(&directive.data);
                    balances.apply_open(open);
                    None
                }
                Directive::Close(_) => {
                    accounts.apply(&directive.data);
                    None
                }
                Directive::Commodity(commodity) => {
                    balances.apply_commodity(commodity, ctx.options);
                    None
                }
                Directive::Transaction(txn) => {
                    balances.apply_transaction(txn);
                    None
                }
                Directive::BalanceCheck(check) => {
                    let account_name = || HashMap::from([("account_name".to_owned(), check.account.name().to_owned())]);
                    for (kind, _) in accounts.errors(&[&check.account]) {
                        ctx.emit_error(kind, directive.span.clone(), account_name());
                    }
                    let distance = balances.distance(&check.account, &check.amount);
                    if exceeds_tolerance(&distance.number, check.tolerance.as_ref()) {
                        ctx.emit_error(ErrorKind::AccountBalanceCheckError, directive.span.clone(), account_name());
                    }
                    let txn = correcting_transaction(check, distance);
                    balances.apply_transaction(&txn);
                    Some(Spanned::new(Directive::Transaction(txn), directive.span.clone()))
                }
                _ => None,
            };
            ret.push(directive);
            ret.extend(correction);
        }
        Ok(ret)
    }
}

fn correcting_transaction(check: &BalanceCheck, distance: Amount) -> Transaction {
    Transaction {
        date: check.date.clone(),
        flag: Some(Flag::BalanceCheck),
        payee: Some(ZhangString::quote("Balance Check")),
        narration: Some(ZhangString::quote(check.account.name())),
        tags: Default::default(),
        links: Default::default(),
        postings: vec![Posting {
            flag: None,
            account: check.account.clone(),
            units: Some(distance),
            cost: None,
            price: None,
            comment: None,
        }],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Directive, Flag};

    use crate::pipeline::test::run_builtin_stages;

    /// the distances booked by the checks' correcting transactions, in stream order
    fn check_distances(directives: &[Directive]) -> Vec<Amount> {
        directives
            .iter()
            .filter_map(|it| match it {
                Directive::Transaction(txn) if txn.flag == Some(Flag::BalanceCheck) => txn.postings[0].units.clone(),
                _ => None,
            })
            .collect()
    }

    fn cny(number: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), "CNY")
    }

    #[test]
    fn should_pass_and_fail_within_tolerance() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 10.004 CNY
              Equity:Open
            2023-01-02 balance Assets:A 10 ~ 0.01 CNY
            2023-01-03 balance Assets:A 10 CNY
            2023-01-04 balance Assets:A 10.01 ~ 0.01 CNY
            2023-01-05 balance Assets:A 9.99 ~ 0.01 CNY
        "#});

        // 10.004 vs 10 is within 0.01; the correcting transaction books the
        // distance, so the account sits at each asserted amount afterwards
        assert_eq!(check_distances(&directives), vec![cny("-0.004"), cny("0"), cny("0.01"), cny("-0.02")]);
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);

        let check = directives
            .iter()
            .find(|it| matches!(it, Directive::Transaction(txn) if txn.flag == Some(Flag::BalanceCheck)))
            .unwrap();
        let Directive::Transaction(check) = check else { unreachable!() };
        assert_eq!(check.payee.as_ref().unwrap().as_str(), "Balance Check");
        assert_eq!(check.narration.as_ref().unwrap().as_str(), "Assets:A");
        assert_eq!(check.postings.len(), 1);
    }

    #[test]
    fn should_see_the_padding_transaction_of_the_same_day() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-02-01 balance Assets:A 100 CNY with pad Equity:Open
            2023-02-01 balance Assets:A 100 CNY
            2023-02-01 balance Equity:Open -100 CNY
        "#});
        assert!(errors.is_empty());
        assert_eq!(check_distances(&directives), vec![cny("0"), cny("0")]);
    }

    #[test]
    fn should_book_inferred_implicit_postings_and_skip_rejected_transactions() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Assets:B
            1970-01-01 open Equity:Open
            2023-01-01 * "implicit posting"
              Assets:A
              Equity:Open -10 CNY
            2023-01-02 * "rejected: two implicit postings"
              Assets:A
              Assets:B
              Equity:Open 10 CNY
            2023-01-03 balance Assets:A 10 CNY
        "#});
        assert!(errors.is_empty());
        assert_eq!(check_distances(&directives), vec![cny("0")]);
    }

    #[test]
    fn should_report_missing_and_closed_accounts() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Closed
            1970-01-02 close Assets:Closed
            2023-01-01 balance Assets:Missing 0 CNY
            2023-01-02 balance Assets:Closed 0 CNY
            2023-01-03 balance Assets:Later 0 CNY
            2023-01-04 open Assets:Later
            2023-01-04 balance Assets:Later 0 CNY
        "#});
        assert_eq!(
            errors,
            vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountClosed, ErrorKind::AccountDoesNotExist]
        );
    }
}
