//! What the balance tools write: the balance of an account page and the batch balance tool.
//!
//! A zhang ledger takes each row as it is, dated now: a `balance`, or a `balance ... with pad`, which pads itself.
//!
//! A beancount ledger is written so that beancount reads it as zhang does, and nothing is padded but what was asked,
//! when it was asked. Beancount knows no times, and checks a `balance` at the start of its date, before the
//! transactions of that day. So a row, "my balance now", is a `balance` dated tomorrow, the start of tomorrow being the
//! end of today, which covers every transaction of today. A row with a pad, "book the difference from that account",
//! also writes the difference, computed now from what the account and its sub-accounts hold, as a padding transaction
//! (flag `P`) dated now. The UI writes no `pad`: a `pad` would pad the next balance of every commodity of its account,
//! and would silently absorb a transaction added later today. Such a transaction makes tomorrow's `balance` fail
//! instead, until the next check of the day replaces it: a balance of the account and commodity dated tomorrow is
//! rewritten in place, not written twice. A padding transaction written before stays, and the difference is computed
//! with it.
//!
//! A `pad` the ledger has, written by hand, would still pad a balance written after it in a commodity it never
//! served, and absorb later transactions in it: such a balance is refused, with the `pad` to close first.
//!
//! In both ledgers, these are refused: a balance of an account that is not open, or padded from an account that is
//! not; a pad from the account itself or one of its sub-accounts, which moves units within the total it asserts and
//! never changes it; and a pad of a commodity the account or a sub-account holds at cost, which would book units
//! without a cost. What is refused is a 400 with the reason, and nothing is written.

use std::collections::BTreeMap;

use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use gotcha::Schematic;
use serde::Serialize;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Flag, Posting, SpanInfo, Spanned, Transaction, ZhangString};
use zhang_core::data_type::is_beancount_endpoint;
use zhang_core::domains::schemas::AccountStatus;
use zhang_core::ledger::Ledger;
use zhang_core::pipeline::serving_pads;
use zhang_core::utils::string_::StringExt;

use crate::error::ServerError;
use crate::ServerResult;

/// one balance of a request: the account's balance in a commodity, padded from `pad` when there is one
pub(crate) struct BalanceRow {
    pub account: Account,
    pub amount: Amount,
    pub pad: Option<Account>,
}

/// What a balance request wrote.
#[derive(Serialize, Schematic)]
pub struct BalanceWriteEntity {
    /// the balances of a beancount ledger the request replaced: those of the same account and commodity for the same
    /// date, which the new balance supersedes
    pub replaced: Vec<ReplacedBalanceEntity>,
}

/// A balance a request replaced.
#[derive(Serialize, Schematic)]
pub struct ReplacedBalanceEntity {
    pub date: NaiveDate,
    pub account: String,
    /// the amount it asserted
    pub amount: Amount,
}

/// what a request writes: directives to append, and balances to rewrite in their files
pub(crate) struct BalanceWrites {
    append: Vec<Directive>,
    /// a balance's place in its file, and what it becomes; nothing for a balance a row replaced twice
    replace: Vec<(SpanInfo, Option<Directive>)>,
    replaced: Vec<ReplacedBalanceEntity>,
}

impl BalanceWrites {
    /// write to the ledger's files: the balances replaced first, in their place, then the directives appended
    pub(crate) async fn write(self, ledger: &Ledger) -> ServerResult<BalanceWriteEntity> {
        let BalanceWrites { append, mut replace, replaced } = self;
        // from the end of each file, so the places of the others stay
        replace.sort_by(|(a, _), (b, _)| (&b.filename, b.start).cmp(&(&a.filename, a.start)));
        let mut files: Vec<(String, String)> = vec![];
        for (span, directive) in replace {
            let Some(file) = span.filename.as_ref().map(|it| it.to_string_lossy().to_string()) else {
                continue;
            };
            if files.last().is_none_or(|(last, _)| last != &file) {
                let content = String::from_utf8(ledger.data_source.async_get(file.clone()).await?).map_err(|e| ServerError::InvalidInput(e.to_string()))?;
                files.push((file.clone(), content));
            }
            let text = match directive {
                Some(directive) => String::from_utf8_lossy(&ledger.data_source.export(directive)?).to_string(),
                None => String::new(),
            };
            let (_, content) = files.last_mut().expect("the file is read");
            content.replace_by_span(&span, &text);
        }
        for (file, content) in files {
            ledger.data_source.async_save(ledger, file, content.as_bytes()).await?;
        }
        if !append.is_empty() {
            ledger.data_source.async_append(ledger, append).await?;
        }
        Ok(BalanceWriteEntity { replaced })
    }
}

/// What `rows` write to `ledger`, made `now`.
pub(crate) fn balance_directives(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date) -> ServerResult<BalanceWrites> {
    refuse_accounts_not_open(ledger, &rows)?;
    let held = held_at_end_of(ledger, now.naive_date());
    refuse_pads_that_cannot_pass(ledger, &rows, &held)?;
    if is_beancount_endpoint(&ledger.entry.1) {
        return beancount_balances(ledger, rows, now, &held);
    }
    let append = rows
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
        .collect();
    Ok(BalanceWrites {
        append,
        replace: vec![],
        replaced: vec![],
    })
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

/// a pad row that can only be reported once written: from the account itself or a sub-account, or of a commodity
/// held at cost
fn refuse_pads_that_cannot_pass(ledger: &Ledger, rows: &[BalanceRow], held: &Held) -> ServerResult<()> {
    let store = ledger.store.read().expect("poison lock detect");
    for row in rows {
        let Some(source) = &row.pad else { continue };
        let account = row.account.name();
        if source == &row.account || is_under(source, account) {
            return Err(refused(format!(
                "{account} cannot be padded from {}, which its balance covers: the padding would move units within the \
                 balance it asserts, and never change it. Pad it from another account",
                source.name()
            )));
        }
        let commodity = &row.amount.commodity;
        let difference = &row.amount.number - held.subtree_in(account, commodity);
        if difference.is_zero() {
            continue;
        }
        let at_cost = store
            .commodity_lots
            .iter()
            .filter(|(name, _)| name.as_str() == account || name.strip_prefix(account).is_some_and(|rest| rest.starts_with(':')))
            .flat_map(|(_, lots)| lots)
            .any(|lot| &lot.commodity == commodity && lot.cost.is_some() && !lot.amount.is_zero());
        if at_cost {
            return Err(refused(format!(
                "{account} holds {commodity} at cost: padding it to {} would book {difference} {commodity} without a cost. \
                 Record them with their cost instead, as a purchase or a sale",
                row.amount
            )));
        }
    }
    Ok(())
}

/// what `rows` write to a beancount ledger `now`; see the module docs
fn beancount_balances(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date, held: &Held) -> ServerResult<BalanceWrites> {
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

    // the balances of the same account and commodity for tomorrow, which the new ones replace
    let replaced = |directive: &Spanned<Directive>| match &directive.data {
        Directive::BalanceCheck(check) => {
            directive.span.filename.is_some()
                && check.date.naive_date() == tomorrow
                && rows
                    .iter()
                    .any(|row| row.account == check.account && row.amount.commodity == check.amount.commodity)
        }
        _ => false,
    };

    // a `pad` of the ledger, which beancount lets pad the next balance of every commodity of its account, must serve
    // none of these balances: it would pad what was not asked, and absorb a transaction added later today
    let balances = rows
        .iter()
        .map(|row| check(Date::Date(tomorrow), row.account.clone(), row.amount.clone()))
        .collect::<Vec<_>>();
    for (row, pad) in rows.iter().zip(serving_pads(&ledger.directives, replaced, &balances)) {
        let Some(pad) = pad else { continue };
        let commodity = &row.amount.commodity;
        let pad_date = pad.date.naive_date();
        let file = ledger
            .directives
            .iter()
            .find(|it| matches!(&it.data, Directive::Pad(it) if it == &pad))
            .and_then(|it| it.span.filename.as_ref())
            .map(|it| {
                // the file within the ledger's directory, which a local ledger names by its full path
                let root = ledger.entry.0.canonicalize().unwrap_or_else(|_| ledger.entry.0.clone());
                it.strip_prefix(&root)
                    .or_else(|_| it.strip_prefix(&ledger.entry.0))
                    .unwrap_or(it)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| ledger.entry.1.clone());
        return Err(refused(format!(
            "beancount pads every commodity of an account: the pad of {account} on {pad_date} from {source}, in {file}, \
             would also pad this balance in {commodity} on {tomorrow}, and absorb any transaction in {commodity} added \
             before it. Close that pad first: edit {file}, and add a balance of {account} in {commodity} on {day_after}, \
             right after it",
            account = pad.account.name(),
            source = pad.pad.name(),
            day_after = pad_date.succ_opt().unwrap_or(pad_date),
        )));
    }

    // what an account and its sub-accounts hold in a commodity at the balances: what they hold at the end of today,
    // and what the paddings of its sub-accounts in this request bring their balances to
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

    let mut append = vec![];
    for row in &rows {
        let Some(source) = &row.pad else { continue };
        let difference = &row.amount.number - expected(row.account.name(), &row.amount.commodity);
        if !difference.is_zero() {
            append.push(padding(
                now.clone(),
                &row.account,
                source,
                Amount::new(difference, row.amount.commodity.clone()),
            ));
        }
    }

    // each new balance in place of the first it replaces; the others it replaces go
    let mut writes = BalanceWrites {
        append: vec![],
        replace: vec![],
        replaced: vec![],
    };
    for (row, balance) in rows.iter().zip(balances) {
        let mut balance = Some(balance);
        for directive in ledger.directives.iter().filter(|it| replaced(it)) {
            let Directive::BalanceCheck(old) = &directive.data else { continue };
            if old.account != row.account || old.amount.commodity != row.amount.commodity {
                continue;
            }
            writes.replaced.push(ReplacedBalanceEntity {
                date: tomorrow,
                account: old.account.name().to_owned(),
                amount: old.amount.clone(),
            });
            writes.replace.push((directive.span.clone(), balance.take()));
        }
        append.extend(balance);
    }
    writes.append = append;
    Ok(writes)
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
