//! The account endpoints on the query engine (#479): every figure of `GET /api/accounts`,
//! `/api/accounts/{a}`, `/api/accounts/{a}/journals`, `/api/accounts/{a}/balances` and
//! `/api/accounts/{a}/documents` comes from a named, documented built-in query, and this module
//! only maps the query results into the existing response shapes.
//!
//! A parent account's page is its subtree (decision 6 of #479): its journal, balance history,
//! header total and documents cover the account and all its sub-accounts, as the account tree
//! does. Balances are valued in the operating currency at today's prices with `convert`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::Account;
use zhang_core::constants::BALANCE_CHECK_PAYEE;
use zhang_core::domains::schemas::{AccountJournalDomain, AccountStatus};
use zhang_core::ledger::Ledger;
use zhang_query::{Inventory, Params, QueryResult, Value};

use crate::builtin::{self, calculated_amount};
use crate::response::{AccountBalanceHistoryEntity, AccountBalanceItemEntity, AccountEntity, AccountInfoEntity, DocumentEntity};
use crate::state::SharedLedger;
use crate::ServerResult;

/// The built-in queries of the account endpoints, in [`crate::builtin::BUILTINS`].
const LIST: &str = "accounts.list";
const BALANCES: &str = "accounts.balances";
const SUBTREE: &str = "accounts.subtree";
const SUBTREE_BALANCES: &str = "accounts.subtree_balances";
const JOURNAL: &str = "accounts.journal";
const JOURNAL_ROWS: &str = "accounts.journal_rows";
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
// cells

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn date(value: &Value) -> Option<NaiveDate> {
    value.as_date()
}

fn decimal(value: &Value) -> BigDecimal {
    value.as_decimal().unwrap_or_else(BigDecimal::zero)
}

fn amount(value: &Value) -> Option<Amount> {
    value.as_amount().cloned()
}

/// The date and time of a row, from its `date` and `time` (`HH:MM:SS`) cells.
fn datetime(date_cell: &Value, time_cell: &Value) -> NaiveDateTime {
    let date = date(date_cell).unwrap_or_default();
    let time = time_cell
        .as_str()
        .and_then(|it| NaiveTime::parse_from_str(it, "%H:%M:%S").ok())
        .unwrap_or_default();
    date.and_time(time)
}

/// The column indexes of a result, by name.
struct Columns(HashMap<String, usize>);

impl Columns {
    fn of(result: &QueryResult) -> Columns {
        Columns(result.columns.iter().enumerate().map(|(idx, column)| (column.name.clone(), idx)).collect())
    }

    fn get<'r>(&self, row: &'r [Value], name: &str) -> &'r Value {
        &row[*self.0.get(name).unwrap_or_else(|| panic!("column {name}"))]
    }
}

// ---------------------------------------------------------------------------------------
// account list and info

/// What the list and the account page show of an account: its directives, its own balance and
/// that of its subtree.
#[derive(Default)]
struct Summary {
    open: Option<NaiveDate>,
    close: Option<NaiveDate>,
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
/// one, by name: the accounts with a directive and those with postings. Each account's subtree
/// totals add up the rows of the account and of the accounts under it, as the account tree does.
fn summaries(accounts: &QueryResult, balances: &QueryResult) -> BTreeMap<String, Summary> {
    let mut summaries: BTreeMap<String, Summary> = BTreeMap::new();
    let columns = Columns::of(accounts);
    for row in &accounts.rows {
        let summary = summaries.entry(text(columns.get(row, "account")).unwrap_or_default()).or_default();
        summary.open = date(columns.get(row, "open"));
        summary.close = date(columns.get(row, "close"));
        summary.alias = text(columns.get(row, "alias"));
    }
    let columns = Columns::of(balances);
    for row in &balances.rows {
        let summary = summaries.entry(text(columns.get(row, "account")).unwrap_or_default()).or_default();
        let currency = text(columns.get(row, "currency")).unwrap_or_default();
        summary.own.units.add_amount(&Amount::new(decimal(columns.get(row, "units")), currency.clone()));
        summary.own.currencies.insert(currency);
        if let Some(value) = columns.get(row, "value").as_inventory() {
            summary.own.value.add_inventory(value);
        }
        let first = date(columns.get(row, "first_date"));
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
    summaries
}

/// The subtree units per currency, as the response's `balance_with_sub_accounts`.
fn with_sub_accounts(summary: &Summary, operating_currency: &str) -> HashMap<String, BigDecimal> {
    summary.subtree.calculated(operating_currency).detail
}

fn status(summary: &Summary) -> AccountStatus {
    if summary.close.is_some() {
        AccountStatus::Close
    } else {
        AccountStatus::Open
    }
}

/// `GET /api/accounts`: every account with an `open` or `close` directive or with postings, by
/// name.
pub fn account_list(ledger: &Ledger) -> ServerResult<Vec<AccountEntity>> {
    let operating_currency = ledger.options.operating_currency.as_str();
    let accounts = run(ledger, LIST, &Params::new())?;
    let balances = run(ledger, BALANCES, &Params::new().bind("operating_currency", operating_currency))?;
    Ok(summaries(&accounts, &balances)
        .into_iter()
        .map(|(name, summary)| AccountEntity {
            status: status(&summary),
            alias: summary.alias.clone(),
            amount: summary.own.calculated(operating_currency),
            balance_with_sub_accounts: with_sub_accounts(&summary, operating_currency),
            has_sub_accounts: summary.has_sub_accounts,
            name,
        })
        .collect())
}

/// `GET /api/accounts/{a}`: an account with an `open` or `close` directive or with postings;
/// `None` for any other name.
pub fn account_info(ledger: &Ledger, account: &str) -> ServerResult<Option<AccountInfoEntity>> {
    let operating_currency = ledger.options.operating_currency.as_str();
    let accounts = run(ledger, SUBTREE, &Params::new().bind("account", account))?;
    let balances = run(
        ledger,
        SUBTREE_BALANCES,
        &Params::new().bind("account", account).bind("operating_currency", operating_currency),
    )?;
    let mut summaries = summaries(&accounts, &balances);
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
    pub rows: Vec<AccountJournalDomain>,
    pub total: Option<u64>,
}

/// A row of `accounts.journal`: one posting, or the part of a posting booked against one lot.
struct PostingRow {
    seq: i64,
    posting_index: i64,
    journal: AccountJournalDomain,
}

impl PostingRow {
    /// whether `other` is another part of the same posting
    fn same_posting(&self, other: &PostingRow) -> bool {
        (self.seq, self.posting_index) == (other.seq, other.posting_index)
    }
}

fn posting_rows(result: &QueryResult) -> Vec<PostingRow> {
    let columns = Columns::of(result);
    result
        .rows
        .iter()
        .map(|row| {
            let currency = text(columns.get(row, "currency")).unwrap_or_default();
            PostingRow {
                seq: columns.get(row, "seq").as_int().unwrap_or_default(),
                posting_index: columns.get(row, "posting_index").as_int().unwrap_or_default(),
                journal: AccountJournalDomain {
                    datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
                    timestamp: columns.get(row, "timestamp").as_int().unwrap_or_default(),
                    account: text(columns.get(row, "account")).unwrap_or_default(),
                    trx_id: text(columns.get(row, "id")).unwrap_or_default(),
                    payee: text(columns.get(row, "payee")),
                    narration: text(columns.get(row, "narration")),
                    inferred_unit: Amount::new(decimal(columns.get(row, "units")), currency.clone()),
                    account_after: amount(columns.get(row, "balance")).unwrap_or_else(|| Amount::new(BigDecimal::zero(), currency)),
                    asserted: None,
                    checked_balance: None,
                    passed: None,
                },
            }
        })
        .collect()
}

/// The rows of `accounts.balance_assertions`, with their `seq`.
fn assertion_rows(result: &QueryResult) -> Vec<(i64, AccountJournalDomain)> {
    let columns = Columns::of(result);
    result
        .rows
        .iter()
        .map(|row| {
            let asserted = amount(columns.get(row, "amount")).expect("an assertion asserts an amount");
            let actual = amount(columns.get(row, "actual")).unwrap_or_else(|| Amount::new(BigDecimal::zero(), asserted.commodity.clone()));
            // zero, written with the decimals of the asserted amount and the balance
            let nothing = BigDecimal::zero().with_scale((&asserted.number - &actual.number).fractional_digit_count());
            let journal = AccountJournalDomain {
                datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
                timestamp: columns.get(row, "timestamp").as_int().unwrap_or_default(),
                account: text(columns.get(row, "account")).unwrap_or_default(),
                trx_id: text(columns.get(row, "id")).unwrap_or_default(),
                payee: Some(BALANCE_CHECK_PAYEE.to_owned()),
                narration: text(columns.get(row, "account")),
                inferred_unit: Amount::new(nothing, asserted.commodity.clone()),
                // the running balance where it stands: the balance it was checked against
                account_after: actual.clone(),
                asserted: Some(asserted),
                checked_balance: Some(actual),
                passed: columns.get(row, "passed").as_bool(),
            };
            (columns.get(row, "seq").as_int().unwrap_or_default(), journal)
        })
        .collect()
}

/// `GET /api/accounts/{a}/journals`: the postings of the account and its sub-accounts, newest
/// first, each with the running balance of the subtree in its currency, and the balance
/// assertions on the account, each with the balance it was checked against.
///
/// The rows are `accounts.journal` and `accounts.balance_assertions` merged by `seq`, the order
/// zhang processed the ledger in: an assertion stands right after the postings its balance
/// includes, so its balance is the running balance where it stands. The rows of a posting booked
/// against several lots are one row. A window pages through the rows of the two queries (a lot
/// row and an assertion are a row each); a posting belongs to the page of its first row.
///
/// `accounts.journal` lists the rows in ledger order, which the journal turns around; a page of it
/// is read from the end with `accounts.journal_page`, which only builds the rows it returns.
pub fn account_journals(ledger: &Ledger, account: &str, window: Option<JournalWindow>) -> ServerResult<Journal> {
    let assertions = assertion_rows(&run(ledger, BALANCE_ASSERTIONS, &Params::new().bind("account", account))?);
    let Some(window) = window else {
        let postings = run(ledger, JOURNAL, &Params::new().bind("account", account)).map_err(too_large_unpaged)?;
        let mut postings = posting_rows(&postings);
        postings.reverse();
        let rows = merge(postings, 0, assertions, 0, u64::MAX);
        return Ok(Journal { rows, total: None });
    };
    let count = run(ledger, JOURNAL_ROWS, &Params::new().bind("account", account))?;
    let total = count.rows.first().and_then(|row| row[0].as_int()).map_or(0, |it| it as u64);
    let assertion_count = assertions.len() as u64;
    // a posting in the window has at most every assertion before it; one more row tells whether the first
    // posting continues one of the previous page
    let first = window.offset.saturating_sub(assertion_count).saturating_sub(1).min(total);
    let end = window.offset.saturating_add(window.size);
    // the rows after the window that complete its last posting
    let mut extra = 16u64;
    loop {
        // rows `first..first + limit` from the newest, `total - first - limit..total - first` in ledger order
        let limit = end.saturating_sub(first).saturating_add(extra).min(total - first);
        let params = Params::new()
            .bind("account", account)
            .bind("limit", i64::try_from(limit).unwrap_or(i64::MAX))
            .bind("offset", i64::try_from(total - first - limit).unwrap_or(i64::MAX));
        let mut postings = posting_rows(&run(ledger, JOURNAL_PAGE, &params)?);
        postings.reverse();
        let complete = first + postings.len() as u64 >= total;
        let open_end = !complete && last_posting_starts_before(&postings, first, &assertions, end);
        if open_end {
            extra = extra.saturating_mul(4);
            continue;
        }
        let rows = merge(postings, first, assertions, window.offset, end);
        return Ok(Journal {
            rows,
            total: Some(total + assertion_count),
        });
    }
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
fn newer_assertions(assertions: &[(i64, AccountJournalDomain)], seq: i64) -> u64 {
    assertions.partition_point(|(assertion, _)| *assertion > seq) as u64
}

/// Whether the posting of the last of `postings` (rows `first..` of `accounts.journal`) starts
/// before row `end`, so the rows after `postings` may still belong to it.
fn last_posting_starts_before(postings: &[PostingRow], first: u64, assertions: &[(i64, AccountJournalDomain)], end: u64) -> bool {
    let Some(last) = postings.last() else { return false };
    let start = postings.iter().rposition(|row| !row.same_posting(last)).map_or(0, |at| at + 1);
    first + start as u64 + newer_assertions(assertions, postings[start].seq) < end
}

/// The rows `offset..end` of the journal: the posting rows `first..` of `accounts.journal` from
/// the newest, which run past `end` or to the oldest, merged with all the assertions by `seq`,
/// newest first, the rows of one posting as one row.
fn merge(postings: Vec<PostingRow>, first: u64, assertions: Vec<(i64, AccountJournalDomain)>, offset: u64, end: u64) -> Vec<AccountJournalDomain> {
    let in_window = |row: u64| (offset..end).contains(&row);
    // the row of every posting row in the whole journal: its index plus the assertions newer than it
    let mut merged: Vec<(u64, Option<AccountJournalDomain>)> = vec![];
    let mut postings_of: Vec<(u64, PostingRow)> = Vec::with_capacity(postings.len());
    for (index, posting) in postings.into_iter().enumerate() {
        let row = first + index as u64 + newer_assertions(&assertions, posting.seq);
        postings_of.push((row, posting));
    }
    for (index, (seq, assertion)) in assertions.into_iter().enumerate() {
        // the postings newer than the assertion: those of the rows fetched, plus the `first` before them. That
        // counts too many for an assertion newer than every fetched row, which then comes before the window
        // anyway (`first` leaves a row for each assertion and one more before it), and too few for one older
        // than every fetched row, which comes after the window, as long as the rows fetched run past it
        let newer = postings_of.partition_point(|(_, posting)| posting.seq > seq) as u64;
        let row = index as u64 + first + newer;
        if in_window(row) {
            merged.push((row, Some(assertion)));
        }
    }
    // a posting's rows are consecutive; it belongs to the window of its first row
    let mut rows: Vec<(u64, AccountJournalDomain)> = vec![];
    let mut previous: Option<&PostingRow> = None;
    let mut taking = false;
    for (row, posting) in &postings_of {
        let continues = previous.is_some_and(|previous| previous.same_posting(posting));
        if continues {
            if taking {
                // an earlier lot of the posting, newest first: it adds its units; the balance after the posting is
                // that after its last lot, the first row
                let (_, journal) = rows.last_mut().expect("the posting's first row is taken");
                journal.inferred_unit.number += &posting.journal.inferred_unit.number;
            }
        } else {
            taking = in_window(*row);
            if taking {
                rows.push((*row, posting.journal.clone()));
            }
        }
        previous = Some(posting);
    }
    merged.extend(rows.into_iter().map(|(row, journal)| (row, Some(journal))));
    merged.sort_by_key(|(row, _)| *row);
    merged.into_iter().filter_map(|(_, journal)| journal).collect()
}

// ---------------------------------------------------------------------------------------
// balance history and documents

/// `GET /api/accounts/{a}/balances`: the balance of the account and its sub-accounts at the end
/// of every day it changed, per currency, in date order.
pub fn account_balance_history(ledger: &Ledger, account: &str) -> ServerResult<AccountBalanceHistoryEntity> {
    let result = run(ledger, BALANCE_HISTORY, &Params::new().bind("account", account))?;
    let columns = Columns::of(&result);
    let mut balance: HashMap<String, Vec<AccountBalanceItemEntity>> = HashMap::new();
    for row in &result.rows {
        let currency = text(columns.get(row, "currency")).unwrap_or_default();
        let item = AccountBalanceItemEntity {
            date: date(columns.get(row, "date")).unwrap_or_default(),
            balance: amount(columns.get(row, "balance")).unwrap_or_else(|| Amount::new(BigDecimal::zero(), currency.clone())),
        };
        balance.entry(currency).or_default().push(item);
    }
    Ok(AccountBalanceHistoryEntity { balance })
}

/// `GET /api/accounts/{a}/documents`: the document directives of the account and its
/// sub-accounts, in ledger order.
pub fn account_documents(ledger: &Ledger, account: &str) -> ServerResult<Vec<DocumentEntity>> {
    let result = run(ledger, DOCUMENTS, &Params::new().bind("account", account))?;
    let columns = Columns::of(&result);
    Ok(result
        .rows
        .iter()
        .map(|row| {
            let path = text(columns.get(row, "path")).unwrap_or_default();
            DocumentEntity {
                datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
                filename: Path::new(&path).file_name().map(|it| it.to_string_lossy().into_owned()).unwrap_or_default(),
                path,
                extension: None,
                account: text(columns.get(row, "account")),
                trx_id: None,
            }
        })
        .collect())
}
