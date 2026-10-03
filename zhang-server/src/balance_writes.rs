//! The directives the balance tools write: the balance of an account page and the batch balance tool.
//!
//! A zhang ledger takes each row as it is, dated now: a `balance`, or a `balance ... with pad`, which pads itself.
//!
//! A beancount ledger is written so that beancount reads it as zhang does, and nothing ends up padded or checked
//! silently otherwise than asked. Beancount knows no times, and checks a `balance` at the start of its date, before
//! the transactions of that day. A `pad` serves the first `balance` of its account in every commodity on a later day:
//!
//! - a row, "my balance now", is a `balance` dated tomorrow, which covers every transaction of today; a transaction
//!   added later today changes it, as in beancount;
//! - the pad rows of an account are one `pad` dated today, from a single account, which serves their balances.
//!   When the account has a `pad` today already, from the same account, it serves them;
//! - that `pad` also serves the next balance of every other commodity of the account: the account's balance in each
//!   other commodity it holds is written along, which the `pad` serves without padding anything, so it cannot pad a
//!   later balance;
//! - a balance row without a pad must be served by no `pad`.
//!
//! What beancount would not read as asked is a 400 with the reason, and nothing is written.

use std::collections::{BTreeMap, HashMap};

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use indexmap::IndexMap;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Pad};
use zhang_core::data_type::is_beancount_endpoint;
use zhang_core::ledger::Ledger;
use zhang_core::pipeline::serving_pads;

use crate::error::ServerError;
use crate::ServerResult;

/// one balance of a request: the account's balance in a commodity, padded from `pad` when there is one
pub(crate) struct BalanceRow {
    pub account: Account,
    pub amount: Amount,
    pub pad: Option<Account>,
}

/// The directives `rows` write to `ledger`, made `now`.
pub(crate) fn balance_directives(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date) -> ServerResult<Vec<Directive>> {
    if is_beancount_endpoint(&ledger.entry.1) {
        return beancount_balances(ledger, rows, now.naive_date());
    }
    Ok(rows
        .into_iter()
        .map(|row| match row.pad {
            Some(pad) => Directive::BalancePad(BalancePad {
                date: now.clone(),
                account: row.account,
                amount: row.amount,
                pad,
                meta: Default::default(),
            }),
            None => check(now.clone(), row.account, row.amount),
        })
        .collect())
}

fn check(date: Date, account: Account, amount: Amount) -> Directive {
    Directive::BalanceCheck(BalanceCheck {
        date,
        account,
        amount,
        tolerance: None,
        meta: Default::default(),
    })
}

fn refused(message: String) -> ServerError {
    ServerError::InvalidInput(message)
}

/// an account padded by a request
struct Padded {
    account: Account,
    /// the account it is padded from
    source: Account,
    /// whether the `pad` is to be written: no `pad` of the account from `source` today yet
    write: bool,
}

/// the directives `rows` write to a beancount ledger `today`; see the module docs
fn beancount_balances(ledger: &Ledger, rows: Vec<BalanceRow>, today: NaiveDate) -> ServerResult<Vec<Directive>> {
    let tomorrow = today.succ_opt().unwrap_or(today);
    for (index, row) in rows.iter().enumerate() {
        if rows[..index]
            .iter()
            .any(|it| it.account == row.account && it.amount.commodity == row.amount.commodity)
        {
            return Err(refused(format!(
                "the balances have two rows for {} in {}: write one",
                row.account.name(),
                row.amount.commodity
            )));
        }
    }

    // the accounts padded, each from a single account
    let single = |account: &Account, detail: String| {
        refused(format!(
            "beancount pads an account from a single account per day: the balances of {} on {tomorrow} {detail}",
            account.name()
        ))
    };
    let mut padded: IndexMap<String, Padded> = IndexMap::new();
    for row in &rows {
        let Some(source) = &row.pad else { continue };
        if let Some(it) = padded.get(row.account.name()) {
            if &it.source != source {
                return Err(single(
                    &row.account,
                    format!("cannot be padded from both {} and {}", it.source.name(), source.name()),
                ));
            }
            continue;
        }
        let existing = ledger.directives.iter().find_map(|it| match &it.data {
            Directive::Pad(pad) if pad.account == row.account && pad.date.naive_date() == today => Some(&pad.pad),
            _ => None,
        });
        if let Some(existing) = existing.filter(|existing| *existing != source) {
            return Err(single(&row.account, format!("are padded from {} already", existing.name())));
        }
        padded.insert(
            row.account.name().to_owned(),
            Padded {
                account: row.account.clone(),
                source: source.clone(),
                write: existing.is_none(),
            },
        );
    }

    // a pad serves the first balance of each commodity of a day only
    let asserted = |account: &Account, commodity: &str| {
        ledger.directives.iter().any(|it| match &it.data {
            Directive::BalanceCheck(check) => &check.account == account && check.amount.commodity == commodity && check.date.naive_date() == tomorrow,
            Directive::BalancePad(pad) => &pad.account == account && pad.amount.commodity == commodity && pad.date.naive_date() == tomorrow,
            _ => false,
        })
    };
    for row in rows.iter().filter(|it| it.pad.is_some()) {
        if asserted(&row.account, &row.amount.commodity) {
            return Err(refused(format!(
                "beancount pads only the first balance of an account in each commodity per day: {} has a balance in {} on {tomorrow} already",
                row.account.name(),
                row.amount.commodity
            )));
        }
    }

    // what an account and its sub-accounts hold in a commodity at the balances: what they hold at the end of today,
    // and what the pads of its sub-accounts in this request bring their balances to
    let held = held_at_end_of(ledger, today);
    let expected = |name: &str, commodity: &str| {
        let padding = rows
            .iter()
            .filter(|row| row.pad.is_some() && row.amount.commodity == commodity && is_under(&row.account, name))
            // the outermost padded sub-accounts: a padded sub-account of theirs is counted in theirs
            .filter(|row| {
                !rows.iter().any(|other| {
                    other.pad.is_some() && other.amount.commodity == commodity && is_under(&other.account, name) && is_under(&row.account, other.account.name())
                })
            })
            .map(|row| &row.amount.number - held.subtree_in(row.account.name(), commodity))
            .fold(BigDecimal::zero(), |sum, it| sum + it);
        held.subtree_in(name, commodity) + padding
    };

    // an account whose pad rows pad nothing gets no new `pad`, which beancount would report unused: its rows are
    // plain balances. A `pad` of the ledger that serves one pads nothing either, and the row asked for a pad
    padded.retain(|name, it| {
        !it.write
            || rows
                .iter()
                .filter(|row| row.pad.is_some() && row.account.name() == name)
                .any(|row| row.amount.number != expected(name, &row.amount.commodity))
    });

    // the balance of every other commodity a padded account holds, which its `pad` serves without padding anything
    let mut along: HashMap<String, Vec<Amount>> = HashMap::new();
    for (name, it) in &padded {
        for (commodity, _) in held.subtree(name) {
            if rows.iter().any(|row| row.account == it.account && row.amount.commodity == commodity) || asserted(&it.account, &commodity) {
                continue;
            }
            let number = expected(name, &commodity);
            along.entry(name.clone()).or_default().push(Amount::new(number, commodity));
        }
    }

    // a balance without a pad must be served by no `pad`, of the ledger or of this request
    let mut new = padded
        .values()
        .filter(|it| it.write)
        .map(|it| {
            Directive::Pad(Pad {
                date: Date::Date(today),
                account: it.account.clone(),
                pad: it.source.clone(),
                meta: Default::default(),
            })
        })
        .collect::<Vec<_>>();
    let first_row = new.len();
    new.extend(rows.iter().map(|row| check(Date::Date(tomorrow), row.account.clone(), row.amount.clone())));
    for (name, amounts) in &along {
        let account = &padded[name].account;
        new.extend(amounts.iter().map(|amount| check(Date::Date(tomorrow), account.clone(), amount.clone())));
    }
    let serving = serving_pads(&ledger.directives, &new);
    for (row, pad) in rows.iter().zip(&serving[first_row..]) {
        let (None, Some(pad)) = (&row.pad, pad) else { continue };
        let commodity = &row.amount.commodity;
        let pad_written = padded.get(pad.account.name()).is_some_and(|it| it.write) && pad.date.naive_date() == today;
        return Err(refused(if pad_written {
            format!(
                "beancount pads every commodity of an account: the pad of {} from {} for these balances would also pad its balance in {commodity}. \
                 Check {commodity} with a pad from {} too",
                pad.account.name(),
                pad.pad.name(),
                pad.pad.name()
            )
        } else {
            format!(
                "beancount pads every commodity of an account: the pad of {} on {} from {} would also pad this balance in {commodity} on {tomorrow}. \
                 Check it with a pad, or close that pad first with a balance of {commodity} after it",
                pad.account.name(),
                pad.date.naive_date(),
                pad.pad.name()
            )
        }));
    }

    // the rows in their order, the balances of the other commodities of a padded account after its rows
    let mut last_row: HashMap<&str, usize> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if padded.contains_key(row.account.name()) {
            last_row.insert(row.account.name(), index);
        }
    }
    let mut written = HashMap::new();
    let mut directives = Vec::with_capacity(new.len());
    for (index, row) in rows.iter().enumerate() {
        let name = row.account.name();
        let directive = match (&row.pad, padded.get(name)) {
            // the first pad row of an account writes its `pad`: the exporter writes it on the day before the balance
            (Some(source), Some(it)) if it.write && written.insert(name.to_owned(), ()).is_none() => Directive::BalancePad(BalancePad {
                date: Date::Date(tomorrow),
                account: row.account.clone(),
                amount: row.amount.clone(),
                pad: source.clone(),
                meta: Default::default(),
            }),
            _ => check(Date::Date(tomorrow), row.account.clone(), row.amount.clone()),
        };
        directives.push(directive);
        if last_row.get(name) == Some(&index) {
            for amount in along.remove(name).unwrap_or_default() {
                directives.push(check(Date::Date(tomorrow), row.account.clone(), amount));
            }
        }
    }
    Ok(directives)
}

/// whether `account` is a strict sub-account of the account named `parent`
fn is_under(account: &Account, parent: &str) -> bool {
    account.name().len() > parent.len() && account.name().starts_with(parent) && account.name()[parent.len()..].starts_with(':')
}

/// the units every account holds at the end of a day: the sum of the postings dated on it or before
struct Held(BTreeMap<String, BTreeMap<String, BigDecimal>>);

fn held_at_end_of(ledger: &Ledger, day: NaiveDate) -> Held {
    let store = ledger.store.read().expect("poison lock detect");
    let mut held: BTreeMap<String, BTreeMap<String, BigDecimal>> = BTreeMap::new();
    for posting in store.postings.iter().filter(|it| it.trx_datetime.date_naive() <= day) {
        let units = held
            .entry(posting.account.name().to_owned())
            .or_default()
            .entry(posting.inferred_amount.commodity.clone())
            .or_insert_with(BigDecimal::zero);
        *units += &posting.inferred_amount.number;
    }
    Held(held)
}

impl Held {
    /// the units the account named `name` and its sub-accounts hold, in each commodity they hold some of
    fn subtree(&self, name: &str) -> Vec<(String, BigDecimal)> {
        let mut total: BTreeMap<String, BigDecimal> = BTreeMap::new();
        for (_, units) in self.accounts_under(name) {
            for (commodity, number) in units {
                *total.entry(commodity.clone()).or_insert_with(BigDecimal::zero) += number;
            }
        }
        total.into_iter().filter(|(_, number)| !number.is_zero()).collect()
    }

    /// the units the account named `name` and its sub-accounts hold in `commodity`
    fn subtree_in(&self, name: &str, commodity: &str) -> BigDecimal {
        self.accounts_under(name)
            .filter_map(|(_, units)| units.get(commodity))
            .fold(BigDecimal::zero(), |sum, it| sum + it)
    }

    fn accounts_under<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (&'a String, &'a BTreeMap<String, BigDecimal>)> + 'a {
        self.0
            .iter()
            .filter(move |(account, _)| account.as_str() == name || (account.starts_with(name) && account[name.len()..].starts_with(':')))
    }
}
