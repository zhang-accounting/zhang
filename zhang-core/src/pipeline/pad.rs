//! `balance ... with pad ...` as a native pipeline stage.

use std::collections::HashMap;

use bigdecimal::Zero;
use zhang_ast::amount::Amount;
use zhang_ast::{BalancePad, Directive, Flag, Posting, Spanned, Transaction, ZhangString};

use super::balance::{AccountStates, UnitBalances};
use super::{ProcessStage, StageContext};
use crate::ZhangResult;

/// sizes every `BalancePad` against the account's balance at that point of the
/// stream and inserts the padding transaction (flag `P`) right after it.
///
/// The balance is the true one, as in beancount: the sum of the postings of the
/// account and all its sub-accounts. A `balance` assertion before the pad changes no
/// balance, even when it fails, so a pad brings the account to its amount from where
/// the postings left it. A pad of a parent account books the difference to the
/// parent account itself.
/// The pad directive stays in the stream (like beancount's `Pad` entry); the
/// store fold books only the synthesized transaction. A pad already at its
/// target amount synthesizes nothing.
pub struct PadStage;

impl ProcessStage for PadStage {
    fn name(&self) -> &str {
        "balance-pad"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut balances = UnitBalances::for_stage(ctx);
        let mut accounts = AccountStates::default();
        let mut ret = Vec::with_capacity(directives.len());

        for directive in directives {
            let padding = match &directive.data {
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
                Directive::BalancePad(pad) => {
                    for (kind, account) in accounts.errors(&[&pad.account, &pad.pad]) {
                        ctx.emit_error(
                            kind,
                            directive.span.clone(),
                            HashMap::from([("account_name".to_owned(), account.name().to_owned())]),
                        );
                    }
                    // nothing to pad: no transaction (beancount does the same)
                    let distance = balances.distance(&pad.account, &pad.amount);
                    (!distance.number.is_zero()).then(|| {
                        let txn = padding_transaction(pad, distance);
                        balances.apply_transaction(&txn);
                        Spanned::new(Directive::Transaction(txn), directive.span.clone())
                    })
                }
                _ => None,
            };
            ret.push(directive);
            ret.extend(padding);
        }
        Ok(ret)
    }
}

/// the pad account's leg is left implicit: it is inferred from the padded one
fn padding_transaction(pad: &BalancePad, distance: Amount) -> Transaction {
    Transaction {
        date: pad.date.clone(),
        flag: Some(Flag::BalancePad),
        payee: Some(ZhangString::quote("Balance Pad")),
        narration: Some(ZhangString::quote(format!("pad {} to {}", pad.account.name(), pad.pad.name()))),
        tags: Default::default(),
        links: Default::default(),
        postings: vec![
            Posting {
                flag: None,
                account: pad.account.clone(),
                units: Some(distance),
                cost: None,
                price: None,
                comment: None,
                meta: Default::default(),
            },
            Posting {
                flag: None,
                account: pad.pad.clone(),
                units: None,
                cost: None,
                price: None,
                comment: None,
                meta: Default::default(),
            },
        ],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod test {
    use bigdecimal::BigDecimal;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Directive, Flag, Transaction};

    use crate::pipeline::test::run_builtin_stages;

    fn synthesized(directives: &[Directive], flag: Flag) -> Vec<&Transaction> {
        directives
            .iter()
            .filter_map(|it| match it {
                Directive::Transaction(txn) if txn.flag == Some(flag.clone()) => Some(txn),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn should_pad_the_distance_to_the_target_amount() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 30 CNY
              Equity:Open
            2023-02-01 balance Assets:A 100 CNY with pad Equity:Open
        "#});
        assert!(errors.is_empty());

        let pads = synthesized(&directives, Flag::BalancePad);
        assert_eq!(pads.len(), 1);
        let pad = pads[0];
        assert_eq!(pad.payee.as_ref().unwrap().as_str(), "Balance Pad");
        assert_eq!(pad.narration.as_ref().unwrap().as_str(), "pad Assets:A to Equity:Open");
        assert_eq!(pad.postings[0].account.name(), "Assets:A");
        assert_eq!(pad.postings[0].units, Some(Amount::new(BigDecimal::from(70), "CNY")));
        assert_eq!(pad.postings[1].account.name(), "Equity:Open");
        assert_eq!(pad.postings[1].units, None);

        // the pad directive stays; its transaction follows it directly
        let pad_position = directives.iter().position(|it| matches!(it, Directive::BalancePad(_))).unwrap();
        assert!(matches!(&directives[pad_position + 1], Directive::Transaction(txn) if txn.flag == Some(Flag::BalancePad)));
    }

    #[test]
    fn should_size_pad_from_the_true_balance_after_a_failing_check() {
        // the failing check changes no balance: the account holds 50 when padded to 150
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 50 CNY
              Equity:Open
            2023-01-02 balance Assets:A 100 CNY
            2023-01-03 balance Assets:A 150 CNY with pad Equity:Open
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        let pads = synthesized(&directives, Flag::BalancePad);
        assert_eq!(pads[0].postings[0].units, Some(Amount::new(BigDecimal::from(100), "CNY")));
    }

    #[test]
    fn should_size_each_pad_from_the_balance_the_postings_reach() {
        // two pads of the same account, with a transaction and a failing check between them
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-01 open Expenses:Food
            2023-01-01 balance Assets:A 100 CNY with pad Equity:Open
            2023-01-02 * ""
              Assets:A -30 CNY
              Expenses:Food
            2023-01-03 balance Assets:A 100 CNY
            2023-01-04 balance Assets:A 120 CNY with pad Equity:Open
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        let pads = synthesized(&directives, Flag::BalancePad)
            .into_iter()
            .map(|pad| pad.postings[0].units.clone().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pads, vec![Amount::new(BigDecimal::from(100), "CNY"), Amount::new(BigDecimal::from(50), "CNY")]);
    }

    #[test]
    fn should_pad_a_parent_account_from_the_balance_of_its_sub_accounts() {
        // the sub-accounts hold 100 of the 150: the parent account itself gets the other 50
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Bank
            1970-01-01 open Assets:Bank:Checking
            1970-01-01 open Assets:Bank:Savings
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:Bank:Checking 60 CNY
              Assets:Bank:Savings 40 CNY
              Equity:Open
            2023-01-02 balance Assets:Bank 150 CNY with pad Equity:Open
            2023-01-03 balance Assets:Bank 150 CNY
        "#});
        assert!(errors.is_empty());
        let pads = synthesized(&directives, Flag::BalancePad);
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0].postings[0].account.name(), "Assets:Bank");
        assert_eq!(pads[0].postings[0].units, Some(Amount::new(BigDecimal::from(50), "CNY")));
    }

    #[test]
    fn should_not_pad_an_account_already_at_its_target() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 10 CNY
              Equity:Open
            2023-01-02 balance Assets:A 10 CNY with pad Equity:Missing
            2023-01-03 balance Assets:A 10 CNY
        "#});
        assert!(synthesized(&directives, Flag::BalancePad).is_empty());
        // the pad directive stays and its account errors are still reported
        assert!(directives.iter().any(|it| matches!(it, Directive::BalancePad(_))));
        assert_eq!(errors, vec![ErrorKind::AccountDoesNotExist]);
    }

    #[test]
    fn should_report_missing_and_closed_accounts() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Assets:Closed
            1970-01-02 close Assets:Closed
            2023-01-01 balance Assets:A 10 CNY with pad Equity:Missing
            2023-01-02 balance Assets:Closed 10 CNY with pad Assets:Missing
        "#});
        assert_eq!(
            errors,
            vec![
                ErrorKind::AccountDoesNotExist,
                ErrorKind::AccountDoesNotExist,
                ErrorKind::AccountClosed,
                // the check stage does not re-report the pad's accounts
            ]
        );
    }
}
