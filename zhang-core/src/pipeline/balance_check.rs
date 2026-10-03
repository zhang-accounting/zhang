//! `balance` assertions as a native pipeline stage.

use std::collections::HashMap;

use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Spanned};

use super::balance::{exceeds_tolerance, AccountStates, UnitBalances};
use super::{AssertionOutcome, ProcessStage, StageContext};
use crate::ZhangResult;

/// validates every `BalanceCheck` against the account's balance at that point of
/// the stream — the sum of the postings of the account and all its sub-accounts, as
/// in beancount, including the padding transactions
/// [`PadStage`](crate::pipeline::PadStage) inserted before it — and reports
/// breaches through the stage error channel.
///
/// A check only checks, as in beancount: passing or failing, it books nothing and
/// changes no balance, so the books keep netting to zero. A failing check is an
/// [`ErrorKind::AccountBalanceCheckError`] and nothing else; `balance ... with pad`
/// is how a balance is corrected on purpose. The stream leaves the stage unchanged;
/// what each check found is recorded with [`StageContext::record_assertion`].
pub struct BalanceCheckStage;

impl ProcessStage for BalanceCheckStage {
    fn name(&self) -> &str {
        "balance-check"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut balances = UnitBalances::for_stage(ctx);
        let mut accounts = AccountStates::default();

        for directive in &directives {
            match &directive.data {
                Directive::Open(open) => {
                    accounts.apply(&directive.data);
                    balances.apply_open(open);
                }
                Directive::Close(_) => accounts.apply(&directive.data),
                Directive::Commodity(commodity) => balances.apply_commodity(commodity, ctx.options),
                Directive::Transaction(txn) => balances.apply_transaction(txn),
                Directive::BalanceCheck(check) => {
                    let account_name = || HashMap::from([("account_name".to_owned(), check.account.name().to_owned())]);
                    for (kind, _) in accounts.errors(&[&check.account]) {
                        ctx.emit_error(kind, directive.span.clone(), account_name());
                    }
                    let distance = balances.distance(&check.account, &check.amount);
                    let passed = !exceeds_tolerance(&distance.number, check.tolerance.as_ref());
                    if !passed {
                        ctx.emit_error(ErrorKind::AccountBalanceCheckError, directive.span.clone(), account_name());
                    }
                    let balance = balances.amount(&check.account, &check.amount.commodity);
                    ctx.record_assertion(&directive.span, AssertionOutcome { balance, passed });
                }
                _ => {}
            }
        }
        Ok(directives)
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

    use crate::pipeline::test::{run_builtin_stages, run_builtin_stages_with_assertions};
    use crate::pipeline::AssertionOutcome;

    fn cny(number: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), "CNY")
    }

    fn outcome(balance: &str, passed: bool) -> AssertionOutcome {
        AssertionOutcome { balance: cny(balance), passed }
    }

    fn transactions(directives: &[Directive]) -> usize {
        directives.iter().filter(|it| matches!(it, Directive::Transaction(_))).count()
    }

    #[test]
    fn should_pass_and_fail_within_tolerance_without_moving_the_balance() {
        let (directives, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 10.004 CNY
              Equity:Open
            2023-01-02 balance Assets:A 10 ~ 0.01 CNY
            2023-01-03 balance Assets:A 10 CNY
            2023-01-04 balance Assets:A 10.01 ~ 0.01 CNY
            2023-01-05 balance Assets:A 9.99 ~ 0.01 CNY
            2023-01-06 balance Assets:A 10.004 CNY
        "#});

        // every check sees the same 10.004: none of them, passing or failing, moves the balance
        assert_eq!(
            outcomes,
            vec![
                outcome("10.004", true),
                outcome("10.004", false),
                outcome("10.004", true),
                outcome("10.004", false),
                outcome("10.004", true),
            ]
        );
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError, ErrorKind::AccountBalanceCheckError]);
        // the stream leaves the stage as it came: no transaction is added
        assert_eq!(transactions(&directives), 1);
        assert!(!directives
            .iter()
            .any(|it| matches!(it, Directive::Transaction(txn) if txn.flag == Some(Flag::BalanceCheck))));
    }

    #[test]
    fn should_see_the_padding_transaction_of_the_same_day() {
        let (_, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-02-01 balance Assets:A 100 CNY with pad Equity:Open
            2023-02-01 balance Assets:A 100 CNY
            2023-02-01 balance Equity:Open -100 CNY
        "#});
        assert!(errors.is_empty());
        assert_eq!(outcomes, vec![outcome("100", true), outcome("-100", true)]);
    }

    #[test]
    fn should_check_against_the_balance_left_by_an_earlier_failing_check() {
        // the second check is checked against the postings (165), not against the 200 the first one asserted
        let (_, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Income:X
            2024-01-01 * "x"
              Assets:A 165 CNY
              Income:X
            2024-01-02 balance Assets:A 200 CNY
            2024-01-03 balance Assets:A 200 CNY
            2024-01-04 balance Assets:A 165 CNY
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError, ErrorKind::AccountBalanceCheckError]);
        assert_eq!(outcomes, vec![outcome("165", false), outcome("165", false), outcome("165", true)]);
    }

    #[test]
    fn should_check_each_currency_of_an_account_on_its_own() {
        let (_, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 10 CNY
              Equity:Open
            2023-01-02 balance Assets:A 10 CNY
            2023-01-02 balance Assets:A 0 USD
            2023-01-02 balance Assets:A 5 USD
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(
            outcomes,
            vec![
                outcome("10", true),
                AssertionOutcome {
                    balance: Amount::new(BigDecimal::from(0), "USD"),
                    passed: true
                },
                AssertionOutcome {
                    balance: Amount::new(BigDecimal::from(0), "USD"),
                    passed: false
                },
            ]
        );
    }

    #[test]
    fn should_check_an_account_with_its_sub_accounts() {
        let (_, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
            1970-01-01 open Assets:Bank
            1970-01-01 open Assets:Bank:Checking
            1970-01-01 open Assets:Bank:Savings
            1970-01-01 open Assets:Banking
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:Bank 5 CNY
              Assets:Bank:Checking 60 CNY
              Assets:Bank:Savings 40 CNY
              Assets:Banking 1000 CNY
              Equity:Open
            2023-01-02 balance Assets:Bank 105 CNY
            2023-01-02 balance Assets:Bank:Checking 60 CNY
            2023-01-02 balance Assets:Bank 5 CNY
        "#});
        // `Assets:Banking` is no sub-account of `Assets:Bank`
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(outcomes, vec![outcome("105", true), outcome("60", true), outcome("105", false)]);
    }

    #[test]
    fn should_book_inferred_implicit_postings_and_skip_rejected_transactions() {
        let (_, errors, outcomes) = run_builtin_stages_with_assertions(indoc! {r#"
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
        assert_eq!(outcomes, vec![outcome("10", true)]);
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
