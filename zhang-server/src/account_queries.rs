//! The account endpoints on the query engine (#479): every figure of `GET /api/accounts`,
//! `/api/accounts/{a}`, `/api/accounts/{a}/journals`, `/api/accounts/{a}/balances` and
//! `/api/accounts/{a}/documents` comes from a named, documented built-in query, and this module
//! only maps the query results into the existing response shapes.
//!
//! A parent account's page is its subtree (decision 6 of #479): its journal, balance history,
//! header total and documents cover the account and all its sub-accounts, as the account tree
//! does. Balances are valued in the operating currency at today's prices with `convert`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveTime};
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::Account;
use zhang_core::constants::BALANCE_CHECK_PAYEE;
use zhang_core::domains::schemas::AccountStatus;
use zhang_core::ledger::Ledger;
use zhang_query::{Inventory, Params, QueryResult};

use crate::builtin::{self, calculated_amount};
use crate::journals::BalanceAssertion;
use crate::response::{AccountBalanceHistoryEntity, AccountBalanceItemEntity, AccountEntity, AccountInfoEntity, AccountJournalEntity, DocumentEntity};
use crate::state::SharedLedger;
use crate::{cells, ServerResult};

/// The built-in queries of the account endpoints, in [`crate::builtin::BUILTINS`].
const LIST: &str = "accounts.list";
const BALANCES: &str = "accounts.balances";
const SUBTREE: &str = "accounts.subtree";
const SUBTREE_BALANCES: &str = "accounts.subtree_balances";
const JOURNAL: &str = "accounts.journal";
const JOURNAL_PAGE: &str = "accounts.journal_page";
const BALANCE_ASSERTIONS: &str = "accounts.balance_assertions";
const BALANCE_HISTORY: &str = "accounts.balance_history";
const DOCUMENTS: &str = "accounts.documents";

/// Run a built-in query with the limits of every query the server runs.
fn run(ledger: &Ledger, name: &str, params: &Params) -> ServerResult<QueryResult> {
    builtin::execute(ledger, name, params, false)
}

/// Run `f` on the ledger off the async workers, under one read lock, so the queries of one
/// response see the same ledger.
pub async fn with_ledger<T: Send + 'static>(ledger: &SharedLedger, f: impl FnOnce(&Ledger) -> ServerResult<T> + Send + 'static) -> ServerResult<T> {
    crate::routes::query::with_ledger(&ledger.0, f).await
}

// ---------------------------------------------------------------------------------------
// account list and info

/// What the list and the account page show of an account: its directives, its own balance and
/// that of its subtree.
#[derive(Default)]
struct Summary {
    open: Option<NaiveDate>,
    close: Option<NaiveDate>,
    /// its status now by the ledger's clock, by the account lifecycle: `open`, `closed`, or none when neither an `open`
    /// nor a `close` of it is in effect
    status: Option<String>,
    alias: Option<String>,
    first_posting: Option<NaiveDate>,
    /// the balance of its own postings
    own: Balance,
    /// the balance of the postings of the account and all its sub-accounts
    subtree: Balance,
    has_sub_accounts: bool,
}

/// The units and the value of some postings, and every currency they hold or held.
#[derive(Default)]
struct Balance {
    units: Inventory,
    /// their value at today's prices
    value: Inventory,
    /// the currencies of the postings: the response keeps a currency that is back at zero
    currencies: BTreeSet<String>,
}

impl Balance {
    fn add(&mut self, other: &Balance) {
        self.units.add_inventory(&other.units);
        self.value.add_inventory(&other.value);
        self.currencies.extend(other.currencies.iter().cloned());
    }

    /// The response's valued balance: the units per currency, with every currency held and the
    /// operating currency, so a new account gets a row to set its opening balance, and their total
    /// value in the operating currency; what no price converts is left out of the total.
    fn calculated(&self, operating_currency: &str) -> CalculatedAmount {
        self.currencies
            .iter()
            .fold(calculated_amount(&self.units, &self.value, operating_currency), |amount, currency| {
                amount.persist_commodity(currency)
            })
            .persist_commodity(operating_currency)
    }
}

/// The accounts of an `accounts.list`-shaped result and the balances of an `accounts.balances`-shaped
/// one (each with the name of its query), by name: the accounts with a directive and those with
/// postings. Each account's subtree totals add up the rows of the account and of the accounts under
/// it, as the account tree does.
fn summaries(accounts: (&str, &QueryResult), balances: (&str, &QueryResult)) -> ServerResult<BTreeMap<String, Summary>> {
    let mut summaries: BTreeMap<String, Summary> = BTreeMap::new();
    for row in cells::rows(accounts.0, accounts.1) {
        let summary = summaries.entry(row.str("account")?.unwrap_or_default()).or_default();
        summary.open = row.date("open")?;
        summary.close = row.date("close")?;
        summary.status = row.str("status")?;
        summary.alias = row.str("alias")?;
    }
    for row in cells::rows(balances.0, balances.1) {
        let summary = summaries.entry(row.str("account")?.unwrap_or_default()).or_default();
        let currency = row.str("currency")?.unwrap_or_default();
        summary
            .own
            .units
            .add_amount(&Amount::new(row.decimal("units")?.unwrap_or_default(), currency.clone()));
        summary.own.currencies.insert(currency);
        if let Some(value) = row.get("value")?.as_inventory() {
            summary.own.value.add_inventory(value);
        }
        let first = row.date("first_date")?;
        summary.first_posting = match (summary.first_posting, first) {
            (Some(earlier), Some(first)) => Some(earlier.min(first)),
            (earlier, first) => earlier.or(first),
        };
    }
    // the account itself, then each of its ancestors that is an account: `Assets:Bank` and `Assets` for
    // `Assets:Bank:Checking`
    let names = summaries.keys().cloned().collect::<Vec<_>>();
    for name in &names {
        let own = std::mem::take(&mut summaries.get_mut(name).expect("a summary of every name").own);
        let ancestors = name.match_indices(':').map(|(at, _)| &name[..at]);
        for (index, receiver) in std::iter::once(name.as_str()).chain(ancestors).enumerate() {
            let Some(summary) = summaries.get_mut(receiver) else { continue };
            summary.has_sub_accounts |= index > 0;
            summary.subtree.add(&own);
        }
        summaries.get_mut(name).expect("a summary of every name").own = own;
    }
    Ok(summaries)
}

/// The subtree units per currency, as the response's `balance_with_sub_accounts`.
fn with_sub_accounts(summary: &Summary, operating_currency: &str) -> HashMap<String, BigDecimal> {
    summary.subtree.calculated(operating_currency).detail
}

/// `Close` once the account's close took effect; an account not opened yet, or with postings only, is listed as open
fn status(summary: &Summary) -> AccountStatus {
    match summary.status.as_deref() {
        Some("closed") => AccountStatus::Close,
        _ => AccountStatus::Open,
    }
}

/// `GET /api/accounts`: every account with an `open` or `close` directive or with postings, by
/// name.
pub fn account_list(ledger: &Ledger) -> ServerResult<Vec<AccountEntity>> {
    let operating_currency = ledger.options.operating_currency.as_str();
    let accounts = run(ledger, LIST, &builtin::bind_instant(Params::new(), builtin::ledger_now(ledger)))?;
    let balances = run(ledger, BALANCES, &Params::new().bind("operating_currency", operating_currency))?;
    Ok(summaries((LIST, &accounts), (BALANCES, &balances))?
        .into_iter()
        .map(|(name, summary)| AccountEntity {
            status: status(&summary),
            alias: summary.alias.clone(),
            amount: summary.own.calculated(operating_currency),
            balance_with_sub_accounts: with_sub_accounts(&summary, operating_currency),
            amount_with_sub_accounts: summary.subtree.calculated(operating_currency),
            has_sub_accounts: summary.has_sub_accounts,
            name,
        })
        .collect())
}

/// Whether `account` has a page, as an account with an `open` or `close` directive or with
/// postings; a name that is no account name is a 400.
fn has_page(ledger: &Ledger, account: &str) -> ServerResult<bool> {
    crate::validate::account(account, &crate::validate::Rules::Zhang)?;
    let named = |query: &str, result: &QueryResult| -> ServerResult<bool> {
        for row in cells::rows(query, result) {
            if row.str("account")?.as_deref() == Some(account) {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let subtree = builtin::bind_instant(Params::new().bind("account", account), builtin::ledger_now(ledger));
    if named(SUBTREE, &run(ledger, SUBTREE, &subtree)?)? {
        return Ok(true);
    }
    // an account without a directive is one with postings
    let operating_currency = ledger.options.operating_currency.as_str();
    let balances = run(
        ledger,
        SUBTREE_BALANCES,
        &Params::new().bind("account", account).bind("operating_currency", operating_currency),
    )?;
    named(SUBTREE_BALANCES, &balances)
}

/// The page of `account` exists: a 400 for a name that is no account name, a 404 for an account
/// that has no page, as `GET /api/accounts/{a}` answers.
fn require_page(ledger: &Ledger, account: &str) -> ServerResult<()> {
    match has_page(ledger, account)? {
        true => Ok(()),
        false => Err(crate::error::ServerError::NotFound),
    }
}

/// `GET /api/accounts/{a}`: an account with an `open` or `close` directive or with postings;
/// `None` for any other account, and a 400 for a name that is no account name.
pub fn account_info(ledger: &Ledger, account: &str) -> ServerResult<Option<AccountInfoEntity>> {
    crate::validate::account(account, &crate::validate::Rules::Zhang)?;
    let operating_currency = ledger.options.operating_currency.as_str();
    let accounts = run(
        ledger,
        SUBTREE,
        &builtin::bind_instant(Params::new().bind("account", account), builtin::ledger_now(ledger)),
    )?;
    let balances = run(
        ledger,
        SUBTREE_BALANCES,
        &Params::new().bind("account", account).bind("operating_currency", operating_currency),
    )?;
    let mut summaries = summaries((SUBTREE, &accounts), (SUBTREE_BALANCES, &balances))?;
    let Some(summary) = summaries.remove(account) else {
        return Ok(None);
    };
    let r#type = Account::from_str(account).map(|it| it.account_type.to_string()).unwrap_or_default();
    let date = summary.open.or(summary.first_posting).or(summary.close).unwrap_or_default();
    Ok(Some(AccountInfoEntity {
        date: date.and_time(NaiveTime::default()),
        r#type,
        name: account.to_owned(),
        status: status(&summary),
        alias: summary.alias.clone(),
        amount: summary.own.calculated(operating_currency),
        amount_with_sub_accounts: summary.subtree.calculated(operating_currency),
        balance_with_sub_accounts: with_sub_accounts(&summary, operating_currency),
        has_sub_accounts: summary.has_sub_accounts,
    }))
}

// ---------------------------------------------------------------------------------------
// journal

/// A page of an account's journal: `size` rows from row `offset` (both from 0), newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalWindow {
    pub offset: u64,
    pub size: u64,
}

/// A journal: its rows, and with a [`JournalWindow`] the number of rows of the whole journal.
pub struct Journal {
    pub rows: Vec<AccountJournalEntity>,
    pub total: Option<u64>,
}

/// A row of `accounts.journal`: one posting, with its `seq`.
struct PostingRow {
    seq: i64,
    journal: AccountJournalEntity,
}

/// The units of `currency` in an inventory, zero if it has none.
fn units_of(inventory: Option<&Inventory>, currency: &str) -> Amount {
    let number = inventory
        .into_iter()
        .flat_map(|inventory| inventory.positions())
        .filter(|position| position.units.commodity == currency)
        .fold(BigDecimal::zero(), |total, position| total + position.units.number);
    Amount::new(number, currency)
}

/// The rows of `accounts.journal` or `accounts.journal_page` (`query`).
fn posting_rows(query: &str, result: &QueryResult) -> ServerResult<Vec<PostingRow>> {
    cells::rows(query, result)
        .map(|row| {
            let currency = row.str("currency")?.unwrap_or_default();
            Ok(PostingRow {
                seq: row.int("seq")?.unwrap_or_default(),
                journal: AccountJournalEntity {
                    datetime: row.datetime("date", "time")?.unwrap_or_default(),
                    timestamp: row.int("timestamp")?.unwrap_or_default(),
                    account: row.str("account")?.unwrap_or_default(),
                    trx_id: row.str("id")?.unwrap_or_default(),
                    payee: row.str("payee")?,
                    narration: row.str("narration")?,
                    inferred_unit: Amount::new(row.decimal("units")?.unwrap_or_default(), currency.clone()),
                    // the balance of the subtree after the posting, in its currency
                    account_after: units_of(row.get("balance")?.as_inventory(), &currency),
                    asserted: None,
                    checked_balance: None,
                    difference: None,
                    tolerance: None,
                    passed: None,
                },
            })
        })
        .collect()
}

/// The rows of `accounts.balance_assertions`, with their `seq`.
fn assertion_rows(result: &QueryResult) -> ServerResult<Vec<(i64, AccountJournalEntity)>> {
    cells::rows(BALANCE_ASSERTIONS, result)
        .map(|row| {
            let assertion = BalanceAssertion::of(&row)?;
            // zero, written with the decimals of the asserted amount and the balance
            let nothing = BigDecimal::zero().with_scale(assertion.difference.number.fractional_digit_count());
            let journal = AccountJournalEntity {
                datetime: row.datetime("date", "time")?.unwrap_or_default(),
                timestamp: row.int("timestamp")?.unwrap_or_default(),
                account: assertion.account.clone(),
                trx_id: row.str("id")?.unwrap_or_default(),
                payee: Some(BALANCE_CHECK_PAYEE.to_owned()),
                narration: Some(assertion.account),
                inferred_unit: Amount::new(nothing, assertion.asserted.commodity.clone()),
                // the running balance where it stands: the balance it was checked against
                account_after: assertion.checked_balance.clone(),
                asserted: Some(assertion.asserted),
                checked_balance: Some(assertion.checked_balance),
                difference: Some(assertion.difference),
                tolerance: assertion.tolerance,
                passed: Some(assertion.passed),
            };
            Ok((row.int("seq")?.unwrap_or_default(), journal))
        })
        .collect()
}

/// `GET /api/accounts/{a}/journals`: the postings of the account and its sub-accounts, newest
/// first, each with the running balance of the subtree in its currency, and the balance
/// assertions on the account, each with the balance it was checked against. An account without a
/// page is a 404, and a name that is no account name a 400, as for `GET /api/accounts/{a}`.
///
/// The rows are `accounts.journal`, a row per posting, and `accounts.balance_assertions` merged by
/// `seq`, the order zhang processed the ledger in: an assertion stands right after the postings
/// its balance includes, so its balance is the running balance where it stands. A window pages
/// through these rows, a posting and an assertion a row each.
///
/// `accounts.journal` lists the postings in ledger order, which the journal turns around; a page
/// of it is read from the end with `accounts.journal_page`, which only builds the postings it
/// returns, and whose total is the number of postings.
pub fn account_journals(ledger: &Ledger, account: &str, window: Option<JournalWindow>) -> ServerResult<Journal> {
    require_page(ledger, account)?;
    let assertions = assertion_rows(&run(ledger, BALANCE_ASSERTIONS, &Params::new().bind("account", account))?)?;
    let Some(window) = window else {
        let postings = run(ledger, JOURNAL, &Params::new().bind("account", account)).map_err(too_large_unpaged)?;
        let mut postings = posting_rows(JOURNAL, &postings)?;
        postings.reverse();
        let rows = merge(postings, 0, assertions, 0, u64::MAX);
        return Ok(Journal { rows, total: None });
    };
    // the postings `offset..offset + limit` in ledger order, and with `count_total` the number of postings
    let page = |limit: u64, offset: u64, count_total: bool| {
        let params = Params::new()
            .bind("account", account)
            .bind("limit", i64::try_from(limit).unwrap_or(i64::MAX))
            .bind("offset", i64::try_from(offset).unwrap_or(i64::MAX));
        builtin::execute(ledger, JOURNAL_PAGE, &params, count_total)
    };
    let postings_total = page(0, 0, true)?.total.unwrap_or_default();
    let assertion_count = assertions.len() as u64;
    let end = window.offset.saturating_add(window.size);
    // the postings `first..last` from the newest: a posting in the window has at most every assertion newer than it
    let first = window.offset.saturating_sub(assertion_count).min(postings_total);
    let last = end.min(postings_total);
    let mut postings = match last.checked_sub(first) {
        Some(limit) if limit > 0 => posting_rows(JOURNAL_PAGE, &page(limit, postings_total - last, false)?)?,
        _ => vec![],
    };
    postings.reverse();
    let rows = merge(postings, first, assertions, window.offset, end);
    Ok(Journal {
        rows,
        total: Some(postings_total + assertion_count),
    })
}

/// The 400 of a journal too large to return at once: it is paged with `page` and `size`.
fn too_large_unpaged(error: crate::error::ServerError) -> crate::error::ServerError {
    match error {
        crate::error::ServerError::QueryError(error) if error.kind == zhang_query::QueryErrorKind::TooLarge => {
            crate::error::ServerError::InvalidInput(format!(
                "the journal is too large to return at once ({}); ask for it by page with `page` and `size`",
                error.message
            ))
        }
        other => other,
    }
}

/// The number of the assertions newer than `seq` (`assertions` newest first).
fn newer_assertions(assertions: &[(i64, AccountJournalEntity)], seq: i64) -> u64 {
    assertions.partition_point(|(assertion, _)| *assertion > seq) as u64
}

/// The rows `offset..end` of the journal: the postings `first..` of `accounts.journal` from the
/// newest, which run to row `end` or to the oldest, merged with all the assertions by `seq`,
/// newest first.
fn merge(postings: Vec<PostingRow>, first: u64, assertions: Vec<(i64, AccountJournalEntity)>, offset: u64, end: u64) -> Vec<AccountJournalEntity> {
    let in_window = |row: u64| (offset..end).contains(&row);
    // the row of every posting in the whole journal: its index plus the assertions newer than it
    let posting_rows = postings
        .iter()
        .enumerate()
        .map(|(index, posting)| first + index as u64 + newer_assertions(&assertions, posting.seq))
        .collect::<Vec<_>>();
    // the row of every assertion: its index plus the postings newer than it, those fetched and the `first`
    // before them. That counts too many for an assertion newer than every fetched posting, which then comes
    // before the window anyway (`first` leaves a row for each assertion before it), and too few for one older
    // than every fetched posting, which comes after the window, as the postings fetched run to its end
    let assertion_rows = assertions
        .iter()
        .enumerate()
        .map(|(index, (seq, _))| index as u64 + first + postings.partition_point(|posting| posting.seq > *seq) as u64)
        .collect::<Vec<_>>();
    let mut rows = postings
        .into_iter()
        .zip(posting_rows)
        .map(|(posting, row)| (row, posting.journal))
        .chain(assertions.into_iter().zip(assertion_rows).map(|((_, assertion), row)| (row, assertion)))
        .filter(|(row, _)| in_window(*row))
        .collect::<Vec<_>>();
    rows.sort_by_key(|(row, _)| *row);
    rows.into_iter().map(|(_, journal)| journal).collect()
}

// ---------------------------------------------------------------------------------------
// balance history and documents

/// `GET /api/accounts/{a}/balances`: the balance of the account and its sub-accounts at the end
/// of every day it changed, per currency, in date order; a 404 or a 400 as for the journal.
pub fn account_balance_history(ledger: &Ledger, account: &str) -> ServerResult<AccountBalanceHistoryEntity> {
    require_page(ledger, account)?;
    let result = run(ledger, BALANCE_HISTORY, &Params::new().bind("account", account))?;
    let mut balance: HashMap<String, Vec<AccountBalanceItemEntity>> = HashMap::new();
    for row in cells::rows(BALANCE_HISTORY, &result) {
        let currency = row.str("currency")?.unwrap_or_default();
        let item = AccountBalanceItemEntity {
            date: row.date("date")?.unwrap_or_default(),
            balance: row.amount("balance")?.unwrap_or_else(|| Amount::new(BigDecimal::zero(), currency.clone())),
        };
        balance.entry(currency).or_default().push(item);
    }
    Ok(AccountBalanceHistoryEntity { balance })
}

/// `GET /api/accounts/{a}/documents`: the document directives of the account and its
/// sub-accounts, in ledger order; a 404 or a 400 as for the journal.
pub fn account_documents(ledger: &Ledger, account: &str) -> ServerResult<Vec<DocumentEntity>> {
    require_page(ledger, account)?;
    let result = run(ledger, DOCUMENTS, &Params::new().bind("account", account))?;
    cells::rows(DOCUMENTS, &result).map(|row| DocumentEntity::of(&row)).collect()
}
