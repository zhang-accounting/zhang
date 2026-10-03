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

/// A posting row of an account's journal, with what places the assertions among the rows.
struct PostingRow {
    day: NaiveDate,
    /// whether it posts a padding (flag `P`), which is booked with the balance entries of its day
    padding: bool,
    journal: AccountJournalDomain,
}

/// An assertion row of an account's journal.
struct AssertionRow {
    day: NaiveDate,
    /// whether it is a `balance ... with pad`, which is checked after the other balance entries of its day
    pads: bool,
    journal: AccountJournalDomain,
}

/// `GET /api/accounts/{a}/journals`: the postings of the account and its sub-accounts, newest
/// first, each with the running balance of the subtree in its currency, and the balance
/// assertions on the account, each with the balance it was checked against, which is the
/// running balance of the subtree where it stands.
///
/// An assertion is checked at the start of its day, among the paddings of the day: a balance
/// before the paddings that change it, a `balance ... with pad` after them. Its row stands where
/// the running balance is the balance it was checked against.
pub fn account_journals(ledger: &Ledger, account: &str) -> ServerResult<Vec<AccountJournalDomain>> {
    let params = Params::new().bind("account", account);
    let postings = run(ledger, JOURNAL, &params)?;
    let assertions = run(ledger, BALANCE_ASSERTIONS, &params)?;

    // both oldest first
    let columns = Columns::of(&postings);
    let rows = postings
        .rows
        .iter()
        .rev()
        .map(|row| {
            let currency = text(columns.get(row, "currency")).unwrap_or_default();
            PostingRow {
                day: date(columns.get(row, "date")).unwrap_or_default(),
                padding: columns.get(row, "flag").as_str() == Some("P"),
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
        .collect::<Vec<_>>();
    let columns = Columns::of(&assertions);
    let checks = assertions.rows.iter().rev().map(|row| {
        let asserted = amount(columns.get(row, "amount")).expect("an assertion asserts an amount");
        let actual = amount(columns.get(row, "actual")).unwrap_or_else(|| Amount::new(BigDecimal::zero(), asserted.commodity.clone()));
        // zero, written with the decimals of the asserted amount and the balance
        let nothing = BigDecimal::zero().with_scale((&asserted.number - &actual.number).fractional_digit_count());
        AssertionRow {
            day: date(columns.get(row, "date")).unwrap_or_default(),
            pads: !columns.get(row, "pad").is_null(),
            journal: AccountJournalDomain {
                datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
                timestamp: columns.get(row, "timestamp").as_int().unwrap_or_default(),
                account: text(columns.get(row, "account")).unwrap_or_default(),
                trx_id: text(columns.get(row, "id")).unwrap_or_default(),
                payee: Some(BALANCE_CHECK_PAYEE.to_owned()),
                narration: text(columns.get(row, "account")),
                inferred_unit: Amount::new(nothing, asserted.commodity.clone()),
                account_after: actual.clone(),
                asserted: Some(asserted),
                checked_balance: Some(actual),
                passed: columns.get(row, "passed").as_bool(),
            },
        }
    });

    // the running balance in `currency` before the row `at`
    let balance_before = |at: usize, currency: &str| {
        rows[..at]
            .iter()
            .rev()
            .find(|row| row.journal.account_after.commodity == currency)
            .map_or_else(BigDecimal::zero, |row| row.journal.account_after.number.clone())
    };
    // each assertion with the row it stands before: among the paddings of its day, where the running
    // balance is the balance it was checked against.
    // TODO(#479): once pads are standalone and their paddings follow them (the pad compat branch), merge the
    // postings and `#balances` by `seq` alone, with `seq` following zhang's processing order, including where a
    // `balance ... with pad` is checked, and drop this placement.
    let mut placed = checks
        .enumerate()
        .map(|(order, check)| {
            let start = rows.partition_point(|row| row.day < check.day);
            let day_end = start + rows[start..].partition_point(|row| row.day == check.day);
            let end = rows[start..day_end].iter().rposition(|row| row.padding).map_or(start, |at| start + at + 1);
            let checked = check.journal.account_after.clone();
            let stands = |at: &usize| balance_before(*at, &checked.commodity) == checked.number;
            let at = if check.pads {
                (start..=end).rev().find(stands).unwrap_or(end)
            } else {
                (start..=end).find(stands).unwrap_or(start)
            };
            (at, check.pads, order, check.journal)
        })
        .collect::<Vec<_>>();
    placed.sort_by_key(|(at, pads, order, _)| (*at, *pads, *order));

    let mut placed = placed.into_iter().peekable();
    let mut journal = Vec::with_capacity(rows.len() + placed.len());
    for (at, row) in rows.into_iter().enumerate() {
        while let Some((_, _, _, assertion)) = placed.next_if(|(stands, ..)| *stands == at) {
            journal.push(assertion);
        }
        journal.push(row.journal);
    }
    journal.extend(placed.map(|(_, _, _, assertion)| assertion));
    // newest first
    journal.reverse();
    Ok(journal)
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
