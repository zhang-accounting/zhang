//! The directives the balance tools write: the balance of an account page and the batch balance tool.
//!
//! A zhang ledger takes each row as it is, dated now: a `balance`, or a `balance ... with pad`, which pads itself.
//!
//! A beancount ledger is written so that beancount reads it as zhang does, and nothing is padded but what was asked,
//! when it was asked. Beancount knows no times, and checks a `balance` at the start of its date, before the
//! transactions of that day. So a row, "my balance now", is a `balance` dated tomorrow, which covers every
//! transaction of today. A row with a pad, "book the difference from that account", also writes the difference,
//! computed now from what the account and its sub-accounts hold, as a padding transaction (flag `P`) dated now.
//! The UI writes no `pad`: a `pad` would pad the next balance of every commodity of its account, and would silently
//! absorb a transaction added later today. Such a transaction makes tomorrow's `balance` fail instead.
//!
//! A `pad` the ledger has, written by hand, would still pad a balance written after it in a commodity it never
//! served, and absorb later transactions in it: such a balance is refused, with the `pad` to close first.
//!
//! In both ledgers, a balance of an account that is not open, or padded from an account that is not, is refused.
//! What is refused is a 400 with the reason, and nothing is written.

use std::collections::BTreeMap;

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Flag, Posting, Transaction, ZhangString};
use zhang_core::data_type::is_beancount_endpoint;
use zhang_core::domains::schemas::AccountStatus;
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
    refuse_accounts_not_open(ledger, &rows)?;
    if is_beancount_endpoint(&ledger.entry.1) {
        return beancount_balances(ledger, rows, now);
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

/// a balance of an account that is not open, or padded from one, would only be reported once written
fn refuse_accounts_not_open(ledger: &Ledger, rows: &[BalanceRow]) -> ServerResult<()> {
    let store = ledger.store.read().expect("poison lock detect");
    for row in rows {
        let accounts = std::iter::once((&row.account, "a balance of")).chain(row.pad.iter().map(|it| (it, "a pad from")));
        for (account, what) in accounts {
            let name = account.name();
            match store.accounts.get(name).map(|it| it.status) {
                Some(AccountStatus::Open) => {}
                Some(AccountStatus::Close) => {
                    return Err(refused(format!(
                        "{name} is closed: {what} {name} cannot be written. Reopen it, or pick an open account"
                    )))
                }
                None => return Err(refused(format!("{name} is not open: {what} {name} cannot be written. Open it first"))),
            }
        }
    }
    Ok(())
}

/// the directives `rows` write to a beancount ledger `now`; see the module docs
fn beancount_balances(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date) -> ServerResult<Vec<Directive>> {
    let today = now.naive_date();
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

    // a `pad` of the ledger, which beancount lets pad the next balance of every commodity of its account, must serve
    // none of these balances: it would pad what was not asked, and absorb a transaction added later today
    let balances = rows
        .iter()
        .map(|row| check(Date::Date(tomorrow), row.account.clone(), row.amount.clone()))
        .collect::<Vec<_>>();
    for (row, pad) in rows.iter().zip(serving_pads(&ledger.directives, &balances)) {
        let Some(pad) = pad else { continue };
        let commodity = &row.amount.commodity;
        let pad_date = pad.date.naive_date();
        return Err(refused(format!(
            "beancount pads every commodity of an account: the pad of {account} on {pad_date} from {source} would also pad \
             this balance in {commodity} on {tomorrow}, and absorb any transaction in {commodity} added before it. Close \
             that pad first: write a balance of {account} in {commodity} on {day_after}, right after it",
            account = pad.account.name(),
            source = pad.pad.name(),
            day_after = pad_date.succ_opt().unwrap_or(pad_date),
        )));
    }

    // what an account and its sub-accounts hold in a commodity at the balances: what they hold at the end of today,
    // and what the paddings of its sub-accounts in this request bring their balances to
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

    let mut directives = vec![];
    for row in &rows {
        let Some(source) = &row.pad else { continue };
        let difference = &row.amount.number - expected(row.account.name(), &row.amount.commodity);
        if !difference.is_zero() {
            directives.push(padding(
                now.clone(),
                &row.account,
                source,
                Amount::new(difference, row.amount.commodity.clone()),
            ));
        }
    }
    directives.extend(balances);
    Ok(directives)
}

/// the transaction booking `difference` to `account` from `source`, as zhang books the padding of a pad
fn padding(date: Date, account: &Account, source: &Account, difference: Amount) -> Directive {
    let posting = |account: &Account, units: Amount| Posting {
        flag: None,
        account: account.clone(),
        units: Some(units),
        cost: None,
        price: None,
        comment: None,
        meta: Default::default(),
    };
    let negated = Amount::new(-difference.number.clone(), difference.commodity.clone());
    Directive::Transaction(Transaction {
        date,
        flag: Some(Flag::BalancePad),
        payee: Some(ZhangString::quote("Balance Pad")),
        narration: Some(ZhangString::quote(format!("pad {} to {}", account.name(), source.name()))),
        tags: Default::default(),
        links: Default::default(),
        postings: vec![posting(account, difference), posting(source, negated)],
        meta: Default::default(),
    })
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
    /// the units the account named `name` and its sub-accounts hold in `commodity`
    fn subtree_in(&self, name: &str, commodity: &str) -> BigDecimal {
        self.0
            .iter()
            .filter(|(account, _)| account.as_str() == name || (account.starts_with(name) && account[name.len()..].starts_with(':')))
            .filter_map(|(_, units)| units.get(commodity))
            .fold(BigDecimal::zero(), |sum, it| sum + it)
    }
}
