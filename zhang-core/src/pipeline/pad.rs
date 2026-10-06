//! `pad` and `balance ... with pad ...` as a native pipeline stage.

use std::collections::{HashMap, HashSet};

use bigdecimal::Zero;
use chrono::{NaiveDate, NaiveDateTime};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Date, Directive, Flag, Pad, Posting, SpanInfo, Spanned, Transaction, ZhangString};

use super::balance::UnitBalances;
use super::{ProcessStage, StageContext};
use crate::data_type::Dialect;
use crate::ledger::Ledger;
use crate::ZhangResult;

/// inserts the padding transactions (flag `P`) of `pad` and `balance ... with pad` directives.
///
/// A pad is sized from the true balance, as in beancount: the sum of the postings of the account and
/// all its sub-accounts. A `balance` assertion changes no balance, even when it fails, so a pad brings
/// the account to the asserted amount from where the postings left it. A pad of a parent account books
/// the difference to the parent account itself. A pad brings the account to exactly the asserted
/// amount: zhang infers no tolerance, and pads even within an explicit `~` tolerance.
///
/// - A `pad` serves the next balance assertion of its account in each currency dated after the day of
///   the `pad`, until the account's next `pad`, as [`PadPairing`] pairs them, like beancount 3.2.3's
///   `pad` plugin, except that only an assertion on the account itself uses it (beancount also lets one
///   on a sub-account use it up). It is sized from the balance at that assertion, with the padding of
///   every assertion served before it (beancount leaves out the padding of other pads). Its padding
///   transaction goes right after the `pad`, dated on it: the stream puts a `pad` after the balance
///   entries of its day (see [`Ledger::sort_directives_datetime`]), so with times the padding is dated
///   at the last of them when that is later than the `pad`. A `pad` that pads nothing — no later
///   assertion of its account needs it — is reported as [`ErrorKind::UnusedPad`].
/// - Padding a commodity the account or a sub-account holds at cost is reported as
///   [`ErrorKind::PadWithCost`] on the assertion, as in beancount: the padding is booked without a cost.
/// - A `balance ... with pad` pads its own assertion: its padding transaction is dated on it and goes
///   right after it. A `pad` before it is served first.
///
/// An account already at the asserted amount gets no padding transaction. The pad directives stay in
/// the stream (like beancount's `Pad` entry); the fold books only the padding transactions.
pub struct PadStage;

/// pairs the `pad` directives of a sorted stream with the balance assertions they serve, the same way for
/// the pad stage and for the stream a plugin sees: a `pad` serves the first assertion of its own account in
/// each currency dated after the day of the `pad` — days are compared, not times, as beancount knows no
/// times — and is replaced by the account's latest `pad` dated before an assertion
pub(crate) struct PadPairing<T> {
    /// the pads of each account that may still serve an assertion, in stream order
    waiting: HashMap<String, Vec<Waiting<T>>>,
    /// the pads a later one replaced
    replaced: Vec<T>,
}

struct Waiting<T> {
    date: NaiveDate,
    /// the currencies whose assertion it served already
    served: HashSet<String>,
    pad: T,
}

impl<T> Default for PadPairing<T> {
    fn default() -> Self {
        Self {
            waiting: HashMap::new(),
            replaced: vec![],
        }
    }
}

impl<T> PadPairing<T> {
    /// a `pad` of `account` dated `date`, the next in the stream
    pub(crate) fn pad(&mut self, account: &Account, date: NaiveDate, pad: T) {
        self.waiting.entry(account.name().to_owned()).or_default().push(Waiting {
            date,
            served: HashSet::new(),
            pad,
        });
    }

    /// the `pad` serving the assertion of `account` in `commodity` dated `date`, the next in the stream: the
    /// account's latest `pad` dated before that day, unless it served an assertion in `commodity` already
    pub(crate) fn serve(&mut self, account: &Account, commodity: &str, date: NaiveDate) -> Option<&mut T> {
        let waiting = self.waiting.get_mut(account.name())?;
        // the stream is sorted: the pads dated before the day come first
        let current = waiting.iter().rposition(|it| it.date < date)?;
        // no later assertion, dated on that day or later, can use an earlier pad
        self.replaced.extend(waiting.drain(..current).map(|it| it.pad));
        let current = &mut waiting[0];
        current.served.insert(commodity.to_owned()).then_some(&mut current.pad)
    }

    /// every `pad` paired, replaced or not
    pub(crate) fn into_pads(self) -> impl Iterator<Item = T> {
        self.replaced.into_iter().chain(self.waiting.into_values().flatten().map(|it| it.pad))
    }
}

/// where a directive is: its file and position, the same for a directive a plugin passed through
pub(crate) type Place = (Option<std::path::PathBuf>, usize, usize);

pub(crate) fn place(span: &SpanInfo) -> Place {
    (span.filename.clone(), span.start, span.end)
}

/// The balance assertions a `pad` may serve once a plugin of ABI v1 returned the stream: those a `pad` it could not see
/// stands for (see [`super::AbiV1View`]). Any other balance assertion the plugin returned is not padded by a `pad`, as
/// the plugin saw none. An assertion is known by its place and what it says; of several alike, the first ones
#[derive(Clone, Default, Debug)]
pub(crate) struct PadServes(HashMap<Place, Vec<(Directive, usize)>>);

impl PadServes {
    /// one more `directive` a `pad` may serve
    pub(crate) fn allow(&mut self, directive: &Spanned<Directive>) {
        let alike = self.0.entry(place(&directive.span)).or_default();
        match alike.iter_mut().find(|(it, _)| it == &directive.data) {
            Some((_, count)) => *count += 1,
            None => alike.push((directive.data.clone(), 1)),
        }
    }

    /// whether a `pad` may serve `directive`, the next one of the stream: it uses up one of those allowed
    pub(crate) fn take(&mut self, directive: &Spanned<Directive>) -> bool {
        let Some(alike) = self.0.get_mut(&place(&directive.span)) else {
            return false;
        };
        match alike.iter_mut().find(|(it, count)| *count > 0 && it == &directive.data) {
            Some((_, count)) => {
                *count -= 1;
                true
            }
            None => false,
        }
    }
}

/// whether a `pad` may serve `directive`, the next one of the stream: any, before a plugin decided
pub(crate) fn may_serve(serves: &mut Option<PadServes>, directive: &Spanned<Directive>) -> bool {
    serves.as_mut().is_none_or(|it| it.take(directive))
}

/// For each of `new`, written after the directives of a loaded ledger (its stream, [`Ledger::directives`]) but for
/// those `gone` (to be replaced): the `pad` that would serve it once the ledger is loaded again, paired as the pad stage
/// pairs them. `None` for a directive that is no balance assertion, or that no `pad` serves. A `pad` among `new` serves
/// too
pub fn serving_pads(directives: &[Spanned<Directive>], dialect: Dialect, gone: impl Fn(&Spanned<Directive>) -> bool, new: &[Directive]) -> Vec<Option<Pad>> {
    // what the pairing reads: the pads, and the balance entries, which also order the pads of a day
    let mut stream = directives
        .iter()
        .filter(|it| !gone(it))
        .filter(|it| matches!(it.data, Directive::Pad(_)) || Ledger::is_balance_entry(&it.data))
        .cloned()
        .collect::<Vec<_>>();
    let existing = stream.len();
    for (index, directive) in new.iter().enumerate() {
        // after every directive of the ledger, each at its own place
        let place = usize::MAX - index;
        stream.push(Spanned::new(
            directive.clone(),
            SpanInfo {
                start: place,
                end: place,
                content: String::new(),
                filename: None,
                ..SpanInfo::default()
            },
        ));
    }
    let keys = Ledger::sort_keys(&stream, dialect);
    let mut order = (0..stream.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| (keys[*index], *index));
    let mut pairing: PadPairing<usize> = PadPairing::default();
    let mut served = vec![None; new.len()];
    for index in order {
        let (account, commodity, date) = match &stream[index].data {
            Directive::Pad(pad) => {
                pairing.pad(&pad.account, pad.date.naive_date(), index);
                continue;
            }
            Directive::BalanceCheck(check) => (&check.account, &check.amount.commodity, check.date.naive_date()),
            Directive::BalancePad(pad) => (&pad.account, &pad.amount.commodity, pad.date.naive_date()),
            _ => continue,
        };
        let Some(pad) = pairing.serve(account, commodity, date).copied() else {
            continue;
        };
        if let (Some(slot), Directive::Pad(pad)) = (index.checked_sub(existing).and_then(|it| served.get_mut(it)), &stream[pad].data) {
            *slot = Some(pad.clone());
        }
    }
    served
}

/// a `pad` waiting for the assertions it serves
struct ActivePad {
    pad: Pad,
    span: SpanInfo,
    /// its place in the stream: its padding goes right after it, and unused pads are reported in this order
    position: usize,
    /// what its padding is dated: the `pad`, or the last balance entry of its day when that is later
    date: Date,
    /// whether it padded anything
    used: bool,
}

impl ProcessStage for PadStage {
    fn name(&self) -> &str {
        "balance-pad"
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut balances = UnitBalances::for_stage(ctx);
        let mut pairing: PadPairing<ActivePad> = PadPairing::default();
        let mut ret = Vec::with_capacity(directives.len());
        // the padding transactions of each `pad`, by the place of the `pad` in `ret`
        let mut paddings: HashMap<usize, Vec<Spanned<Directive>>> = HashMap::new();
        // the time of the last balance entry so far: the stream puts a `pad` after every balance entry of its day
        let mut last_balance_entry: Option<NaiveDateTime> = None;
        // the assertions a `pad` may serve, when a plugin decided them
        let mut serves = ctx.pad_serves.clone();

        for directive in directives {
            if Ledger::is_balance_entry(&directive.data) {
                last_balance_entry = directive.datetime();
            }
            let padding = match &directive.data {
                Directive::Open(open) => {
                    balances.apply_open(open);
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
                    let date = match (last_balance_entry, directive.datetime()) {
                        (Some(last), Some(at)) if last.date() == at.date() && last > at => Date::Datetime(last),
                        _ => pad.date.clone(),
                    };
                    let waiting = ActivePad {
                        pad: pad.clone(),
                        span: directive.span.clone(),
                        position: ret.len(),
                        date,
                        used: false,
                    };
                    pairing.pad(&pad.account, pad.date.naive_date(), waiting);
                    None
                }
                Directive::BalanceCheck(_) if !may_serve(&mut serves, &directive) => None,
                Directive::BalanceCheck(check) => {
                    serve(
                        ctx,
                        &mut pairing,
                        &mut paddings,
                        &mut balances,
                        &check.date,
                        &check.account,
                        &check.amount,
                        &directive.span,
                    );
                    None
                }
                Directive::BalancePad(pad) => {
                    if may_serve(&mut serves, &directive) {
                        serve(
                            ctx,
                            &mut pairing,
                            &mut paddings,
                            &mut balances,
                            &pad.date,
                            &pad.account,
                            &pad.amount,
                            &directive.span,
                        );
                    }
                    // nothing to pad: no transaction (beancount does the same)
                    let distance = balances.distance(&pad.account, &pad.amount);
                    (!distance.number.is_zero()).then(|| {
                        report_cost(ctx, &balances, &pad.account, &pad.amount.commodity, &directive.span);
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
        let mut pads = pairing.into_pads().collect::<Vec<_>>();
        pads.sort_by_key(|it| it.position);
        for pad in pads {
            report_unused(ctx, pad);
        }
        if paddings.is_empty() {
            return Ok(ret);
        }
        // each padding transaction right after its `pad`, in the order of the assertions it serves
        let mut placed = Vec::with_capacity(ret.len() + paddings.values().map(Vec::len).sum::<usize>());
        for (position, directive) in ret.into_iter().enumerate() {
            placed.push(directive);
            placed.extend(paddings.remove(&position).unwrap_or_default());
        }
        Ok(placed)
    }
}

/// serve the assertion at `span` of `account` dated `date` with the `pad` paired with it, if any: the first
/// assertion of each currency after the `pad` gets the padding it needs, kept in `paddings` for the `pad`
#[allow(clippy::too_many_arguments)]
fn serve(
    ctx: &mut StageContext, pairing: &mut PadPairing<ActivePad>, paddings: &mut HashMap<usize, Vec<Spanned<Directive>>>, balances: &mut UnitBalances,
    date: &Date, account: &Account, asserted: &Amount, span: &SpanInfo,
) {
    let Some(waiting) = pairing.serve(account, &asserted.commodity, date.naive_date()) else {
        return;
    };
    let distance = balances.distance(account, asserted);
    if distance.number.is_zero() {
        return;
    }
    report_cost(ctx, balances, account, &asserted.commodity, span);
    waiting.used = true;
    let txn = padding_transaction(waiting.date.clone(), &waiting.pad.account, &waiting.pad.pad, distance);
    balances.apply_transaction(&txn);
    paddings
        .entry(waiting.position)
        .or_default()
        .push(Spanned::new(Directive::Transaction(txn), waiting.span.clone()));
}

/// padding a commodity the account or a sub-account holds at cost is an error on the assertion it serves, as in
/// beancount: the padding is booked without a cost. Beancount reports it once for each lot held at cost, zhang once
/// for the assertion
fn report_cost(ctx: &mut StageContext, balances: &UnitBalances, account: &Account, commodity: &str, span: &SpanInfo) {
    if balances.holds_at_cost(account, commodity) {
        ctx.emit_error(
            ErrorKind::PadWithCost,
            span.clone(),
            HashMap::from([
                ("account_name".to_owned(), account.name().to_owned()),
                ("commodity".to_owned(), commodity.to_owned()),
            ]),
        );
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
                written: None,
            },
            Posting {
                flag: None,
                account: pad.clone(),
                units: None,
                cost: None,
                price: None,
                comment: None,
                meta: Default::default(),
                written: None,
            },
        ],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod test {
    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Date, Directive, Flag, Transaction};

    use crate::data_type::DataType;
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
    fn should_pair_a_pad_by_days_whatever_the_times() {
        // a `balance` on the day of the `pad` is not padded, also later that day: the next day's is
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2024-03-01 10:00:00 balance Assets:A 0 CNY
            2024-03-01 pad Assets:A Equity:Open
            2024-03-02 09:00:00 balance Assets:A 150 CNY
        "#});
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings(&directives), vec![padding("2024-03-01", "Assets:A", "150 CNY", "Equity:Open")]);
        // the `pad` and its padding come after the balance of its day, at its time
        let booked = synthesized(&directives, Flag::BalancePad)[0];
        assert_eq!(
            booked.date,
            Date::Datetime(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap().and_hms_opt(10, 0, 0).unwrap())
        );

        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            2024-03-01 10:00:00 pad Assets:A Equity:Open
            2024-03-01 12:00:00 balance Assets:A 150 CNY
            2024-03-02 balance Assets:A 150 CNY
        "#});
        // the balance at noon is checked before the `pad` of its day, against nothing, as beancount checks it
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(paddings(&directives), vec![padding("2024-03-01", "Assets:A", "150 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_put_the_padding_right_after_its_pad() {
        // as beancount orders a day: its balances, then the rest in file order, the padding right after its pad
        let (directives, _) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Equity:Open
            1970-01-01 open Expenses:Food
            2024-01-05 * "before pad"
              Assets:A -10 CNY
              Expenses:Food
            2024-01-05 pad Assets:A Equity:Open
            2024-01-05 * "after pad"
              Assets:A -20 CNY
              Expenses:Food
            2024-01-05 balance Assets:A -30 CNY
            2024-01-06 balance Assets:A 100 CNY
        "#});
        let order = directives
            .iter()
            .filter(|it| it.datetime().is_some_and(|date| date.date().to_string() != "1970-01-01"))
            .map(|it| match it {
                Directive::Transaction(txn) => txn
                    .payee
                    .as_ref()
                    .or(txn.narration.as_ref())
                    .map(|it| it.as_str().to_owned())
                    .unwrap_or_default(),
                other => other.directive_type().to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(order, vec!["BalanceCheck", "before pad", "Pad", "Balance Pad", "after pad", "BalanceCheck"]);
        assert_eq!(paddings(&directives), vec![padding("2024-01-05", "Assets:A", "130 CNY", "Equity:Open")]);
    }

    #[test]
    fn should_not_report_padding_a_commodity_whose_lots_are_sold() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Stock
            1970-01-01 open Assets:Cash
            1970-01-01 open Equity:Open
            1970-01-01 open Income:Gains
            2024-01-02 * "buy"
              Assets:Stock 10 AAPL {100 USD}
              Assets:Cash -1000 USD
            2024-01-03 * "sell all"
              Assets:Stock -10 AAPL {100 USD} @ 120 USD
              Assets:Cash 1200 USD
              Income:Gains -200 USD
            2024-01-04 pad Assets:Stock Equity:Open
            2024-01-05 balance Assets:Stock 3 AAPL
        "#});
        // no lot is held at cost any more
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(paddings(&directives), vec![padding("2024-01-04", "Assets:Stock", "3 AAPL", "Equity:Open")]);
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
    fn should_report_unused_pads_in_stream_order() {
        let content = indoc! {r#"
            1970-01-01 open Assets:A
            1970-01-01 open Assets:B
            1970-01-01 open Equity:Open
            2023-01-02 pad Assets:B Equity:Open
            2023-01-02 pad Assets:A Equity:Open
            2023-01-03 pad Assets:A Equity:Open
            2023-01-04 balance Assets:A 10 CNY
        "#};
        let directives = crate::data_type::text::ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        let mut ctx = crate::pipeline::StageContext::new(&[]);
        crate::pipeline::run_pipeline(
            &crate::pipeline::builtin_stages(),
            crate::ledger::Ledger::sort_directives_datetime(directives, crate::data_type::Dialect::Zhang),
            &mut ctx,
        )
        .unwrap();
        // the pad of B, waiting to the end, and the first pad of A, which the second replaced, as they are in the stream
        let unused = ctx
            .into_errors()
            .into_iter()
            .filter(|it| it.kind == ErrorKind::UnusedPad)
            .map(|it| it.span.content.trim().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(unused, vec!["2023-01-02 pad Assets:B Equity:Open", "2023-01-02 pad Assets:A Equity:Open"]);
    }

    #[test]
    fn should_report_padding_a_commodity_held_at_cost() {
        let (directives, errors) = run_builtin_stages(indoc! {r#"
            1970-01-01 open Assets:Broker
            1970-01-01 open Assets:Broker:Stock
            1970-01-01 open Assets:Broker:Cash
            1970-01-01 open Assets:Other
            1970-01-01 open Equity:Open
            2024-01-02 * "buy"
              Assets:Broker:Stock 10 AAPL {100 USD}
              Assets:Broker:Cash -1000 USD
            2024-01-03 pad Assets:Broker:Stock Equity:Open
            2024-01-04 balance Assets:Broker:Stock 15 AAPL
            2024-01-05 balance Assets:Broker 17 AAPL with pad Equity:Open
            2024-01-06 pad Assets:Other Equity:Open
            2024-01-07 balance Assets:Other 3 AAPL
            2024-01-08 balance Assets:Broker:Cash -900 USD with pad Equity:Open
        "#});
        // the stock and its parent hold AAPL at cost; `Assets:Other` and the USD of the cash do not
        assert_eq!(errors, vec![ErrorKind::PadWithCost, ErrorKind::PadWithCost]);
        // the padding is still booked, without a cost
        assert_eq!(
            paddings(&directives),
            vec![
                padding("2024-01-03", "Assets:Broker:Stock", "5 AAPL", "Equity:Open"),
                padding("2024-01-05", "Assets:Broker", "2 AAPL", "Equity:Open"),
                padding("2024-01-06", "Assets:Other", "3 AAPL", "Equity:Open"),
                padding("2024-01-08", "Assets:Broker:Cash", "100 USD", "Equity:Open"),
            ]
        );
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
                // the check after the close only records, as in beancount: it reports nothing of its own
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
