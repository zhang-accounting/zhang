//! The account lifecycle ([`AccountLifecycle`]), the one rule of when an account is active, and the pipeline stage that
//! reports the references to inactive accounts with it (beancount's `validate_active_accounts`).

use std::collections::HashMap;

use chrono::NaiveDateTime;
use itertools::Itertools;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Date, Directive, Spanned};

use super::{ProcessStage, StageContext};
use crate::domains::schemas::AccountStatus;
use crate::ZhangResult;

/// An `open` or a `close` of an account, as [`AccountLifecycle`] keeps it.
#[derive(Debug, Clone)]
enum Change {
    /// an `open`, at its date and time
    Open(NaiveDateTime),
    /// a `close`, at its date and time, and its date as written, which tells when it takes effect
    Close { at: NaiveDateTime, date: Date },
}

impl Change {
    /// the date and time of the directive, midnight for a date alone: where it is in the stream
    fn at(&self) -> NaiveDateTime {
        match self {
            Change::Open(at) | Change::Close { at, .. } => *at,
        }
    }
}

/// What the changes of an account up to an instant make of it.
#[derive(Default)]
struct Folded<'a> {
    /// whether an `open` opened it
    opened: bool,
    /// the date of the `close` that closed it since its latest `open`: the first one, as a later one closes nothing
    close: Option<&'a Date>,
}

impl Folded<'_> {
    fn status(&self, at: NaiveDateTime) -> Option<AccountStatus> {
        match self.close {
            Some(close) if close.close_precedes(at) => Some(AccountStatus::Close),
            _ if self.opened => Some(AccountStatus::Open),
            _ => None,
        }
    }
}

/// How a directive uses an account, which decides whether it may follow the account's close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountUse {
    /// It books on the account, moving what it holds: a posting, a `pad`, a `balance ... with pad`. The account must be
    /// active.
    Books,
    /// It only records something about the account: a `balance` without a pad, a `document`, a `note`. As in beancount,
    /// it may follow the close; the account must have been opened.
    Records,
}

/// When each account is active, from its `open` and `close` directives: the one rule every directive that references an
/// account is checked with ([`ActiveAccountsStage`]), and that the host reads with
/// [`Ledger::account_status`](crate::ledger::Ledger::account_status).
///
/// - An account is open from its `open`, which sorts before every other directive of its date and time.
/// - A `close` closes it, but it stays active through the close: a `close` with only a date closes it at the end of that
///   day (24:00), and a `close` with a time at that time ([`Date::close_precedes`]). A directive at the close's own
///   date and time is still within it.
/// - An `open` after a `close` opens the account again. A second `close` of a closed account closes nothing: the first
///   one stands, and the second is an error.
#[derive(Debug, Clone, Default)]
pub struct AccountLifecycle {
    /// the `open` and `close` directives of each account, in stream order, which is the order of their dates and times
    changes: HashMap<String, Vec<Change>>,
}

impl AccountLifecycle {
    /// the lifecycle of the accounts of a sorted stream
    pub fn of<'a>(directives: impl IntoIterator<Item = &'a Spanned<Directive>>) -> Self {
        let mut lifecycle = AccountLifecycle::default();
        for directive in directives {
            lifecycle.apply(&directive.data);
        }
        lifecycle
    }

    /// fold an `open` or a `close`, the next one in stream order; other directives are ignored
    pub fn apply(&mut self, directive: &Directive) {
        let Some(at) = directive.datetime() else { return };
        let (account, change) = match directive {
            Directive::Open(open) => (&open.account, Change::Open(at)),
            Directive::Close(close) => (&close.account, Change::Close { at, date: close.date.clone() }),
            _ => return,
        };
        self.changes.entry(account.name().to_owned()).or_default().push(change);
    }

    /// the changes of `account` dated `at` or before
    fn fold(&self, account: &str, at: NaiveDateTime) -> Folded<'_> {
        let mut folded = Folded::default();
        for change in self.changes.get(account).into_iter().flatten().take_while(|it| it.at() <= at) {
            match change {
                Change::Open(_) => folded = Folded { opened: true, close: None },
                Change::Close { date, .. } => {
                    folded.close.get_or_insert(date);
                }
            }
        }
        folded
    }

    /// The status of `account` at the wall-clock time `at` of the ledger's timezone: `Open` from its `open` until its
    /// close takes effect, `Close` after that, and `None` when neither an `open` nor a `close` of it is in effect.
    pub fn status(&self, account: &str, at: NaiveDateTime) -> Option<AccountStatus> {
        self.fold(account, at).status(at)
    }

    /// The status of `account` once every `open` and `close` of it is in effect, whatever their dates: `Close` when its
    /// latest one closed it.
    pub fn final_status(&self, account: &str) -> Option<AccountStatus> {
        self.status(account, NaiveDateTime::MAX)
    }

    /// The error a directive dated `at` raises by using `account` the way `usage` tells, `None` when it may:
    /// [`AccountDoesNotExist`](ErrorKind::AccountDoesNotExist) when no `open` opened the account by then, and, for a
    /// directive that books, [`AccountClosed`](ErrorKind::AccountClosed) when the account is closed. A directive that
    /// only records something may follow the close.
    pub fn reference_error(&self, account: &Account, at: NaiveDateTime, usage: AccountUse) -> Option<ErrorKind> {
        let folded = self.fold(account.name(), at);
        match (folded.status(at), usage) {
            _ if !folded.opened => Some(ErrorKind::AccountDoesNotExist),
            (Some(AccountStatus::Close), AccountUse::Books) => Some(ErrorKind::AccountClosed),
            _ => None,
        }
    }

    /// The error of a `close` of `account` dated `at`, folded after this check: `AccountDoesNotExist` when no `open`
    /// opened the account, `AccountClosed` when it is closed already, whether or not that close took effect yet.
    pub fn close_error(&self, account: &Account, at: NaiveDateTime) -> Option<ErrorKind> {
        let folded = self.fold(account.name(), at);
        if !folded.opened {
            Some(ErrorKind::AccountDoesNotExist)
        } else if folded.close.is_some() {
            Some(ErrorKind::AccountClosed)
        } else {
            None
        }
    }
}

/// reports every reference to an account that a directive may not make at its date and time, by the rule of
/// [`AccountLifecycle`]. Report-only: the stream is returned unchanged, so such a transaction is still booked, and such a
/// document still listed.
///
/// - a reference to an account with no `open` before it, never opened or opened only later, reports
///   `AccountDoesNotExist`
/// - a directive that books ([`AccountUse::Books`]), a transaction, a `pad` or a `balance ... with pad`, reports
///   `AccountClosed` after the account's close took effect: after the day of a `close` with only a date, after the time
///   of one with a time
/// - a directive that only records ([`AccountUse::Records`]), a `balance` without a pad, a `document` or a `note`, may
///   follow the close, as in beancount
/// - a transaction reports each account its postings name once, in the order they first appear in it
/// - a `pad` or `balance ... with pad` reports the account padded and the account it is padded from, every missing one
///   first
/// - a `close` of an account never opened reports `AccountDoesNotExist`, and of a closed one `AccountClosed`
///
/// Each error is reported on the directive's span with an `account_name` meta.
///
/// It runs before [`PadStage`](crate::pipeline::PadStage), so the `P` transactions the pad stage synthesizes from the
/// `pad` and `balance ... with pad` directives, which it reports, do not exist yet here, and nothing is reported twice.
/// Hand-written `P` transactions are checked like any other.
pub struct ActiveAccountsStage;

impl ProcessStage for ActiveAccountsStage {
    fn name(&self) -> &str {
        "active-accounts"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut lifecycle = AccountLifecycle::default();

        for directive in &directives {
            let Some(at) = directive.data.datetime() else { continue };
            let errors: Vec<(ErrorKind, &Account)> = match &directive.data {
                Directive::Close(close) => lifecycle
                    .close_error(&close.account, at)
                    .map(|kind| (kind, &close.account))
                    .into_iter()
                    .collect(),
                Directive::Transaction(txn) => txn
                    .postings
                    .iter()
                    .map(|posting| &posting.account)
                    .unique_by(|account| account.name())
                    .filter_map(|account| lifecycle.reference_error(account, at, AccountUse::Books).map(|kind| (kind, account)))
                    .collect(),
                Directive::Note(note) => reference_errors(&lifecycle, &[&note.account], at, AccountUse::Records),
                Directive::Document(document) => reference_errors(&lifecycle, &[&document.account], at, AccountUse::Records),
                Directive::BalanceCheck(check) => reference_errors(&lifecycle, &[&check.account], at, AccountUse::Records),
                Directive::Pad(pad) => reference_errors(&lifecycle, &[&pad.account, &pad.pad], at, AccountUse::Books),
                Directive::BalancePad(pad) => reference_errors(&lifecycle, &[&pad.account, &pad.pad], at, AccountUse::Books),
                _ => vec![],
            };
            lifecycle.apply(&directive.data);
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

/// the errors of a directive dated `at` that uses `accounts` the way `usage` tells: every missing account, then every
/// closed one
fn reference_errors<'a>(lifecycle: &AccountLifecycle, accounts: &[&'a Account], at: NaiveDateTime, usage: AccountUse) -> Vec<(ErrorKind, &'a Account)> {
    accounts
        .iter()
        .filter_map(|account| lifecycle.reference_error(account, at, usage).map(|kind| (kind, *account)))
        .sorted_by_key(|(kind, _)| *kind != ErrorKind::AccountDoesNotExist)
        .collect()
}

#[cfg(test)]
mod test {
    use chrono::{NaiveDate, NaiveDateTime};
    use indoc::indoc;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::Account;

    use super::{AccountLifecycle, AccountUse};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::domains::schemas::AccountStatus;
    use crate::pipeline::test::run_builtin_stages;

    fn lifecycle(content: &str) -> AccountLifecycle {
        let directives = ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        AccountLifecycle::of(&directives)
    }

    fn account(name: &str) -> Account {
        name.parse().unwrap()
    }

    fn at(day: u32, time: &str) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(1970, 1, day).unwrap().and_time(time.parse().unwrap())
    }

    #[test]
    fn should_follow_account_lifecycle() {
        let lifecycle = lifecycle(indoc! {r#"
            1970-01-01 close Assets:NeverOpened
            1970-01-01 open Assets:A
            1970-01-01 open Assets:Reopened
            1970-01-02 close Assets:A
            1970-01-02 close Assets:Reopened
            1970-01-03 open Assets:Reopened
        "#});

        // a close of an account never opened closes it, but it never was active
        assert_eq!(lifecycle.final_status("Assets:NeverOpened"), Some(AccountStatus::Close));
        assert_eq!(
            lifecycle.reference_error(&account("Assets:NeverOpened"), at(5, "00:00:00"), AccountUse::Books),
            Some(ErrorKind::AccountDoesNotExist)
        );
        assert_eq!(lifecycle.final_status("Assets:A"), Some(AccountStatus::Close));
        assert_eq!(lifecycle.final_status("Assets:Reopened"), Some(AccountStatus::Open));
        assert_eq!(lifecycle.final_status("Assets:Missing"), None);

        // active through the whole day of the close
        let closed = account("Assets:A");
        assert_eq!(lifecycle.reference_error(&closed, at(2, "00:00:00"), AccountUse::Books), None);
        assert_eq!(lifecycle.reference_error(&closed, at(2, "23:59:59"), AccountUse::Books), None);
        assert_eq!(
            lifecycle.reference_error(&closed, at(3, "00:00:00"), AccountUse::Books),
            Some(ErrorKind::AccountClosed)
        );
        assert_eq!(
            lifecycle.reference_error(&account("Assets:Missing"), at(1, "00:00:00"), AccountUse::Books),
            Some(ErrorKind::AccountDoesNotExist)
        );
        // a directive that only records may follow the close, but not use an account never opened
        assert_eq!(lifecycle.reference_error(&closed, at(3, "00:00:00"), AccountUse::Records), None);
        assert_eq!(
            lifecycle.reference_error(&account("Assets:NeverOpened"), at(5, "00:00:00"), AccountUse::Records),
            Some(ErrorKind::AccountDoesNotExist)
        );
        // closed for the day between its close and its reopening, open again after it
        let reopened = account("Assets:Reopened");
        assert_eq!(lifecycle.status("Assets:Reopened", at(2, "12:00:00")), Some(AccountStatus::Open));
        assert_eq!(lifecycle.reference_error(&reopened, at(3, "00:00:00"), AccountUse::Books), None);
        assert_eq!(lifecycle.reference_error(&reopened, at(4, "00:00:00"), AccountUse::Books), None);
    }

    #[test]
    fn should_close_at_the_time_of_a_close_with_one() {
        let lifecycle = lifecycle(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-02 10:00:00 close Assets:A
            1970-01-03 08:00:00 open Assets:A
        "#});
        assert_eq!(lifecycle.status("Assets:A", at(1, "00:00:00")), Some(AccountStatus::Open));
        // up to the time of the close, then closed until it is opened again
        assert_eq!(lifecycle.status("Assets:A", at(2, "10:00:00")), Some(AccountStatus::Open));
        assert_eq!(lifecycle.status("Assets:A", at(2, "10:00:01")), Some(AccountStatus::Close));
        assert_eq!(lifecycle.status("Assets:A", at(3, "07:59:59")), Some(AccountStatus::Close));
        assert_eq!(lifecycle.status("Assets:A", at(3, "08:00:00")), Some(AccountStatus::Open));
        // before its first open
        assert_eq!(
            lifecycle.status("Assets:A", NaiveDate::from_ymd_opt(1969, 12, 31).unwrap().and_hms_opt(23, 0, 0).unwrap()),
            None
        );
    }

    #[test]
    fn should_keep_the_first_close_of_a_closed_account() {
        let lifecycle = lifecycle(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-02 close Assets:A
            1970-01-05 close Assets:A
        "#});
        // the second close closes nothing: the account stays closed from the end of the first close day
        assert_eq!(
            lifecycle.reference_error(&account("Assets:A"), at(3, "00:00:00"), AccountUse::Books),
            Some(ErrorKind::AccountClosed)
        );
        assert_eq!(lifecycle.close_error(&account("Assets:A"), at(5, "00:00:00")), Some(ErrorKind::AccountClosed));
        assert_eq!(
            lifecycle.close_error(&account("Assets:B"), at(5, "00:00:00")),
            Some(ErrorKind::AccountDoesNotExist)
        );
    }

    #[test]
    fn should_report_closes_of_accounts_never_opened_or_closed_already() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 close Assets:Missing
            1970-01-02 close Assets:A
            1970-01-02 close Assets:A
            1970-01-03 close Assets:Missing
        "#});
        assert_eq!(
            errors,
            vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountClosed, ErrorKind::AccountDoesNotExist]
        );
    }

    #[test]
    fn should_check_every_directive_at_its_own_date_and_time() {
        let directives = |content: &str| run_builtin_stages(content).1;
        // on the close day: every directive may still use the account
        let errors = directives(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-05 close Assets:A
            1970-01-05 document Assets:A "a.pdf"
            1970-01-05 note Assets:A "closed today"
            1970-01-05 pad Equity:Open Assets:A
            1970-01-05 12:00:00 balance Assets:A 0 CNY
            1970-01-05 13:00:00 balance Assets:A 0 CNY with pad Equity:Open
            1970-01-06 balance Equity:Open 0 CNY
        "#});
        assert_eq!(errors, vec![ErrorKind::UnusedPad]);
        // the day after: what books reports it, the pad and the `balance ... with pad`; the document, the note and the
        // balance only record, and may follow the close
        let errors = directives(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-05 close Assets:A
            1970-01-06 document Assets:A "a.pdf"
            1970-01-06 note Assets:A "closed yesterday"
            1970-01-06 pad Equity:Open Assets:A
            1970-01-06 12:00:00 balance Assets:A 0 CNY
            1970-01-06 13:00:00 balance Assets:A 0 CNY with pad Equity:Open
            1970-01-07 balance Equity:Open 0 CNY
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountClosed, ErrorKind::AccountClosed, ErrorKind::UnusedPad]);
    }

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
