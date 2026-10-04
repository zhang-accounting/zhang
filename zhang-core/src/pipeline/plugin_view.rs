//! The directive stream as a WASM plugin of ABI v1 sees it.
//!
//! ABI v1 lets zhang add fields, never directive kinds: a plugin built against an older
//! zhang-ast cannot read a kind it does not know. The `pad` directive came after v1, so a
//! plugin never sees one, and sees the stream a beancount ledger loaded as before `pad` existed:
//! every `pad` is set aside before the plugin runs, and a `balance` a `pad` serves is shown as a
//! `balance ... with pad` from the pad account of its `pad`.
//!
//! What the plugin returns is its word. The view puts a `pad` back only where that pads what
//! the plugin returned, and leaves it out otherwise:
//!
//! - a `pad` whose served balances all come back as `balance ... with pad`, of one account from
//!   one pad account, is put back with that account and pad account (a plugin may rename either),
//!   right after the directive it followed (by span), or where its date puts it when that
//!   directive is gone. The balances turn back into `balance`s with their tolerance, and keep
//!   everything else the plugin changed in them;
//! - a `pad` one of whose served balances the plugin dropped, or turned into a plain `balance`,
//!   or gave another account or pad account than the others, is left out, and so is a `pad` that,
//!   put back, would not serve exactly the balances it served, such as when the plugin moved one
//!   to another day. The balances the plugin returned as `balance ... with pad` stay so, and each
//!   pads its own assertion;
//! - a `pad` that serves no balance is invisible to a plugin: it is put back as it was, and must
//!   still serve none: where it would serve one, it is left out.
//!
//! A `pad` left out pads nothing, and is reported as [`ErrorKind::UnusedPad`], as the pad stage
//! reports a `pad` put back that pads nothing.
//!
//! A `pad` put back serves only the balances it stood for ([`PadServes`], which the pad stage and
//! the views of later plugins follow): any other balance the plugin returned, such as one it added
//! or turned into a plain `balance`, is not padded by a `pad` the plugin could not see. So leaving a
//! `pad` out changes no other `pad`, and the pads of an account are checked once, from the last.
//!
//! A plugin that changes nothing gets exactly the stream it was given back, with the `pad`s.
//! Exposing `pad` to plugins is future ABI work.

use std::collections::{HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, BalanceCheck, BalancePad, Directive, Pad, SpanInfo, Spanned};

use super::pad::{may_serve, place, PadPairing, PadServes, Place};
use super::{ProcessStage, StageContext};
use crate::ledger::Ledger;
use crate::ZhangResult;

/// a WASM plugin's stage, run on the stream as plugins of ABI v1 see it
pub struct AbiV1View {
    plugin: Box<dyn ProcessStage>,
}

impl AbiV1View {
    pub fn new(plugin: Box<dyn ProcessStage>) -> Self {
        Self { plugin }
    }
}

impl ProcessStage for AbiV1View {
    fn name(&self) -> &str {
        self.plugin.name()
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let (shown, hidden) = hide_pads(directives, ctx.pad_serves.clone());
        let returned = self.plugin.process(shown, ctx)?;
        Ok(hidden.restore(returned, ctx))
    }
}

/// a `pad` [`hide_pads`] took out of the stream
struct HiddenPad {
    /// the place of the directive before it that is no `pad`, if any
    before: Option<Place>,
    pad: Pad,
    span: SpanInfo,
}

/// what [`hide_pads`] took out of the stream
struct HiddenPads {
    /// each `pad`, in stream order
    pads: Vec<HiddenPad>,
    /// each `balance` shown as a `balance ... with pad`, by its place: the `pad` serving it, and its tolerance
    shown: HashMap<Place, (usize, Option<BigDecimal>)>,
    /// each `balance ... with pad` a `pad` serves first, by its place: that `pad`. A plugin sees it as it is
    padded: HashMap<Place, usize>,
}

/// the stream without its `pad` directives, the `balance` each serves shown as a `balance ... with pad`,
/// paired as the pad stage pairs them, a `pad` serving only the assertions `serves` allows when an earlier plugin decided
fn hide_pads(directives: Vec<Spanned<Directive>>, mut serves: Option<PadServes>) -> (Vec<Spanned<Directive>>, HiddenPads) {
    let mut pairing = PadPairing::default();
    let mut hidden = HiddenPads {
        pads: vec![],
        shown: HashMap::new(),
        padded: HashMap::new(),
    };
    let mut before = None;
    let mut shown = Vec::with_capacity(directives.len());
    for directive in directives {
        let served = matches!(directive.data, Directive::BalanceCheck(_) | Directive::BalancePad(_)) && may_serve(&mut serves, &directive);
        let Spanned { data, span } = directive;
        let data = match data {
            Directive::Pad(pad) => {
                pairing.pad(&pad.account, pad.date.naive_date(), hidden.pads.len());
                hidden.pads.push(HiddenPad {
                    before: before.clone(),
                    pad,
                    span,
                });
                continue;
            }
            Directive::BalanceCheck(check) if !served => Directive::BalanceCheck(check),
            Directive::BalanceCheck(check) => match pairing.serve(&check.account, &check.amount.commodity, check.date.naive_date()).copied() {
                Some(index) => {
                    hidden.shown.insert(place(&span), (index, check.tolerance));
                    Directive::BalancePad(BalancePad {
                        date: check.date,
                        account: check.account,
                        amount: check.amount,
                        pad: hidden.pads[index].pad.pad.clone(),
                        meta: check.meta,
                    })
                }
                None => Directive::BalanceCheck(check),
            },
            Directive::BalancePad(pad) => {
                // an assertion the `pad` serves first, before the `balance ... with pad` pads the rest
                if let Some(index) = served
                    .then(|| pairing.serve(&pad.account, &pad.amount.commodity, pad.date.naive_date()).copied())
                    .flatten()
                {
                    hidden.padded.insert(place(&span), index);
                }
                Directive::BalancePad(pad)
            }
            other => other,
        };
        before = Some(place(&span));
        shown.push(Spanned::new(data, span));
    }
    (shown, hidden)
}

impl HiddenPads {
    /// the stream a plugin returned, with the `pad` directives that pad what it returned put back, and the
    /// `balance` directives they serve turned back
    fn restore(self, returned: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> Vec<Spanned<Directive>> {
        let HiddenPads { pads, shown, padded } = self;
        if pads.is_empty() {
            return returned;
        }
        let count = pads.len();
        // how many balances each pad serves: shown as `balance ... with pad`, and as they are
        let mut serves = vec![0; count];
        for (index, _) in shown.values() {
            serves[*index] += 1;
        }
        let mut serves_first = vec![0; count];
        for index in padded.values() {
            serves_first[*index] += 1;
        }

        // the stream again, each pad back after the directive it followed
        let mut after: HashMap<Option<Place>, Vec<usize>> = HashMap::new();
        for (index, pad) in pads.iter().enumerate() {
            after.entry(pad.before.clone()).or_default().push(index);
        }
        let mut pads = pads.into_iter().map(Some).collect::<Vec<_>>();
        let mut out = Vec::with_capacity(returned.len() + count);
        // each pad by its place in `out`, and its place
        let mut pad_of: HashMap<usize, usize> = HashMap::with_capacity(count);
        let mut position_of = vec![0; count];
        let mut put_back = |indices: Vec<usize>, pads: &mut [Option<HiddenPad>], out: &mut Vec<Spanned<Directive>>| {
            for index in indices {
                let HiddenPad { pad, span, .. } = pads[index].take().expect("a pad is put back once");
                pad_of.insert(out.len(), index);
                position_of[index] = out.len();
                out.push(Spanned::new(Directive::Pad(pad), span));
            }
        };
        put_back(after.remove(&None).unwrap_or_default(), &mut pads, &mut out);
        // the places in `out` of the balances each pad serves that the plugin returned as `balance ... with pad`:
        // those shown so, and those that were
        let mut claims: Vec<Vec<usize>> = vec![vec![]; count];
        let mut natives: Vec<Vec<usize>> = vec![vec![]; count];
        let mut claimed: HashSet<Place> = HashSet::new();
        for directive in returned {
            let here = place(&directive.span);
            if matches!(directive.data, Directive::BalancePad(_)) && !claimed.contains(&here) {
                // the first at a place is the one the plugin was shown; the others are its own
                if let Some((index, _)) = shown.get(&here) {
                    claims[*index].push(out.len());
                    claimed.insert(here.clone());
                } else if let Some(index) = padded.get(&here) {
                    natives[*index].push(out.len());
                    claimed.insert(here.clone());
                }
            }
            out.push(directive);
            if let Some(indices) = after.remove(&Some(here)) {
                put_back(indices, &mut pads, &mut out);
            }
        }
        // the directive a pad followed is gone: the re-sort puts the pad back on its date
        let mut rest = after.into_values().flatten().collect::<Vec<_>>();
        rest.sort_by_key(|index| pads[*index].as_ref().map(|pad| (pad.span.filename.clone(), pad.span.start)));
        put_back(rest, &mut pads, &mut out);

        // what the plugin's `balance ... with pad`s say each pad pads, and from where
        let mut keep = vec![true; count];
        for index in 0..count {
            if serves[index] == 0 && serves_first[index] == 0 {
                continue;
            }
            if claims[index].len() != serves[index] || natives[index].len() != serves_first[index] {
                keep[index] = false;
                continue;
            }
            let mut said = claims[index].iter().map(|position| match &out[*position].data {
                Directive::BalancePad(it) => (it.account.clone(), it.pad.clone()),
                _ => unreachable!("a claim is a `balance ... with pad`"),
            });
            let Some((account, source)) = said.next() else { continue };
            if !said.all(|it| it == (account.clone(), source.clone())) {
                keep[index] = false;
                continue;
            }
            if let Directive::Pad(pad) = &mut out[position_of[index]].data {
                pad.account = account;
                pad.pad = source;
            }
        }
        // the balances of the pads put back turn back into `balance`s; the pad account the plugin gave each is kept,
        // should its pad be left out after all
        let mut sources: HashMap<usize, Account> = HashMap::new();
        for index in (0..count).filter(|index| keep[*index]) {
            for position in &claims[index] {
                let tolerance = shown[&place(&out[*position].span)].1.clone();
                let Directive::BalancePad(pad) = out[*position].data.clone() else {
                    unreachable!("a claim is a `balance ... with pad`")
                };
                sources.insert(*position, pad.pad);
                out[*position].data = Directive::BalanceCheck(BalanceCheck {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,
                    tolerance,
                    meta: pad.meta,
                });
            }
        }
        // the balances each pad put back stands for, the only ones a pad may serve: a plugin sees no pad, so any other
        // balance it returned is not padded by one
        let stands_for = (0..count)
            .map(|index| {
                let mut positions = if keep[index] {
                    claims[index].iter().chain(&natives[index]).copied().collect::<Vec<_>>()
                } else {
                    vec![]
                };
                positions.sort_unstable();
                positions
            })
            .collect::<Vec<_>>();
        // a pad put back that would serve other balances than those, is left out: the balances it stood for pad
        // themselves, as the plugin returned them
        for index in left_out(&out, &pad_of, &keep, &stands_for) {
            keep[index] = false;
            for position in &claims[index] {
                let Directive::BalanceCheck(check) = out[*position].data.clone() else {
                    unreachable!("a claim put back is a `balance`")
                };
                out[*position].data = Directive::BalancePad(BalancePad {
                    date: check.date,
                    account: check.account,
                    amount: check.amount,
                    pad: sources.remove(position).expect("the pad account of a claim is kept"),
                    meta: check.meta,
                });
            }
        }
        let mut serves = PadServes::default();
        for index in (0..count).filter(|index| keep[*index]) {
            for position in &stands_for[index] {
                serves.allow(&out[*position]);
            }
        }
        ctx.pad_serves = Some(serves);
        // a pad left out pads nothing: it is reported unused, as the pad stage reports a pad put back that pads nothing
        for index in (0..count).filter(|index| !keep[*index]) {
            if let Directive::Pad(pad) = &out[position_of[index]].data {
                ctx.emit_error(
                    ErrorKind::UnusedPad,
                    out[position_of[index]].span.clone(),
                    HashMap::from([("account_name".to_owned(), pad.account.name().to_owned())]),
                );
            }
        }
        out.into_iter()
            .enumerate()
            .filter(|(position, _)| pad_of.get(position).is_none_or(|index| keep[*index]))
            .map(|(_, directive)| directive)
            .collect()
    }
}

/// the day of a pad or a balance assertion
fn day(directive: &Directive) -> Option<NaiveDate> {
    match directive {
        Directive::Pad(pad) => Some(pad.date.naive_date()),
        Directive::BalanceCheck(check) => Some(check.date.naive_date()),
        Directive::BalancePad(pad) => Some(pad.date.naive_date()),
        _ => None,
    }
}

/// The pads put back (`keep`) that would not serve exactly the balances they stand for once `out` is sorted, as the
/// pad stage pairs them, a pad serving only the balances some pad stands for: a pad serves the first of each commodity
/// of those dated after it, and before any later pad. A pad that stands for none must serve none. A pad the plugin
/// wrote itself serves what it serves.
///
/// The pads of an account are checked from the last to the first: a pad left out leaves the balances it stood for to
/// no pad, and the earlier pads of its account those it would have served. Each balance is looked at about once.
fn left_out(out: &[Spanned<Directive>], pad_of: &HashMap<usize, usize>, keep: &[bool], stands_for: &[Vec<usize>]) -> Vec<usize> {
    let keys = Ledger::sort_keys(out);
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for (index, positions) in stands_for.iter().enumerate() {
        for position in positions {
            owner.insert(*position, index);
        }
    }
    // the pads and the balances a pad stands for of each account, by their place in `out`
    let mut accounts: HashMap<&str, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (position, directive) in out.iter().enumerate() {
        match &directive.data {
            Directive::Pad(pad) if pad_of.get(&position).is_none_or(|index| keep[*index]) => accounts.entry(pad.account.name()).or_default().0.push(position),
            Directive::BalanceCheck(check) if owner.contains_key(&position) => accounts.entry(check.account.name()).or_default().1.push(position),
            Directive::BalancePad(pad) if owner.contains_key(&position) => accounts.entry(pad.account.name()).or_default().1.push(position),
            _ => {}
        }
    }
    let commodity = |position: usize| match &out[position].data {
        Directive::BalanceCheck(check) => check.amount.commodity.as_str(),
        Directive::BalancePad(pad) => pad.amount.commodity.as_str(),
        _ => unreachable!("a balance assertion"),
    };
    let mut left_out = vec![];
    for (mut pads, mut balances) in accounts.into_values() {
        // in the order of the stream once sorted: the key, then the place in `out`
        pads.sort_by_key(|position| (keys[*position], *position));
        balances.sort_by_key(|position| (keys[*position], *position));
        let index_of = balances
            .iter()
            .enumerate()
            .map(|(index, position)| (*position, index))
            .collect::<HashMap<_, _>>();
        // the balances no later pad serves are `balances[..end]`, but for those of pads left out
        let mut gone = vec![false; balances.len()];
        let mut end = balances.len();
        for position in pads.into_iter().rev() {
            let date = day(&out[position].data);
            // the balances dated after the pad, which it serves the first of each commodity of
            let mut start = end;
            while start > 0 && day(&out[balances[start - 1]].data) > date {
                start -= 1;
            }
            let Some(index) = pad_of.get(&position).copied() else {
                end = start;
                continue;
            };
            let mut commodities = HashSet::new();
            let mut served = (start..end)
                .filter(|it| !gone[*it])
                .map(|it| balances[it])
                .filter(|it| commodities.insert(commodity(*it)))
                .collect::<Vec<_>>();
            served.sort_unstable();
            if served == stands_for[index] {
                end = start;
            } else {
                left_out.push(index);
                for position in &stands_for[index] {
                    gone[index_of[position]] = true;
                }
            }
        }
    }
    left_out
}

#[cfg(test)]
mod test {
    use std::str::FromStr;
    use std::sync::Mutex;

    use indoc::indoc;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Account, BalanceCheck, BalancePad, Directive, SpanInfo, Spanned};

    use super::{hide_pads, AbiV1View};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;
    use crate::pipeline::test::balance_stages;
    use crate::pipeline::{run_pipeline, ProcessStage, StageContext};
    use crate::ZhangResult;

    /// a plugin of ABI v1: it records the stream it is given, and returns it changed by `change`
    struct OldPlugin {
        seen: Mutex<Vec<Directive>>,
        change: fn(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>,
    }

    impl ProcessStage for OldPlugin {
        fn name(&self) -> &str {
            "old"
        }

        fn process(&self, directives: Vec<Spanned<Directive>>, _ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            *self.seen.lock().unwrap() = directives.iter().map(|it| it.data.clone()).collect();
            Ok((self.change)(directives))
        }
    }

    const LEDGER: &str = indoc! {r#"
        1970-01-01 open Assets:Bank
        1970-01-01 open Assets:Cash
        1970-01-01 open Equity:Open
        1970-01-01 open Expenses:Food
        2024-01-01 pad Assets:Bank Equity:Open
        2024-01-01 pad Assets:Cash Equity:Open
        2024-01-02 * "lunch"
          Assets:Bank -10 CNY
          Expenses:Food
        2024-01-05 balance Assets:Bank 100.00 ~ 0.01 CNY
        2024-01-06 balance Assets:Bank 100.00 CNY
    "#};

    fn parse(content: &str) -> Vec<Spanned<Directive>> {
        Ledger::sort_directives_datetime(ZhangDataType {}.transform(content.to_owned(), None).unwrap())
    }

    /// run the plugin through the view, then the built-in stages, as a load does
    fn run(plugin: &'static OldPlugin, content: &str) -> (Vec<Directive>, Vec<ErrorKind>) {
        let (out, errors) = run_reporting(plugin, content);
        (out, errors.into_iter().map(|(kind, _)| kind).collect())
    }

    /// [`run`], with the line of the directive each error is on
    fn run_reporting(plugin: &'static OldPlugin, content: &str) -> (Vec<Directive>, Vec<(ErrorKind, String)>) {
        struct Shared(&'static OldPlugin);
        impl ProcessStage for Shared {
            fn name(&self) -> &str {
                self.0.name()
            }
            fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
                self.0.process(directives, ctx)
            }
        }
        let stages: Vec<Box<dyn ProcessStage>> = std::iter::once(Box::new(AbiV1View::new(Box::new(Shared(plugin)))) as Box<dyn ProcessStage>)
            .chain(balance_stages())
            .collect();
        let mut ctx = StageContext::new(&[]);
        let out = run_pipeline(&stages, parse(content), &mut ctx).unwrap();
        let errors = ctx.into_errors().into_iter().map(|it| (it.kind, it.span.content.trim().to_owned())).collect();
        (out.into_iter().map(|it| it.data).collect(), errors)
    }

    fn plugin(change: fn(Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>>) -> &'static OldPlugin {
        Box::leak(Box::new(OldPlugin {
            seen: Mutex::new(vec![]),
            change,
        }))
    }

    /// the kinds of the directives, with the pad account of a `balance ... with pad`
    fn kinds(directives: &[Directive]) -> Vec<String> {
        directives
            .iter()
            .filter(|it| it.datetime().is_some_and(|date| date.date().to_string() != "1970-01-01"))
            .map(|it| match it {
                Directive::BalancePad(pad) => format!("BalancePad {} {} from {}", pad.account.name(), pad.amount.commodity, pad.pad.name()),
                Directive::BalanceCheck(check) => format!("BalanceCheck {} {}", check.account.name(), check.amount.commodity),
                Directive::Pad(pad) => format!("Pad {}", pad.account.name()),
                Directive::Transaction(txn) => format!("Transaction {}", txn.flag.clone().unwrap()),
                other => other.directive_type().to_string(),
            })
            .collect()
    }

    #[test]
    fn a_plugin_sees_no_pad_and_a_served_balance_as_a_balance_with_pad() {
        let plugin = plugin(|stream| stream);
        let (out, errors) = run(plugin, LEDGER);

        // what the plugin saw: the stream a beancount ledger loaded as before `pad`
        assert_eq!(
            kinds(&plugin.seen.lock().unwrap()),
            vec!["Transaction *", "BalancePad Assets:Bank CNY from Equity:Open", "BalanceCheck Assets:Bank CNY",]
        );
        // the pads are back and work: the Cash pad pads nothing and is reported unused
        assert_eq!(errors, vec![ErrorKind::UnusedPad]);
        assert_eq!(
            kinds(&out),
            vec![
                "Pad Assets:Bank",
                "Transaction P",
                "Pad Assets:Cash",
                "Transaction *",
                "BalanceCheck Assets:Bank CNY",
                "BalanceCheck Assets:Bank CNY",
            ]
        );
        // the served balance keeps its tolerance
        let Directive::BalanceCheck(served) = out.iter().find(|it| matches!(it, Directive::BalanceCheck(_))).unwrap() else {
            unreachable!()
        };
        assert_eq!(served.tolerance.as_ref().map(|it| it.to_string()), Some("0.01".to_owned()));
        // the same as the load without the plugin
        let (plain, plain_errors) = crate::pipeline::test::run_builtin_stages(LEDGER);
        assert_eq!(out, plain);
        assert_eq!(errors, plain_errors);
    }

    #[test]
    fn a_change_the_plugin_makes_to_the_served_balance_is_kept() {
        let plugin = plugin(|stream| {
            stream
                .into_iter()
                .filter(|it| !matches!(it.data, Directive::Transaction(_)))
                .map(|mut it| {
                    if let Directive::BalancePad(pad) = &mut it.data {
                        pad.amount.number = 110.into();
                    }
                    it
                })
                .collect()
        });
        let (out, _) = run(plugin, LEDGER);
        let padded = out
            .iter()
            .find_map(|it| match it {
                Directive::Transaction(txn) if txn.postings[0].account.name() == "Assets:Bank" => txn.postings[0].units.clone(),
                _ => None,
            })
            .unwrap();
        // without the lunch it removed, the pad brings the bank to the 110 the plugin asserted
        assert_eq!(padded.number, 110.into());
        let first_check = out
            .iter()
            .find_map(|it| match it {
                Directive::BalanceCheck(check) => Some(check),
                _ => None,
            })
            .unwrap();
        assert_eq!(first_check.amount.number, 110.into());
        assert_eq!(first_check.tolerance.as_ref().map(|it| it.to_string()), Some("0.01".to_owned()));
    }

    /// (date, padded account, units, account padded from) of the padding transactions, in stream order
    fn paddings(directives: &[Directive]) -> Vec<String> {
        directives
            .iter()
            .filter_map(|it| match it {
                Directive::Transaction(txn) if txn.flag == Some(zhang_ast::Flag::BalancePad) => Some(format!(
                    "{} {} {} from {}",
                    txn.date.naive_date(),
                    txn.postings[0].account.name(),
                    txn.postings[0].units.as_ref().unwrap(),
                    txn.postings[1].account.name()
                )),
                _ => None,
            })
            .collect()
    }

    fn pads(directives: &[Directive]) -> Vec<String> {
        directives
            .iter()
            .filter_map(|it| match it {
                Directive::Pad(pad) => Some(format!("{} {} from {}", pad.date.naive_date(), pad.account.name(), pad.pad.name())),
                _ => None,
            })
            .collect()
    }

    /// a pad serving a balance in two currencies, and a later balance of the same account it does not serve
    const TWO_CURRENCIES: &str = indoc! {r#"
        1970-01-01 open Assets:Bank
        1970-01-01 open Equity:Open
        1970-01-01 open Equity:Fx
        1970-01-01 open Equity:Other
        2024-01-01 pad Assets:Bank Equity:Open
        2024-01-02 balance Assets:Bank 100 CNY
        2024-01-02 balance Assets:Bank 20 USD
        2024-01-06 balance Assets:Bank 100 CNY
    "#};

    /// the stream with every served `balance ... with pad` changed by `change`
    fn each_shown(stream: Vec<Spanned<Directive>>, change: impl Fn(&mut BalancePad)) -> Vec<Spanned<Directive>> {
        stream
            .into_iter()
            .map(|mut it| {
                if let Directive::BalancePad(pad) = &mut it.data {
                    change(pad);
                }
                it
            })
            .collect()
    }

    fn account(name: &str) -> Account {
        Account::from_str(name).unwrap()
    }

    #[test]
    fn a_plugin_that_changes_nothing_gets_its_stream_back_exactly() {
        // pads first in the stream, pads after the same directive, a `balance ... with pad` a pad serves first
        let original = parse(indoc! {r#"
            1969-12-30 pad Assets:Bank Equity:Open
            1969-12-31 pad Assets:Cash Equity:Open
            1970-01-01 open Assets:Bank
            1970-01-01 open Assets:Cash
            1970-01-01 open Equity:Open
            1970-01-01 open Equity:Other
            2024-01-02 pad Assets:Bank Equity:Open
            2024-01-02 pad Assets:Cash Equity:Open
            2024-01-03 balance Assets:Bank 100.00 ~ 0.01 CNY
            2024-01-03 balance Assets:Bank 100 CNY with pad Equity:Other
            2024-01-04 balance Assets:Cash 5 USD with pad Equity:Other
        "#});
        assert!(matches!(original[0].data, Directive::Pad(_)), "a pad comes first");
        let (shown, hidden) = hide_pads(original.clone(), None);
        assert!(!shown.iter().any(|it| matches!(it.data, Directive::Pad(_))));
        assert_eq!(hidden.restore(shown, &mut StageContext::new(&[])), original);
    }

    #[test]
    fn a_balance_with_pad_uses_up_the_pad_a_plugin_does_not_see() {
        let plugin = plugin(|stream| stream);
        let (out, errors) = run(
            plugin,
            indoc! {r#"
                1970-01-01 open Assets:A
                1970-01-01 open Equity:Open
                1970-01-01 open Equity:Other
                2023-01-01 pad Assets:A Equity:Open
                2023-01-02 balance Assets:A 10 CNY with pad Equity:Other
                2023-01-03 balance Assets:A 20 CNY
            "#},
        );
        // the pad serves the `balance ... with pad`: the later `balance` is not served, and shown as it is
        assert_eq!(
            kinds(&plugin.seen.lock().unwrap()),
            vec!["BalancePad Assets:A CNY from Equity:Other", "BalanceCheck Assets:A CNY"]
        );
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
        assert_eq!(paddings(&out), vec!["2023-01-01 Assets:A 10 CNY from Equity:Open"]);
    }

    #[test]
    fn a_pad_whose_preceding_directive_a_plugin_removed_is_put_back_on_its_date() {
        let plugin = plugin(|stream| stream.into_iter().filter(|it| !matches!(it.data, Directive::Transaction(_))).collect());
        let (out, errors) = run(
            plugin,
            indoc! {r#"
                1970-01-01 open Assets:Bank
                1970-01-01 open Equity:Open
                1970-01-01 open Equity:Other
                2024-01-01 * "coffee" ""
                  Assets:Bank -1 CNY
                  Equity:Other
                2024-01-01 pad Assets:Bank Equity:Open
                2024-01-01 * "tea" ""
                  Assets:Bank -1 CNY
                  Equity:Other
                2024-01-01 pad Assets:Bank Equity:Other
                2024-01-02 balance Assets:Bank 100 CNY
            "#},
        );
        // both pads are back, in their order: the later one pads the bank
        assert_eq!(
            pads(&out),
            vec!["2024-01-01 Assets:Bank from Equity:Open", "2024-01-01 Assets:Bank from Equity:Other"]
        );
        assert_eq!(paddings(&out), vec!["2024-01-01 Assets:Bank 100 CNY from Equity:Other"]);
        assert_eq!(errors, vec![ErrorKind::UnusedPad]);
    }

    #[test]
    fn a_pad_account_the_plugin_gives_a_served_balance_is_the_pad_account_of_its_pad() {
        let plugin = plugin(|stream| each_shown(stream, |pad| pad.pad = account("Equity:Other")));
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(pads(&out), vec!["2024-01-01 Assets:Bank from Equity:Other"]);
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-01 Assets:Bank 100 CNY from Equity:Other",
                "2024-01-01 Assets:Bank 20 USD from Equity:Other"
            ]
        );
    }

    #[test]
    fn a_pad_follows_a_plugin_renaming_its_account() {
        let plugin = plugin(|stream| {
            stream
                .into_iter()
                .map(|mut it| {
                    let rename = |it: &mut Account| {
                        if it.name() == "Assets:Bank" {
                            *it = account("Assets:Safe");
                        }
                    };
                    match &mut it.data {
                        Directive::Open(open) => rename(&mut open.account),
                        Directive::BalanceCheck(check) => rename(&mut check.account),
                        Directive::BalancePad(pad) => rename(&mut pad.account),
                        _ => {}
                    }
                    it
                })
                .collect()
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(pads(&out), vec!["2024-01-01 Assets:Safe from Equity:Open"]);
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-01 Assets:Safe 100 CNY from Equity:Open",
                "2024-01-01 Assets:Safe 20 USD from Equity:Open"
            ]
        );
    }

    #[test]
    fn a_plugin_turning_a_served_balance_into_a_plain_balance_leaves_its_pad_out() {
        let plugin = plugin(|stream| {
            stream
                .into_iter()
                .map(|mut it| {
                    if let Directive::BalancePad(pad) = &it.data {
                        if pad.amount.commodity == "CNY" {
                            it.data = Directive::BalanceCheck(BalanceCheck {
                                date: pad.date.clone(),
                                account: pad.account.clone(),
                                amount: pad.amount.clone(),
                                tolerance: None,
                                meta: pad.meta.clone(),
                            });
                        }
                    }
                    it
                })
                .collect()
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        // the CNY balance pads nothing and fails; the USD one, still `with pad`, pads itself, dated on it
        assert!(pads(&out).is_empty());
        assert_eq!(paddings(&out), vec!["2024-01-02 Assets:Bank 20 USD from Equity:Open"]);
        // the pad left out pads nothing
        assert_eq!(
            errors,
            vec![ErrorKind::UnusedPad, ErrorKind::AccountBalanceCheckError, ErrorKind::AccountBalanceCheckError]
        );
    }

    #[test]
    fn a_plugin_dropping_a_served_balance_leaves_its_pad_out() {
        let plugin = plugin(|stream| {
            stream
                .into_iter()
                .filter(|it| !matches!(&it.data, Directive::BalancePad(pad) if pad.amount.commodity == "CNY"))
                .collect()
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        // the pad does not move on to the next CNY balance, which fails; left out, it pads nothing
        assert!(pads(&out).is_empty());
        assert_eq!(paddings(&out), vec!["2024-01-02 Assets:Bank 20 USD from Equity:Open"]);
        assert_eq!(errors, vec![ErrorKind::UnusedPad, ErrorKind::AccountBalanceCheckError]);
    }

    #[test]
    fn served_balances_given_different_pad_accounts_leave_their_pad_out() {
        let plugin = plugin(|stream| {
            each_shown(stream, |pad| {
                if pad.amount.commodity == "USD" {
                    pad.pad = account("Equity:Fx");
                }
            })
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        // each `balance ... with pad` pads itself from its own pad account
        assert!(pads(&out).is_empty());
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-02 Assets:Bank 100 CNY from Equity:Open",
                "2024-01-02 Assets:Bank 20 USD from Equity:Fx"
            ]
        );
        // the pad itself pads nothing
        assert_eq!(errors, vec![ErrorKind::UnusedPad]);
    }

    #[test]
    fn a_served_balance_the_plugin_moves_before_its_pad_leaves_the_pad_out() {
        let plugin = plugin(|stream| {
            each_shown(stream, |pad| {
                if pad.amount.commodity == "CNY" {
                    pad.date = zhang_ast::Date::Date(chrono::NaiveDate::from_ymd_opt(2023, 12, 31).unwrap());
                }
            })
        });
        let (out, _) = run(plugin, TWO_CURRENCIES);
        // put back, the pad would serve the next CNY balance instead
        assert!(pads(&out).is_empty());
        assert_eq!(
            paddings(&out),
            vec![
                "2023-12-31 Assets:Bank 100 CNY from Equity:Open",
                "2024-01-02 Assets:Bank 20 USD from Equity:Open"
            ]
        );
    }

    #[test]
    fn a_balance_with_pad_the_plugin_adds_at_a_served_place_is_its_own() {
        // a copy of the served balance, from another pad account, after it: the pad serves the first, which leaves
        // nothing to pad to the copy
        let copying = plugin(|stream| {
            stream
                .into_iter()
                .flat_map(|it| {
                    let copy = match &it.data {
                        Directive::BalancePad(pad) => {
                            let mut pad = pad.clone();
                            pad.pad = account("Equity:Other");
                            Some(Spanned::new(Directive::BalancePad(pad), it.span.clone()))
                        }
                        _ => None,
                    };
                    std::iter::once(it).chain(copy)
                })
                .collect()
        });
        let (out, errors) = run(copying, TWO_CURRENCIES);
        assert_eq!(pads(&out), vec!["2024-01-01 Assets:Bank from Equity:Open"]);
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-01 Assets:Bank 100 CNY from Equity:Open",
                "2024-01-01 Assets:Bank 20 USD from Equity:Open"
            ]
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(out.iter().filter(|it| matches!(it, Directive::BalancePad(_))).count(), 2);

        // the same copy in place of a served balance turned plain: the plain balance comes first, so the pad would
        // serve it; the pad is left out, and the copy pads itself
        let replacing = plugin(|stream| {
            stream
                .into_iter()
                .flat_map(|it| match &it.data {
                    Directive::BalancePad(pad) if pad.amount.commodity == "CNY" => {
                        let plain = Directive::BalanceCheck(BalanceCheck {
                            date: pad.date.clone(),
                            account: pad.account.clone(),
                            amount: pad.amount.clone(),
                            tolerance: None,
                            meta: pad.meta.clone(),
                        });
                        let mut copy = pad.clone();
                        copy.pad = account("Equity:Other");
                        vec![Spanned::new(plain, it.span.clone()), Spanned::new(Directive::BalancePad(copy), it.span)]
                    }
                    _ => vec![it],
                })
                .collect()
        });
        let (out, _) = run(replacing, TWO_CURRENCIES);
        assert!(pads(&out).is_empty());
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-02 Assets:Bank 100 CNY from Equity:Other",
                "2024-01-02 Assets:Bank 20 USD from Equity:Open"
            ]
        );
    }

    /// two pads of the cash on one day: the later one pads, the earlier one is unused
    const TWO_PADS_OF_A_DAY: &str = indoc! {r#"
        1970-01-01 open Assets:Cash
        1970-01-01 open Equity:X
        1970-01-01 open Equity:Y
        1970-01-01 open Expenses:Food
        2024-01-03 pad Assets:Cash Equity:X
        2024-01-03 * "x" ""
          Assets:Cash -1 CNY
          Expenses:Food
        2024-01-03 pad Assets:Cash Equity:Y
        2024-01-06 balance Assets:Cash 50 CNY
    "#};

    #[test]
    fn an_unused_pad_a_plugin_moves_after_the_used_one_is_left_out_rather_than_pad() {
        // reversed, the stream puts the unused pad after the one that pads: put back, it would pad the balance
        let reversing = plugin(|mut stream| {
            stream.reverse();
            stream
        });
        let (out, errors) = run_reporting(reversing, TWO_PADS_OF_A_DAY);
        assert_eq!(pads(&out), vec!["2024-01-03 Assets:Cash from Equity:Y"]);
        assert_eq!(paddings(&out), vec!["2024-01-03 Assets:Cash 51 CNY from Equity:Y"]);
        // left out, it is still reported unused, as it is in the ledger
        let unused = vec![(ErrorKind::UnusedPad, "2024-01-03 pad Assets:Cash Equity:X".to_owned())];
        assert_eq!(errors, unused);

        // as it is, it stays unused
        let (out, errors) = run_reporting(plugin(|stream| stream), TWO_PADS_OF_A_DAY);
        assert_eq!(pads(&out).len(), 2);
        assert_eq!(paddings(&out), vec!["2024-01-03 Assets:Cash 51 CNY from Equity:Y"]);
        assert_eq!(errors, unused);
    }

    #[test]
    fn a_balance_a_plugin_adds_is_not_padded_by_a_pad_it_cannot_see() {
        let plugin = plugin(|mut stream| {
            let span = SpanInfo {
                start: 999_999,
                ..SpanInfo::default()
            };
            stream.push(Spanned::new(
                Directive::BalanceCheck(BalanceCheck {
                    date: zhang_ast::Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 4).unwrap()),
                    account: account("Assets:Bank"),
                    amount: zhang_ast::amount::Amount::new(5.into(), "EUR"),
                    tolerance: None,
                    meta: Default::default(),
                }),
                span,
            ));
            stream
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        // the pad pads what it padded, and the plugin's balance in EUR fails, as the plugin saw it
        let (plain, _) = crate::pipeline::test::run_builtin_stages(TWO_CURRENCIES);
        assert_eq!(paddings(&out), paddings(&plain));
        assert_eq!(pads(&out), pads(&plain));
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
    }

    #[test]
    fn a_plugin_dropping_every_served_balance_leaves_the_pad_out_reported_unused() {
        let plugin = plugin(|stream| stream.into_iter().filter(|it| !matches!(it.data, Directive::BalancePad(_))).collect());
        let (out, errors) = run_reporting(
            plugin,
            indoc! {r#"
                1970-01-01 open Assets:Bank
                1970-01-01 open Equity:Open
                2024-01-01 pad Assets:Bank Equity:Open
                2024-01-02 balance Assets:Bank 100 CNY
            "#},
        );
        // nothing is left for the pad: it is left out, and pads nothing
        assert!(pads(&out).is_empty());
        assert!(paddings(&out).is_empty());
        assert_eq!(errors, vec![(ErrorKind::UnusedPad, "2024-01-01 pad Assets:Bank Equity:Open".to_owned())]);
    }

    /// pads of one account, each serving one balance, in two commodities in turn
    fn pads_in_turn(count: usize) -> String {
        let mut text = String::from("1970-01-01 open Assets:A\n1970-01-01 open Equity:Open\n");
        let start = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        for index in 0..count {
            let pad = start + chrono::Duration::days(2 * index as i64);
            let commodity = if index % 2 == 0 { "CNY" } else { "USD" };
            text.push_str(&format!(
                "{pad} pad Assets:A Equity:Open\n{} balance Assets:A {} {commodity}\n",
                pad.succ_opt().unwrap(),
                index + 1
            ));
        }
        text
    }

    #[test]
    fn a_plain_balance_a_plugin_makes_of_the_last_served_one_leaves_the_other_pads_as_they_are() {
        let text = pads_in_turn(2000);
        let plugin = plugin(|mut stream| {
            let last = stream.iter().rposition(|it| matches!(it.data, Directive::BalancePad(_))).unwrap();
            if let Directive::BalancePad(pad) = stream[last].data.clone() {
                stream[last].data = Directive::BalanceCheck(BalanceCheck {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,
                    tolerance: None,
                    meta: pad.meta,
                });
            }
            stream
        });
        let started = std::time::Instant::now();
        let (out, errors) = run(plugin, &text);
        let elapsed = started.elapsed();
        // the last pad is left out; the one before does not pad the plain balance, which fails; the other pads and
        // their paddings are as they were
        let (plain, _) = crate::pipeline::test::run_builtin_stages(&text);
        assert_eq!(pads(&out).len(), 1999);
        assert_eq!(out.iter().filter(|it| matches!(it, Directive::BalancePad(_))).count(), 0);
        assert_eq!(paddings(&out), paddings(&plain)[..1999].to_vec());
        // the last pad, left out, pads nothing
        assert_eq!(errors, vec![ErrorKind::UnusedPad, ErrorKind::AccountBalanceCheckError]);
        // each balance is paired about once: no time quadratic in the pads
        assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn a_balance_a_plugin_adds_at_the_place_of_a_served_one_is_told_apart_by_what_it_says() {
        // a plain 90 CNY balance at the place of the served 100 CNY one, before it: the pad serves the 100 only
        let plugin = plugin(|stream| {
            stream
                .into_iter()
                .flat_map(|it| match &it.data {
                    Directive::BalancePad(pad) if pad.amount.commodity == "CNY" => {
                        let mut amount = pad.amount.clone();
                        amount.number = 90.into();
                        let added = Directive::BalanceCheck(BalanceCheck {
                            date: pad.date.clone(),
                            account: pad.account.clone(),
                            amount,
                            tolerance: None,
                            meta: Default::default(),
                        });
                        vec![Spanned::new(added, it.span.clone()), it]
                    }
                    _ => vec![it],
                })
                .collect()
        });
        let (out, errors) = run(plugin, TWO_CURRENCIES);
        assert_eq!(
            paddings(&out),
            vec![
                "2024-01-01 Assets:Bank 100 CNY from Equity:Open",
                "2024-01-01 Assets:Bank 20 USD from Equity:Open"
            ]
        );
        // the 90 is checked as it is, against the 100 padded
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
    }
}
