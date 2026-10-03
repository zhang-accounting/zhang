//! `pad` and `balance ... with pad ...` as a native pipeline stage.

use std::collections::{HashMap, HashSet};

use bigdecimal::Zero;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Date, Directive, Flag, Pad, Posting, SpanInfo, Spanned, Transaction, ZhangString};

use super::balance::{AccountStates, UnitBalances};
use super::{ProcessStage, StageContext};
use crate::ZhangResult;

/// inserts the padding transactions (flag `P`) of `pad` and `balance ... with pad` directives.
///
/// A pad is sized from the true balance, as in beancount: the sum of the postings of the account and
/// all its sub-accounts. A `balance` assertion changes no balance, even when it fails, so a pad brings
/// the account to the asserted amount from where the postings left it. A pad of a parent account books
/// the difference to the parent account itself. A pad brings the account to exactly the asserted
/// amount: zhang infers no tolerance, and pads even within an explicit `~` tolerance.
///
/// - A `pad` serves the next balance assertion of its account (the account itself, not a sub-account)
///   in each currency, until the account's next `pad`, as beancount 3.2.3's `pad` plugin does. Its
///   padding transaction is dated on the `pad`, so the balances between the `pad` and the assertion
///   include it. A `pad` that pads nothing — no later assertion of its account needs it — is reported
///   as [`ErrorKind::UnusedPad`].
/// - A `balance ... with pad` pads its own assertion: its padding transaction is dated on it and goes
///   right after it. A `pad` before it is served first.
///
/// An account already at the asserted amount gets no padding transaction. The pad directives stay in
/// the stream (like beancount's `Pad` entry); the store fold books only the padding transactions.
pub struct PadStage;

/// a `pad` waiting for the assertions it serves
struct ActivePad {
    pad: Pad,
    span: SpanInfo,
    /// its place in the stream, the order unused pads are reported in
    position: usize,
    /// the currencies whose next assertion it served already
    served: HashSet<String>,
    /// whether it padded anything
    used: bool,
}

impl ProcessStage for PadStage {
    fn name(&self) -> &str {
        "balance-pad"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut balances = UnitBalances::for_stage(ctx);
        let mut accounts = AccountStates::default();
        // the `pad` waiting for assertions, by the account it pads
        let mut active: HashMap<String, ActivePad> = HashMap::new();
        let mut ret = Vec::with_capacity(directives.len());
        // the padding transactions of `pad` directives, dated on their `pad`: the re-sort after the stage puts
        // them there, after the balance assertions of that day, as beancount orders a day
        let mut paddings = vec![];

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
                Directive::Pad(pad) => {
                    report_account_errors(ctx, &accounts, &[&pad.account, &pad.pad], &directive.span);
                    let waiting = ActivePad {
                        pad: pad.clone(),
                        span: directive.span.clone(),
                        position: ret.len(),
                        served: HashSet::new(),
                        used: false,
                    };
                    if let Some(replaced) = active.insert(pad.account.name().to_owned(), waiting) {
                        report_unused(ctx, replaced);
                    }
                    None
                }
                Directive::BalanceCheck(check) => {
                    paddings.extend(serve(&mut active, &mut balances, &check.account, &check.amount));
                    None
                }
                Directive::BalancePad(pad) => {
                    report_account_errors(ctx, &accounts, &[&pad.account, &pad.pad], &directive.span);
                    paddings.extend(serve(&mut active, &mut balances, &pad.account, &pad.amount));
                    // nothing to pad: no transaction (beancount does the same)
                    let distance = balances.distance(&pad.account, &pad.amount);
                    (!distance.number.is_zero()).then(|| {
                        let txn = padding_transaction(pad.date.clone(), &pad.account, &pad.pad, distance);
                        balances.apply_transaction(&txn);
                        Spanned::new(Directive::Transaction(txn), directive.span.clone())
                    })
                }
                _ => None,
            };
            ret.push(directive);
            ret.extend(padding);
        }
        let mut waiting = active.into_values().collect::<Vec<_>>();
        waiting.sort_by_key(|it| it.position);
        for waiting in waiting {
            report_unused(ctx, waiting);
        }
        ret.extend(paddings);
        Ok(ret)
    }
}

/// serve an assertion of `account` with the `pad` waiting for it, if any: the first assertion of each currency
/// after the `pad` gets the padding it needs. Returns the padding transaction
fn serve(active: &mut HashMap<String, ActivePad>, balances: &mut UnitBalances, account: &Account, asserted: &Amount) -> Option<Spanned<Directive>> {
    let waiting = active.get_mut(account.name())?;
    if !waiting.served.insert(asserted.commodity.clone()) {
        return None;
    }
    let distance = balances.distance(account, asserted);
    if distance.number.is_zero() {
        return None;
    }
    waiting.used = true;
    let txn = padding_transaction(waiting.pad.date.clone(), &waiting.pad.account, &waiting.pad.pad, distance);
    balances.apply_transaction(&txn);
    Some(Spanned::new(Directive::Transaction(txn), waiting.span.clone()))
}

fn report_account_errors(ctx: &mut StageContext, accounts: &AccountStates, references: &[&Account], span: &SpanInfo) {
    for (kind, account) in accounts.errors(references) {
        ctx.emit_error(kind, span.clone(), HashMap::from([("account_name".to_owned(), account.name().to_owned())]));
    }
}

/// a `pad` that padded nothing is an error, as in beancount
fn report_unused(ctx: &mut StageContext, waiting: ActivePad) {
    if !waiting.used {
        ctx.emit_error(
            ErrorKind::UnusedPad,
            waiting.span,
            HashMap::from([("account_name".to_owned(), waiting.pad.account.name().to_owned())]),
        );
    }
}

/// the pad account's leg is left implicit: it is inferred from the padded one
fn padding_transaction(date: Date, account: &Account, pad: &Account, distance: Amount) -> Transaction {
    Transaction {
        date,
        flag: Some(Flag::BalancePad),
        payee: Some(ZhangString::quote("Balance Pad")),
        narration: Some(ZhangString::quote(format!("pad {} to {}", account.name(), pad.name()))),
        tags: Default::default(),
        links: Default::default(),
        postings: vec![
            Posting {
                flag: None,
                account: account.clone(),
                units: Some(distance),
                cost: None,
                price: None,
                comment: None,
                meta: Default::default(),
            },
            Posting {
                flag: None,
                account: pad.clone(),
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

    /// (date, padded account, units, account padded from) of the padding transactions, in stream order
    fn paddings(directives: &[Directive]) -> Vec<(String, String, String, String)> {
        synthesized(directives, Flag::BalancePad)
            .into_iter()
            .map(|txn| {
                let units = txn.postings[0].units.as_ref().unwrap();
                (
                    txn.date.naive_date().to_string(),
                    txn.postings[0].account.name().to_owned(),
                    format!("{} {}", units.number, units.commodity),
                    txn.postings[1].account.name().to_owned(),
                )
            })
            .collect()
    }

    fn padding(date: &str, account: &str, units: &str, from: &str) -> (String, String, String, String) {
        (date.to_owned(), account.to_owned(), units.to_owned(), from.to_owned())
    }

    #[test]
    fn should_date_the_padding_of_a_pad_on_the_pad() {
        // the padding is sized from the balance at the assertion, after the transaction between them, and dated
        // on the `pad`: the check of the pad account between them sees it
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-01 open Expenses:Food
            2024-01-01 pad Assets:A Equity:Open
            2024-01-10 * ""
              Assets:A -30 CNY
              Expenses:Food
            2024-01-15 balance Equity:Open -100 CNY
            2024-02-01 balance Assets:A 70 CNY
        "#});
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings(&directives), vec![padding("2024-01-01", "Assets:A", "100 CNY", "Equity:Open")]);
        // in the stream, the padding comes on the day of its pad
        let position = |predicate: &dyn Fn(&Directive) -> bool| directives.iter().position(predicate).unwrap();
        let padding_at = position(&|it| matches!(it, Directive::Transaction(txn) if txn.flag == Some(Flag::BalancePad)));
        let spending_at = position(&|it| matches!(it, Directive::Transaction(txn) if txn.flag != Some(Flag::BalancePad)));
        assert!(padding_at < spending_at);
    }

    #[test]
    fn should_pad_the_next_assertion_of_each_currency_once() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Wallet
            1970-01-01 open Assets:Other
            1970-01-01 open Equity:Open
            2024-01-01 pad Assets:Wallet Equity:Open
            2024-01-02 balance Assets:Wallet 300 CNY
            2024-01-02 balance Assets:Wallet 150 USD
            2024-01-02 balance Assets:Other 1 CNY
            2024-01-03 balance Assets:Wallet 400 CNY
        "#});
        assert_eq!(
            paddings(&directives),
            vec![
                padding("2024-01-01", "Assets:Wallet", "300 CNY", "Equity:Open"),
                padding("2024-01-01", "Assets:Wallet", "150 USD", "Equity:Open"),
            ]
        );
        // the pad of `Assets:Wallet` serves neither another account nor a second CNY assertion
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError, ErrorKind::AccountBalanceCheckError]);
    }

    #[test]
    fn should_not_pad_an_assertion_on_the_day_of_the_pad() {
        // a day's balances come before its pads, as in beancount
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2017-12-01 pad Assets:A Equity:Open
            2017-12-01 balance Assets:A 0.10 CNY
            2017-12-02 balance Assets:A 0.10 CNY
        "#});
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(paddings(&directives), vec![padding("2017-12-01", "Assets:A", "0.10 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_pad_each_account_from_its_own_pad() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Bank
            1970-01-01 open Assets:Cash
            1970-01-01 open Equity:Open
            1970-01-01 open Expenses:Food
            2024-01-01 pad Assets:Bank Equity:Open
            2024-01-02 pad Assets:Cash Equity:Open
            2024-01-03 * ""
              Assets:Bank -40 CNY
              Expenses:Food
            2024-01-05 balance Assets:Cash 20 CNY
            2024-01-06 balance Assets:Bank 1000 CNY
            2024-01-08 pad Assets:Bank Equity:Open
            2024-01-10 balance Assets:Bank 1500 CNY
        "#});
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            paddings(&directives),
            vec![
                padding("2024-01-01", "Assets:Bank", "1040 CNY", "Equity:Open"),
                padding("2024-01-02", "Assets:Cash", "20 CNY", "Equity:Open"),
                padding("2024-01-08", "Assets:Bank", "500 CNY", "Equity:Open"),
            ]
        );
    }

    #[test]
    fn should_pad_exactly_even_within_an_explicit_tolerance() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 99.98 CNY
              Equity:Open
            2023-01-02 pad Assets:A Equity:Open
            2023-01-03 balance Assets:A 100.00 ~ 0.05 CNY
        "#});
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings(&directives), vec![padding("2023-01-02", "Assets:A", "0.02 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_report_a_pad_that_pads_nothing() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Assets:B
            1970-01-01 open Assets:C
            1970-01-01 open Equity:Open
            2023-01-01 * ""
              Assets:A 10 CNY
              Equity:Open
            2023-01-02 pad Assets:A Equity:Open
            2023-01-02 pad Assets:B Equity:Open
            2023-01-02 pad Assets:C Equity:Open
            2023-01-03 pad Assets:C Equity:Open
            2023-01-04 balance Assets:A 10 CNY
            2023-01-04 balance Assets:C 5 CNY
        "#});
        // already at its amount; no later assertion; replaced by a later pad before any assertion
        assert_eq!(errors, vec![ErrorKind::UnusedPad, ErrorKind::UnusedPad, ErrorKind::UnusedPad]);
        assert_eq!(paddings(&directives), vec![padding("2023-01-03", "Assets:C", "5 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_serve_a_balance_with_pad_from_an_earlier_pad() {
        // the `pad` pads the assertion, so the `balance ... with pad` has nothing left to pad
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-01 open Equity:Other
            2023-01-01 pad Assets:A Equity:Open
            2023-01-02 balance Assets:A 10 CNY with pad Equity:Other
        "#});
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings(&directives), vec![padding("2023-01-01", "Assets:A", "10 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_report_missing_and_closed_accounts_of_a_pad() {
        let (_, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Assets:Closed
            1970-01-02 close Assets:Closed
            2023-01-01 pad Assets:A Equity:Missing
            2023-01-02 balance Assets:A 10 CNY
            2023-01-03 pad Assets:Closed Assets:A
            2023-01-04 balance Assets:Closed 10 CNY
        "#});
        assert_eq!(
            errors,
            vec![
                ErrorKind::AccountDoesNotExist,
                ErrorKind::AccountClosed,
                // the check reports the closed account of its own
                ErrorKind::AccountClosed,
            ]
        );
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
