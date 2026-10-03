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
//!   or added an assertion of the account before one. The balances the plugin returned as
//!   `balance ... with pad` stay so, and each pads its own assertion;
//! - a `pad` that serves no balance is invisible to a plugin: it is put back as it was, and
//!   serves the assertions of its account a plugin added after it.
//!
//! A plugin that changes nothing gets exactly the stream it was given back, with the `pad`s.
//! Exposing `pad` to plugins is future ABI work.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bigdecimal::BigDecimal;
use zhang_ast::{Account, BalanceCheck, BalancePad, Directive, Pad, SpanInfo, Spanned};

use super::pad::PadPairing;
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
        let (shown, hidden) = hide_pads(directives);
        let returned = self.plugin.process(shown, ctx)?;
        Ok(hidden.restore(returned))
    }
}

/// where a directive is: its file and position, the same for a directive a plugin passed through
type Place = (Option<PathBuf>, usize, usize);

fn place(span: &SpanInfo) -> Place {
    (span.filename.clone(), span.start, span.end)
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
/// paired as the pad stage pairs them
fn hide_pads(directives: Vec<Spanned<Directive>>) -> (Vec<Spanned<Directive>>, HiddenPads) {
    let mut pairing = PadPairing::default();
    let mut hidden = HiddenPads {
        pads: vec![],
        shown: HashMap::new(),
        padded: HashMap::new(),
    };
    let mut before = None;
    let mut shown = Vec::with_capacity(directives.len());
    for directive in directives {
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
                if let Some(index) = pairing.serve(&pad.account, &pad.amount.commodity, pad.date.naive_date()).copied() {
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
    fn restore(self, returned: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
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
        // the place of each pad in `out`
        let mut pad_of: HashMap<usize, usize> = HashMap::with_capacity(count);
        let mut put_back = |indices: Vec<usize>, pads: &mut [Option<HiddenPad>], out: &mut Vec<Spanned<Directive>>| {
            for index in indices {
                let HiddenPad { pad, span, .. } = pads[index].take().expect("a pad is put back once");
                pad_of.insert(out.len(), index);
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
            let position = pad_of
                .iter()
                .find_map(|(position, it)| (*it == index).then_some(*position))
                .expect("every pad is back");
            if let Directive::Pad(pad) = &mut out[position].data {
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
        // a pad put back must serve exactly the balances it stands for; leaving one out may change what an earlier
        // pad of its account serves, so check again until none is left out
        loop {
            let paired = pair(&out, &pad_of, &keep);
            let mut left_out = false;
            for index in 0..count {
                if !keep[index] || (serves[index] == 0 && serves_first[index] == 0) {
                    continue;
                }
                let mut expected = claims[index].iter().chain(&natives[index]).copied().collect::<Vec<_>>();
                expected.sort_unstable();
                if paired[index] == expected {
                    continue;
                }
                keep[index] = false;
                left_out = true;
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
            if !left_out {
                break;
            }
        }
        out.into_iter()
            .enumerate()
            .filter(|(position, _)| pad_of.get(position).is_none_or(|index| keep[*index]))
            .map(|(_, directive)| directive)
            .collect()
    }
}

/// the places in `out` of the assertions each pad serves, as the pad stage pairs them once `out` is sorted, with the
/// pads not kept left out
fn pair(out: &[Spanned<Directive>], pad_of: &HashMap<usize, usize>, keep: &[bool]) -> Vec<Vec<usize>> {
    let keys = Ledger::sort_keys(out);
    let mut order = (0..out.len())
        .filter(|position| match &out[*position].data {
            Directive::Pad(_) => pad_of.get(position).is_none_or(|index| keep[*index]),
            Directive::BalanceCheck(_) | Directive::BalancePad(_) => true,
            _ => false,
        })
        .collect::<Vec<_>>();
    // the sort is stable: the key, then the place in `out`
    order.sort_by_key(|position| (keys[*position], *position));
    // a `pad` the plugin emitted itself is paired too, and stands for nothing
    let mut pairing: PadPairing<Option<usize>> = PadPairing::default();
    let mut paired = vec![vec![]; keep.len()];
    for position in order {
        let served = match &out[position].data {
            Directive::Pad(pad) => {
                pairing.pad(&pad.account, pad.date.naive_date(), pad_of.get(&position).copied());
                continue;
            }
            Directive::BalanceCheck(check) => pairing.serve(&check.account, &check.amount.commodity, check.date.naive_date()),
            Directive::BalancePad(pad) => pairing.serve(&pad.account, &pad.amount.commodity, pad.date.naive_date()),
            _ => None,
        };
        if let Some(Some(index)) = served {
            paired[*index].push(position);
        }
    }
    for served in &mut paired {
        served.sort_unstable();
    }
    paired
}

#[cfg(test)]
mod test {
    use std::str::FromStr;
    use std::sync::Mutex;

    use indoc::indoc;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Account, BalanceCheck, BalancePad, Directive, Spanned};

    use super::{hide_pads, AbiV1View};
    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;
    use crate::ledger::Ledger;
    use crate::pipeline::{builtin_stages, run_pipeline, ProcessStage, StageContext};
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
            .chain(builtin_stages())
            .collect();
        let mut ctx = StageContext::new(&[]);
        let out = run_pipeline(&stages, parse(content), &mut ctx).unwrap();
        let errors = ctx.into_errors().into_iter().map(|it| it.kind).collect();
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
        let (shown, hidden) = hide_pads(original.clone());
        assert!(!shown.iter().any(|it| matches!(it.data, Directive::Pad(_))));
        assert_eq!(hidden.restore(shown), original);
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
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError, ErrorKind::AccountBalanceCheckError]);
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
        // the pad does not move on to the next CNY balance, which fails
        assert!(pads(&out).is_empty());
        assert_eq!(paddings(&out), vec!["2024-01-02 Assets:Bank 20 USD from Equity:Open"]);
        assert_eq!(errors, vec![ErrorKind::AccountBalanceCheckError]);
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
        assert!(errors.is_empty(), "{errors:?}");
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
}
