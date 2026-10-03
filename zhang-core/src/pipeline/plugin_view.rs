//! The directive stream as a WASM plugin of ABI v1 sees it.
//!
//! ABI v1 lets zhang add fields, never directive kinds: a plugin built against an older
//! zhang-ast cannot read a kind it does not know. The `pad` directive came after v1, so a
//! plugin never sees one, and sees the stream it saw before `pad` existed:
//!
//! - every `pad` is set aside before the plugin runs, and put back after it, right after the
//!   directive it followed (by span), or at the end of the stream when that directive is gone.
//!   The re-sort after the stage puts it back on its date;
//! - a `balance` a `pad` serves is shown as the `balance ... with pad` a beancount ledger loaded
//!   as before, with the pad account of its `pad`, and turned back into the `balance`, with its
//!   tolerance, after the plugin. A plugin may change it like any directive: its amount, account
//!   and metadata are kept.
//!
//! Exposing `pad` to plugins is future ABI work.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bigdecimal::BigDecimal;
use zhang_ast::{Account, BalanceCheck, BalancePad, Directive, SpanInfo, Spanned};

use super::{ProcessStage, StageContext};
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

/// what [`hide_pads`] took out of the stream
struct HiddenPads {
    /// each `pad`, in stream order, with the place of the directive before it that is no `pad`
    pads: Vec<(Option<Place>, Spanned<Directive>)>,
    /// the tolerance of each `balance` shown as a `balance ... with pad`, by its place
    served: HashMap<Place, Option<BigDecimal>>,
}

/// the stream without its `pad` directives, the `balance` each serves shown as a `balance ... with pad`.
/// The stream is sorted: a `pad` serves the next `balance` of its account in each currency, until the
/// account's next `pad`, as the pad stage pairs them
fn hide_pads(directives: Vec<Spanned<Directive>>) -> (Vec<Spanned<Directive>>, HiddenPads) {
    // the account a `pad` pads -> (the account it pads from, the currencies it served already)
    let mut active: HashMap<String, (Account, HashSet<String>)> = HashMap::new();
    let mut hidden = HiddenPads {
        pads: vec![],
        served: HashMap::new(),
    };
    let mut before = None;
    let mut shown = Vec::with_capacity(directives.len());
    for directive in directives {
        let Spanned { data, span } = directive;
        let data = match data {
            Directive::Pad(pad) => {
                active.insert(pad.account.name().to_owned(), (pad.pad.clone(), HashSet::new()));
                hidden.pads.push((before.clone(), Spanned::new(Directive::Pad(pad), span)));
                continue;
            }
            Directive::BalanceCheck(check) => {
                let pad = active
                    .get_mut(check.account.name())
                    .and_then(|(pad, served)| served.insert(check.amount.commodity.clone()).then(|| pad.clone()));
                match pad {
                    Some(pad) => {
                        hidden.served.insert(place(&span), check.tolerance);
                        Directive::BalancePad(BalancePad {
                            date: check.date,
                            account: check.account,
                            amount: check.amount,
                            pad,
                            meta: check.meta,
                        })
                    }
                    None => Directive::BalanceCheck(check),
                }
            }
            Directive::BalancePad(pad) => {
                // an assertion the `pad` serves first, before the `balance ... with pad` pads the rest
                if let Some((_, served)) = active.get_mut(pad.account.name()) {
                    served.insert(pad.amount.commodity.clone());
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
    /// the stream a plugin returned, with the `balance` directives it was shown as `balance ... with pad`
    /// turned back, and the `pad` directives put back
    fn restore(self, returned: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        let HiddenPads { pads, served } = self;
        // the pads to put back after the directive at a place, in stream order
        let mut after: HashMap<Option<Place>, Vec<Spanned<Directive>>> = HashMap::new();
        for (before, pad) in pads {
            after.entry(before).or_default().push(pad);
        }
        let mut ret = after.remove(&None).unwrap_or_default();
        ret.reserve(returned.len());
        for directive in returned {
            let here = place(&directive.span);
            let Spanned { data, span } = directive;
            let data = match data {
                Directive::BalancePad(pad) if served.contains_key(&here) => Directive::BalanceCheck(BalanceCheck {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,
                    tolerance: served[&here].clone(),
                    meta: pad.meta,
                }),
                other => other,
            };
            ret.push(Spanned::new(data, span));
            ret.extend(after.remove(&Some(here)).unwrap_or_default());
        }
        // the directive a pad followed is gone: the re-sort puts the pad back on its date
        let mut rest = after.into_values().flatten().collect::<Vec<_>>();
        rest.sort_by_key(|pad| (pad.span.filename.clone(), pad.span.start));
        ret.extend(rest);
        ret
    }
}

#[cfg(test)]
mod test {
    use std::sync::Mutex;

    use indoc::indoc;
    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Directive, Spanned};

    use super::AbiV1View;
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
                "Transaction P",
                "Pad Assets:Bank",
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
}
