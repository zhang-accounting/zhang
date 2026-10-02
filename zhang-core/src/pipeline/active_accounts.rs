//! references to inactive accounts as a native pipeline stage (beancount's `validate_active_accounts`).

use std::collections::HashMap;

use itertools::Itertools;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Directive, Spanned};

use super::balance::AccountStates;
use super::{ProcessStage, StageContext};
use crate::ZhangResult;

/// reports the transaction postings and notes that reference an account which is not active at
/// their date. Report-only: the stream is returned unchanged, so such a transaction is still booked.
///
/// An account is active from its `open` (which sorts first within its datetime) through the whole
/// day of its `close`, as in beancount, which sorts `close` after every other entry of its day:
/// - a posting to an account with no `open` before the transaction — never opened, or opened only
///   later — reports `AccountDoesNotExist`
/// - a posting dated after the day the account was closed reports `AccountClosed`
/// - a `note` reports `AccountDoesNotExist` only: like beancount, it may follow a `close`
///
/// Each account is reported once per directive, on the directive's span with an `account_name`
/// meta, in the order the accounts first appear in it.
///
/// It runs before [`PadStage`](crate::pipeline::PadStage) and
/// [`BalanceCheckStage`](crate::pipeline::BalanceCheckStage): those report the accounts of their
/// own directives, and the `P`/`C` transactions they synthesize from them do not exist yet here, so
/// nothing is reported twice. Hand-written `P`/`C` transactions are checked like any other.
pub struct ActiveAccountsStage;

impl ProcessStage for ActiveAccountsStage {
    fn name(&self) -> &str {
        "active-accounts"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut accounts = AccountStates::default();

        for directive in &directives {
            let errors: Vec<(ErrorKind, &Account)> = match &directive.data {
                Directive::Open(_) | Directive::Close(_) => {
                    accounts.apply(&directive.data);
                    vec![]
                }
                Directive::Transaction(txn) => {
                    let date = txn.date.naive_date();
                    txn.postings
                        .iter()
                        .map(|posting| &posting.account)
                        .unique_by(|account| account.name())
                        .filter_map(|account| accounts.inactive_error(account, date).map(|kind| (kind, account)))
                        .collect()
                }
                Directive::Note(note) if !accounts.exists(&note.account) => vec![(ErrorKind::AccountDoesNotExist, &note.account)],
                _ => vec![],
            };
            for (kind, account) in errors {
                ctx.emit_error(
                    kind,
                    directive.span.clone(),
                    HashMap::from([("account_name".to_owned(), account.name().to_owned())]),
                );
            }
        }
        Ok(directives)
    }
}

#[cfg(test)]
mod test {
    use indoc::indoc;
    use zhang_ast::error::ErrorKind;

    use crate::pipeline::test::run_builtin_stages;

    #[test]
    fn should_report_missing_and_closed_posting_accounts() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Cash
            1970-01-01 open Expenses:Old
            1970-01-05 close Expenses:Old
            1970-01-05 * "on the close day: still active"
              Assets:Cash -1 CNY
              Expenses:Old 1 CNY
            1970-01-06 * "after the close day"
              Assets:Cash -1 CNY
              Expenses:Old 1 CNY
            1970-01-07 * "never opened, twice"
              Assets:Cash -2 CNY
              Expenses:Fodo 1 CNY
              Expenses:Fodo 1 CNY
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountClosed, ErrorKind::AccountDoesNotExist]);
    }

    #[test]
    fn should_check_hand_written_balance_transactions_but_not_synthesized_ones() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-02 P "hand-written padding"
              Assets:A 1 CNY
              Equity:Typo -1 CNY
            1970-01-03 balance Assets:A 5 CNY with pad Equity:Missing
        "#});
        // the transaction's typo, then the pad stage's report of its own directive, once
        assert_eq!(errors, vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountDoesNotExist]);
    }

    #[test]
    fn should_report_notes_before_open_but_not_after_close() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 note Assets:Later "before its open"
            1970-01-01 note Assets:Missing "never opened"
            1970-01-02 open Assets:Later
            1970-01-03 close Assets:Later
            1970-01-04 note Assets:Later "after its close"
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountDoesNotExist]);
    }
}
