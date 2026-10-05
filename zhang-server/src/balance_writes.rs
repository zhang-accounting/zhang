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
//! instead, until the next check of the day replaces it: each balance of the account and commodity dated tomorrow
//! asserts exactly the new amount, in its place, without the tolerance it may have had, and its comment and metadata
//! stay. None is written twice. A padding transaction
//! written before stays, and the difference is computed with it. A file changed since the ledger was loaded is not
//! edited: its places are stale, and the write is a 409.
//!
//! A `pad` the ledger has, written by hand, would still pad a balance written after it in a commodity it never
//! served, and absorb later transactions in it: such a balance is refused, with the `pad` to close first.
//!
//! In both ledgers, these are refused, by the rule the ledger checks its directives with: a balance of an account that
//! is not opened by the time it is checked (a balance may follow the close), and a pad to or from an account that is
//! not open now (an account is active through the day of a `close` with only a date, and until the time of one with a
//! time); a pad from the account itself or one of its sub-accounts, which moves units within the total it asserts and
//! never changes it; and a pad of a commodity the account or a sub-account holds at cost, which would book units
//! without a cost. What is refused is a 400 with the reason, and nothing is written.

use std::collections::BTreeMap;

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use gotcha::Schematic;
use serde::Serialize;
use zhang_ast::account::is_under;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Flag, Posting, SpanInfo, Spanned, Transaction, ZhangString};
use zhang_core::data_source::loaded_file;
use zhang_core::data_type::text::exporter::ZhangDataTypeExportable;
use zhang_core::data_type::text::parser::balance_amount_span;
use zhang_core::data_type::Dialect;
use zhang_core::ledger::Ledger;
use zhang_core::pipeline::{serving_pads, AccountUse};
use zhang_core::utils::plain_decimal;
use zhang_core::utils::string_::StringExt;
use zhang_query::{Params, Query, QueryResult};

use crate::cells::first_row;
use crate::error::ServerError;
use crate::routes::query::{execute_options, max_result_values};
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
    /// the tolerance (`~`) it was written with, which the new balance does not keep: a balance from the balance tools
    /// is exact. Null for an exact one
    #[serde(serialize_with = "zhang_shared::decimal::plain::serialize_option")]
    pub tolerance: Option<BigDecimal>,
}

/// what a request writes: directives to append, and balances to rewrite in their files
pub(crate) struct BalanceWrites {
    append: Vec<Directive>,
    /// each balance replaced, at its place in its file, and the directive replacing it
    replace: Vec<(SpanInfo, Directive)>,
    replaced: Vec<ReplacedBalanceEntity>,
}

impl BalanceWrites {
    /// write to the ledger's files: the balances replaced first, in their place, then the directives appended. A
    /// file whose balances are not where the ledger loaded them is left as it is: [`ZhangError::FileChanged`]
    ///
    /// [`ZhangError::FileChanged`]: zhang_core::ZhangError::FileChanged
    pub(crate) async fn write(self, ledger: &Ledger) -> ServerResult<BalanceWriteEntity> {
        let BalanceWrites { append, replace, replaced } = self;
        // by file; in a file from the end, so the places of the others stay
        let mut files: BTreeMap<String, Vec<(SpanInfo, Directive)>> = BTreeMap::new();
        for (span, directive) in replace {
            let Some(file) = span.filename.as_ref().map(|it| it.to_string_lossy().to_string()) else {
                continue;
            };
            files.entry(file).or_default().push((span, directive));
        }
        let mut edited = vec![];
        for (file, mut replacements) in files {
            let spans = replacements.iter().map(|(span, _)| span.clone()).collect::<Vec<_>>();
            let mut content = ledger.data_source.async_get_unchanged(file.clone(), &spans).await?;
            replacements.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
            for (span, directive) in replacements {
                let text = match (&directive, with_amount_of(&span.content, &directive)) {
                    (_, Some(text)) => text,
                    (directive, None) => String::from_utf8_lossy(&ledger.data_source.export(directive.clone())?).to_string(),
                };
                content.text.replace_by_span(&span, &text);
            }
            edited.push((file, content));
        }
        for (file, content) in edited {
            ledger.data_source.async_save(ledger, file, &content.into_bytes()).await?;
        }
        if !append.is_empty() {
            ledger.data_source.async_append(ledger, append).await?;
        }
        Ok(BalanceWriteEntity { replaced })
    }
}

/// `text`, a `balance` as written, asserting exactly the amount of `balance` instead: its amount, from the number to
/// the commodity, with any tolerance (`~`) in between ([`balance_amount_span`]), is the new one as the exporter writes
/// it, and its comment and metadata stay as written. `None` when `text` is not a `balance`
fn with_amount_of(text: &str, balance: &Directive) -> Option<String> {
    let Directive::BalanceCheck(balance) = balance else { return None };
    let amount = balance_amount_span(text)?;
    Some(format!("{}{}{}", &text[..amount.start], balance.amount.clone().export(), &text[amount.end..]))
}

/// What `rows` write to `ledger`, made `now`.
pub(crate) fn balance_directives(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date) -> ServerResult<BalanceWrites> {
    // a beancount ledger checks the balance at the start of tomorrow, and books the padding now
    let checked_at = match ledger.dialect {
        Dialect::Beancount => now.naive_date().succ_opt().unwrap_or(now.naive_date()).and_time(NaiveTime::MIN),
        Dialect::Zhang => now.naive_datetime(),
    };
    refuse_accounts_not_open(ledger, &rows, checked_at, now.naive_datetime())?;
    refuse_pads_that_cannot_pass(ledger, &rows, now.naive_date())?;
    if ledger.dialect == Dialect::Beancount {
        return beancount_balances(ledger, rows, now);
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

/// a row that would only be reported once written, by the rule the ledger checks its directives with: a balance of an
/// account not opened by the time it is `checked_at` (a balance may follow the close, as it only records), or a padding,
/// booked at `padded_at`, to or from an account that is not open then
fn refuse_accounts_not_open(ledger: &Ledger, rows: &[BalanceRow], checked_at: NaiveDateTime, padded_at: NaiveDateTime) -> ServerResult<()> {
    for row in rows {
        let padding = row.pad.iter().flat_map(|pad| {
            [
                (&row.account, "a balance of", padded_at, AccountUse::Books),
                (pad, "a pad from", padded_at, AccountUse::Books),
            ]
        });
        let references = std::iter::once((&row.account, "a balance of", checked_at, AccountUse::Records)).chain(padding);
        for (account, what, at, usage) in references {
            let name = account.name();
            match ledger.account_reference_error(account, at, usage) {
                None => {}
                Some(ErrorKind::AccountClosed) => {
                    return Err(refused(format!(
                        "{name} is closed: {what} {name} cannot be written. Reopen it, or pick an open account"
                    )))
                }
                Some(_) => return Err(refused(format!("{name} is not open: {what} {name} cannot be written. Open it first"))),
            }
        }
    }
    Ok(())
}

/// a pad row that can only be reported once written: from the account itself or a sub-account, or of a commodity
/// held at cost at the end of `today`, which the padding is booked with
fn refuse_pads_that_cannot_pass(ledger: &Ledger, rows: &[BalanceRow], today: NaiveDate) -> ServerResult<()> {
    for row in rows {
        let Some(source) = &row.pad else { continue };
        let account = row.account.name();
        if is_under(source.name(), account) {
            return Err(refused(format!(
                "{account} cannot be padded from {}, which its balance covers: the padding would move units within the \
                 balance it asserts, and never change it. Pad it from another account",
                source.name()
            )));
        }
        let commodity = &row.amount.commodity;
        let difference = &row.amount.number - held_units(ledger, account, commodity, today)?;
        if difference.is_zero() {
            continue;
        }
        if !held(LOTS_AT_COST, ledger, account, commodity, today)?.rows.is_empty() {
            return Err(refused(format!(
                "{account} holds {commodity} at cost: padding it to {} would book {} {commodity} without a cost. \
                 Record them with their cost instead, as a purchase or a sale",
                row.amount,
                plain_decimal(&difference)
            )));
        }
    }
    Ok(())
}

/// what `rows` write to a beancount ledger `now`; see the module docs
fn beancount_balances(ledger: &Ledger, rows: Vec<BalanceRow>, now: Date) -> ServerResult<BalanceWrites> {
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

    // the balances of the same account and commodity for tomorrow, which the new ones replace: those in a file of
    // the ledger, where they can be. One a plugin made is in none (#476), so the new one is appended
    let replaced = |directive: &Spanned<Directive>| match &directive.data {
        Directive::BalanceCheck(check) => {
            loaded_file(ledger, &directive.span).is_some()
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
    for (row, pad) in rows.iter().zip(serving_pads(&ledger.directives, ledger.dialect, replaced, &balances)) {
        let Some(pad) = pad else { continue };
        let commodity = &row.amount.commodity;
        let pad_date = pad.date.naive_date();
        let file = ledger
            .directives
            .iter()
            .find(|it| matches!(&it.data, Directive::Pad(it) if it == &pad))
            .and_then(|it| it.span.filename.as_ref())
            .map(|it| ledger.path_in_ledger(it).unwrap_or_else(|| it.clone()).display().to_string())
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
    let expected = |name: &str, commodity: &str| -> ServerResult<BigDecimal> {
        let padding = rows
            .iter()
            .filter(|row| row.pad.is_some() && row.amount.commodity == commodity && below(&row.account, name))
            // the outermost padded sub-accounts: a padded sub-account of theirs is counted in theirs
            .filter(|row| {
                !rows.iter().any(|other| {
                    other.pad.is_some() && other.amount.commodity == commodity && below(&other.account, name) && below(&row.account, other.account.name())
                })
            })
            .try_fold(BigDecimal::zero(), |sum, row| {
                ServerResult::Ok(sum + (&row.amount.number - held_units(ledger, row.account.name(), commodity, today)?))
            })?;
        Ok(held_units(ledger, name, commodity, today)? + padding)
    };

    let mut append = vec![];
    for row in &rows {
        let Some(source) = &row.pad else { continue };
        let difference = &row.amount.number - expected(row.account.name(), &row.amount.commodity)?;
        if !difference.is_zero() {
            append.push(padding(
                now.clone(),
                &row.account,
                source,
                Amount::new(difference, row.amount.commodity.clone()),
            ));
        }
    }

    // each new balance in place of those of the same account and commodity for tomorrow, which assert its amount
    // instead; appended when there is none
    let mut writes = BalanceWrites {
        append: vec![],
        replace: vec![],
        replaced: vec![],
    };
    for (row, balance) in rows.iter().zip(balances) {
        let mut replacing = false;
        for directive in ledger.directives.iter().filter(|it| replaced(it)) {
            let Directive::BalanceCheck(old) = &directive.data else { continue };
            if old.account != row.account || old.amount.commodity != row.amount.commodity {
                continue;
            }
            writes.replaced.push(ReplacedBalanceEntity {
                date: tomorrow,
                account: old.account.name().to_owned(),
                amount: old.amount.clone(),
                tolerance: old.tolerance.clone(),
            });
            writes.replace.push((directive.span.clone(), balance.clone()));
            replacing = true;
        }
        if !replacing {
            append.push(balance);
        }
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
        written: None,
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
fn below(account: &Account, parent: &str) -> bool {
    is_under(account.name(), parent) && account.name() != parent
}

/// The units the account `:account` and its sub-accounts hold of `:commodity` at the end of `:day`: those of the booked
/// postings dated on it or before.
const HELD_UNITS: &str = "SELECT sum(number) AS units WHERE under(account, :account) AND currency = :commodity AND date <= :day";

/// The lots of `:commodity` the account `:account` and its sub-accounts hold at cost at the end of `:day`: those of the
/// booked postings dated on it or before with a cost, and units left.
const LOTS_AT_COST: &str = "SELECT account WHERE under(account, :account) AND currency = :commodity AND date <= :day \
                            AND cost_number IS NOT NULL \
                            GROUP BY account, cost_date, cost_number, cost_currency, cost_label HAVING sum(number) != 0";

/// The result of `query`, [`HELD_UNITS`] or [`LOTS_AT_COST`], for the account named `account` and its sub-accounts, in
/// `commodity`, at the end of `day`.
fn held(query: &str, ledger: &Ledger, account: &str, commodity: &str, day: NaiveDate) -> ServerResult<QueryResult> {
    let params = Params::new().bind("account", account).bind("commodity", commodity).bind("day", day);
    let query = Query::compile_with_params(query, &params.types())?;
    Ok(query.execute_with_options(ledger, &params, &execute_options(max_result_values()))?)
}

/// The units the account named `account` and its sub-accounts hold of `commodity` at the end of `day`.
fn held_units(ledger: &Ledger, account: &str, commodity: &str, day: NaiveDate) -> ServerResult<BigDecimal> {
    let result = held(HELD_UNITS, ledger, account, commodity, day)?;
    Ok(first_row(HELD_UNITS, &result)
        .map(|row| row.decimal("units"))
        .transpose()?
        .flatten()
        .unwrap_or_default())
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Account, Date};

    use super::{check, with_amount_of};

    /// A balance replaced in its place asserts the new amount, its tolerance dropped, and keeps its comment and
    /// metadata as written, also a comment glued to its commodity, which the grammar reads as a comment
    #[test]
    fn a_balance_replaced_in_its_place_keeps_its_comment_and_metadata() {
        let day = Date::Date(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());
        let balance = check(day, Account::from_str("Assets:A").unwrap(), Amount::new(BigDecimal::from(61), "CNY"));
        for (written, replaced) in [
            ("2024-01-16 balance Assets:A 60 CNY", "2024-01-16 balance Assets:A 61 CNY"),
            (
                "2024-01-16 balance Assets:A 60 ~ 0.01 CNY ; a comment",
                "2024-01-16 balance Assets:A 61 CNY ; a comment",
            ),
            ("2024-01-16 balance Assets:A 60 CNY;glued", "2024-01-16 balance Assets:A 61 CNY;glued"),
            (
                "2024-01-16  balance  Assets:A  60.00 CNY\n  statement: \"a.pdf\"",
                "2024-01-16  balance  Assets:A  61 CNY\n  statement: \"a.pdf\"",
            ),
        ] {
            assert_eq!(with_amount_of(written, &balance).as_deref(), Some(replaced), "{written:?}");
        }
        assert_eq!(with_amount_of("2024-01-16 pad Assets:A Equity:Open", &balance), None);
    }
}
