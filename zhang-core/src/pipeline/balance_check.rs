//! `balance` assertions as a native pipeline stage.

use std::collections::{HashMap, HashSet};

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, BalancePad, Date, Directive, SpanInfo, Spanned, Transaction};

use super::balance::{exceeds_tolerance, UnitBalances};
use super::{AssertionOutcome, ProcessStage, StageContext};
use crate::booking::written_groups;
use crate::data_type::Dialect;
use crate::ledger::Ledger;
use crate::utils::read_time;
use crate::ZhangResult;

/// validates every balance assertion, `balance` and `balance ... with pad`, against the account's
/// balance — the sum of the postings of the account and all its sub-accounts, as in beancount,
/// including the padding transactions [`PadStage`](crate::pipeline::PadStage) inserted — and reports
/// breaches through the stage error channel.
///
/// A `balance` is checked where it stands in the stream. A `balance ... with pad` is checked once every
/// balance entry of its time is applied, its own padding and those of other pads included: a later
/// `balance ... with pad` of a sub-account at the same time changes the balance it asserts, and must
/// not leave it silently false.
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
        let mut ignored_times = IgnoredTimes::of(&directives, ctx.dialect());
        let mut balances = UnitBalances::for_stage(ctx);
        // the `balance ... with pad` directives of the balance entries being applied, checked after the last one
        let mut pads: Vec<(&BalancePad, &SpanInfo)> = vec![];
        let mut pads_at = None;

        for directive in &directives {
            if !pads.is_empty() && !(Ledger::is_balance_entry(&directive.data) && directive.datetime() == pads_at) {
                for (pad, span) in pads.drain(..) {
                    check(ctx, &balances, &pad.account, &pad.amount, None, span);
                }
            }
            match &directive.data {
                Directive::Open(open) => balances.apply_open(open),
                Directive::Commodity(commodity) => balances.apply_commodity(commodity, ctx.options),
                Directive::Transaction(txn) => {
                    let units = balances.apply_transaction(txn);
                    ignored_times.transaction(directive, txn, &units);
                }
                Directive::BalanceCheck(check_directive) => {
                    check(
                        ctx,
                        &balances,
                        &check_directive.account,
                        &check_directive.amount,
                        check_directive.tolerance.as_ref(),
                        &directive.span,
                    );
                }
                Directive::BalancePad(pad) => {
                    pads.push((pad, &directive.span));
                    pads_at = directive.datetime();
                }
                _ => {}
            }
        }
        for (pad, span) in pads {
            check(ctx, &balances, &pad.account, &pad.amount, None, span);
        }
        ignored_times.report(ctx, &directives);
        Ok(directives)
    }
}

/// check the assertion at `span` of `amount` on `account` against the balance now, report it if it fails, and
/// record what it found
fn check(ctx: &mut StageContext, balances: &UnitBalances, account: &Account, amount: &Amount, tolerance: Option<&BigDecimal>, span: &SpanInfo) {
    let distance = balances.distance(account, amount);
    let passed = !exceeds_tolerance(&distance.number, tolerance);
    if !passed {
        ctx.emit_error(ErrorKind::AccountBalanceCheckError, span.clone(), account_name(account));
    }
    let balance = balances.amount(account, &amount.commodity);
    ctx.record_assertion(span, AssertionOutcome { balance, passed });
}

/// The balances of a beancount ledger whose `time` zhang ignores ([`ErrorKind::BalanceTimeIgnored`]).
///
/// A `balance` of a beancount ledger, in whichever of its files, is checked at the start of its date, as beancount checks it, and its `time`
/// metadata is plain metadata. Earlier versions of zhang read that `time`, as `H:M:S` (spaces around it trimmed), and
/// checked the balance at it, after the transactions of its day before it. Where those transactions changed what the
/// account and its sub-accounts hold in the balance's commodity, the balance checks a different amount now: it is
/// reported, once. The padding transactions of `pad` directives do not count
struct IgnoredTimes<'a> {
    /// the balances with a time, by their day
    timed: HashMap<NaiveDate, Vec<TimedBalance<'a>>>,
    /// the places of the `pad` directives, which their padding transactions share
    pads: HashSet<(&'a Option<std::path::PathBuf>, usize, usize)>,
}

/// a balance with a time: its index in the stream, its account, commodity and time, and what the transactions of its
/// day before that time changed of it
type TimedBalance<'a> = (usize, &'a Account, &'a str, NaiveTime, BigDecimal);

impl<'a> IgnoredTimes<'a> {
    /// the timed balances among `directives` of a ledger in the format `dialect`: none in a zhang ledger, whose files
    /// are read as zhang text whatever their extension
    fn of(directives: &'a [Spanned<Directive>], dialect: Dialect) -> Self {
        let mut timed: HashMap<NaiveDate, Vec<_>> = HashMap::new();
        for (index, directive) in directives.iter().enumerate() {
            let Directive::BalanceCheck(check) = &directive.data else { continue };
            if !matches!(check.date, Date::Date(_)) || dialect != Dialect::Beancount {
                continue;
            }
            if let Some(time) = check.meta.get_one("time").and_then(|it| read_time(it.as_str())) {
                timed
                    .entry(check.date.naive_date())
                    .or_default()
                    .push((index, &check.account, check.amount.commodity.as_str(), time, BigDecimal::zero()));
            }
        }
        let pads = if timed.is_empty() {
            HashSet::new()
        } else {
            directives
                .iter()
                .filter(|it| matches!(it.data, Directive::Pad(_)))
                .map(|it| (&it.span.filename, it.span.start, it.span.end))
                .collect()
        };
        Self { timed, pads }
    }

    /// a transaction of the stream, with the units it booked to each of its postings as written
    /// (one amount per group of booked legs, [`written_groups`])
    fn transaction(&mut self, directive: &Spanned<Directive>, txn: &Transaction, units: &[Amount]) {
        let Some(at) = directive.datetime() else { return };
        let Some(balances) = self.timed.get_mut(&at.date()) else { return };
        if self.pads.contains(&(&directive.span.filename, directive.span.start, directive.span.end)) {
            return;
        }
        for (_, account, commodity, time, changed) in balances.iter_mut() {
            if at.time() >= *time {
                continue;
            }
            for (group, units) in written_groups(&txn.postings).into_iter().zip(units) {
                let name = group.legs[0].account.name();
                let under = name == account.name() || name.strip_prefix(account.name()).is_some_and(|rest| rest.starts_with(':'));
                if under && units.commodity == *commodity {
                    *changed += &units.number;
                }
            }
        }
    }

    fn report(self, ctx: &mut StageContext, directives: &[Spanned<Directive>]) {
        let mut changed = self.timed.into_values().flatten().filter(|it| !it.4.is_zero()).collect::<Vec<_>>();
        changed.sort_by_key(|it| it.0);
        for (index, account, _, time, _) in changed {
            ctx.emit_error(
                ErrorKind::BalanceTimeIgnored,
                directives[index].span.clone(),
                HashMap::from([
                    ("account_name".to_owned(), account.name().to_owned()),
                    ("time".to_owned(), time.format("%H:%M:%S").to_string()),
                ]),
            );
        }
    }
}

fn account_name(account: &Account) -> HashMap<String, String> {
    HashMap::from([("account_name".to_owned(), account.name().to_owned())])
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
            // a balance after the close only records, as in beancount
            vec![ErrorKind::AccountDoesNotExist, ErrorKind::AccountDoesNotExist]
        );
    }

    #[test]
    fn should_not_report_the_time_of_a_balance_of_a_zhang_file() {
        // in a zhang file, a `time` metadata is plain metadata: zhang never read it as the balance's time
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Expenses:Food
            2024-03-02 08:00:00 * "breakfast" ""
              Assets:A -10 CNY
              Expenses:Food
            2024-03-02 balance Assets:A 0 CNY
              time: "09:30:00"
        "#});
        assert!(errors.is_empty(), "{errors:?}");
    }
}
