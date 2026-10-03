//! The account endpoints on the query engine (#479): every figure of `GET /api/accounts`,
//! `/api/accounts/{a}`, `/api/accounts/{a}/journals`, `/api/accounts/{a}/balances` and
//! `/api/accounts/{a}/documents` comes from a named, documented built-in query, and this module
//! only maps the query results into the existing response shapes.
//!
//! A parent account's page is its subtree (decision 6 of #479): its journal, balance history,
//! header total and documents cover the account and all its sub-accounts, as the account tree
//! does. Balances are valued in the operating currency at today's prices with `convert`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::Account;
use zhang_core::constants::BALANCE_CHECK_PAYEE;
use zhang_core::domains::schemas::{AccountJournalDomain, AccountStatus};
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Inventory, ParamTypes, Params, Query, QueryResult, Value};

use crate::response::{AccountBalanceHistoryEntity, AccountBalanceItemEntity, AccountEntity, AccountInfoEntity, DocumentEntity};
use crate::routes::query::{execute_options, max_result_values};
use crate::state::SharedLedger;
use crate::ServerResult;

/// A named query behind an endpoint: the BQL it runs and the types of its parameters. User
/// input only ever reaches a query as a bound parameter.
pub struct BuiltinQuery {
    pub name: &'static str,
    pub description: &'static str,
    pub bql: &'static str,
    pub params: &'static [(&'static str, DataType)],
}

/// Every account with an `open` or `close` directive, by name.
pub const ACCOUNTS: BuiltinQuery = BuiltinQuery {
    name: "accounts",
    description: "Every account with an open or close directive, with its open and close dates and its alias, by name.",
    bql: "SELECT account, open, close, meta('alias') AS alias
FROM #accounts
ORDER BY account",
    params: &[],
};

/// The balance of every account, of its own postings, per currency.
pub const ACCOUNT_BALANCES: BuiltinQuery = BuiltinQuery {
    name: "account_balances",
    description: "The balance of every account that has postings, of its own postings, per currency: the units, their value \
                  in the operating currency at today's prices, and the date of the first posting.",
    bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency",
    params: &[("operating_currency", DataType::Str)],
};

/// The accounts of [`ACCOUNTS`] in the subtree of an account.
pub const SUBTREE_ACCOUNTS: BuiltinQuery = BuiltinQuery {
    name: "account_subtree",
    description: "An account and its sub-accounts that have an open or close directive, with their open and close dates and \
                  their aliases, by name.",
    bql: "SELECT account, open, close, meta('alias') AS alias
FROM #accounts
WHERE under(account, :account)
ORDER BY account",
    params: &[("account", DataType::Str)],
};

/// [`ACCOUNT_BALANCES`] of the accounts in the subtree of an account.
pub const SUBTREE_BALANCES: BuiltinQuery = BuiltinQuery {
    name: "account_subtree_balances",
    description: "The balance of an account and of each of its sub-accounts, of its own postings, per currency: the units, \
                  their value in the operating currency at today's prices, and the date of the first posting.",
    bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency",
    params: &[("account", DataType::Str), ("operating_currency", DataType::Str)],
};

/// The postings of an account and its sub-accounts, newest first, with their running balance.
pub const ACCOUNT_JOURNAL: BuiltinQuery = BuiltinQuery {
    name: "account_journal",
    description: "The postings of an account and its sub-accounts, newest first, one row per posting, each with the running \
                  balance of the account and its sub-accounts in the posting's currency right after it.",
    bql: "SELECT date, time, timestamp, flag, id, account, payee, narration, currency,
       sum(number) AS units,
       last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index, date, time, timestamp, flag, id, account, payee, narration, currency
ORDER BY seq DESC, posting_index DESC",
    params: &[("account", DataType::Str)],
};

/// The balance assertions on an account, newest first.
pub const ACCOUNT_BALANCE_ASSERTIONS: BuiltinQuery = BuiltinQuery {
    name: "account_balance_assertions",
    description: "The balance assertions on an account, newest first: the asserted amount, the balance of the account and \
                  its sub-accounts it was checked against, whether it held, and the account a balance with pad pads from.",
    bql: "SELECT date, time, timestamp, id, account, amount, actual, passed, pad
FROM #balances
WHERE account = :account
ORDER BY seq DESC",
    params: &[("account", DataType::Str)],
};

/// The balance of an account and its sub-accounts at the end of every day it changed.
pub const ACCOUNT_BALANCE_HISTORY: BuiltinQuery = BuiltinQuery {
    name: "account_balance_history",
    description: "The balance of an account and its sub-accounts at the end of every day with a posting, per currency, in \
                  date order.",
    bql: "SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency",
    params: &[("account", DataType::Str)],
};

/// The document directives of an account and its sub-accounts.
pub const ACCOUNT_DOCUMENTS: BuiltinQuery = BuiltinQuery {
    name: "account_documents",
    description: "The document directives of an account and its sub-accounts, in ledger order, with the path of each file \
                  relative to the ledger's directory.",
    bql: "SELECT date, time, account, path
FROM #documents
WHERE source = 'directive' AND under(account, :account)",
    params: &[("account", DataType::Str)],
};

/// The built-in queries of the account endpoints.
pub const ACCOUNT_BUILTINS: &[&BuiltinQuery] = &[
    &ACCOUNTS,
    &ACCOUNT_BALANCES,
    &SUBTREE_ACCOUNTS,
    &SUBTREE_BALANCES,
    &ACCOUNT_JOURNAL,
    &ACCOUNT_BALANCE_ASSERTIONS,
    &ACCOUNT_BALANCE_HISTORY,
    &ACCOUNT_DOCUMENTS,
];

/// Every built-in, compiled once.
static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    ACCOUNT_BUILTINS
        .iter()
        .map(|builtin| {
            let types = builtin
                .params
                .iter()
                .fold(ParamTypes::new(), |types, (name, data_type)| types.bind(*name, *data_type));
            let query = Query::compile_with_params(builtin.bql, &types).unwrap_or_else(|error| panic!("built-in query {}: {error}", builtin.name));
            (builtin.name, query)
        })
        .collect()
});

/// Run a built-in query with the limits of every query the server runs.
fn run(ledger: &Ledger, builtin: &BuiltinQuery, params: &Params) -> ServerResult<QueryResult> {
    let query = COMPILED.get(builtin.name).expect("every built-in is compiled");
    Ok(query.execute_with_options(ledger, params, &execute_options(max_result_values()))?)
}

/// Run `f` on the ledger off the async workers, under one read lock, so the queries of one
/// response see the same ledger.
pub async fn with_ledger<T: Send + 'static>(ledger: &SharedLedger, f: impl FnOnce(&Ledger) -> ServerResult<T> + Send + 'static) -> ServerResult<T> {
    let ledger = ledger.0.clone().read_owned().await;
    tokio::task::spawn_blocking(move || f(&ledger)).await?
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
    /// the units of its own postings, per currency; a currency back at zero is kept
    units: BTreeMap<String, BigDecimal>,
    /// the value of its own postings at today's prices
    value: Inventory,
    /// the units of the postings of the account and all its sub-accounts, per currency
    subtree_units: BTreeMap<String, BigDecimal>,
    /// the value of the postings of the account and all its sub-accounts at today's prices
    subtree_value: Inventory,
    has_sub_accounts: bool,
}

/// The accounts of an [`ACCOUNTS`]-shaped result and the balances of an [`ACCOUNT_BALANCES`]-shaped
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
        summary.units.insert(currency, decimal(columns.get(row, "units")));
        if let Some(value) = columns.get(row, "value").as_inventory() {
            summary.value.add_inventory(value);
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
        let (units, value) = {
            let summary = &summaries[name];
            (summary.units.clone(), summary.value.clone())
        };
        let ancestors = name.match_indices(':').map(|(at, _)| &name[..at]);
        for (index, receiver) in std::iter::once(name.as_str()).chain(ancestors).enumerate() {
            let Some(summary) = summaries.get_mut(receiver) else { continue };
            summary.has_sub_accounts |= index > 0;
            for (currency, number) in &units {
                *summary.subtree_units.entry(currency.clone()).or_insert_with(BigDecimal::zero) += number;
            }
            summary.subtree_value.add_inventory(&value);
        }
    }
    summaries
}

/// The response's valued balance: the units per currency, always with the operating currency,
/// and their total value in the operating currency. What no price converts is left out of the
/// total.
fn calculated_amount(units: &BTreeMap<String, BigDecimal>, value: &Inventory, operating_currency: &str) -> CalculatedAmount {
    let calculated = value
        .positions()
        .filter(|position| position.units.commodity == operating_currency)
        .fold(BigDecimal::zero(), |total, position| total + position.units.number);
    let mut detail: HashMap<String, BigDecimal> = units.iter().map(|(currency, number)| (currency.clone(), number.clone())).collect();
    detail.entry(operating_currency.to_owned()).or_default();
    CalculatedAmount {
        calculated: Amount::new(calculated, operating_currency.to_owned()),
        detail,
    }
}

/// The subtree units, always with the operating currency, so a new account gets a row to set its
/// opening balance.
fn with_sub_accounts(summary: &Summary, operating_currency: &str) -> HashMap<String, BigDecimal> {
    let mut balance: HashMap<String, BigDecimal> = summary
        .subtree_units
        .iter()
        .map(|(currency, number)| (currency.clone(), number.clone()))
        .collect();
    balance.entry(operating_currency.to_owned()).or_default();
    balance
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
    let accounts = run(ledger, &ACCOUNTS, &Params::new())?;
    let balances = run(ledger, &ACCOUNT_BALANCES, &Params::new().bind("operating_currency", operating_currency))?;
    Ok(summaries(&accounts, &balances)
        .into_iter()
        .map(|(name, summary)| AccountEntity {
            status: status(&summary),
            alias: summary.alias.clone(),
            amount: calculated_amount(&summary.units, &summary.value, operating_currency),
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
    let accounts = run(ledger, &SUBTREE_ACCOUNTS, &Params::new().bind("account", account))?;
    let balances = run(
        ledger,
        &SUBTREE_BALANCES,
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
        amount: calculated_amount(&summary.units, &summary.value, operating_currency),
        amount_with_sub_accounts: calculated_amount(&summary.subtree_units, &summary.subtree_value, operating_currency),
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
    let postings = run(ledger, &ACCOUNT_JOURNAL, &params)?;
    let assertions = run(ledger, &ACCOUNT_BALANCE_ASSERTIONS, &params)?;

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
    // balance is the balance it was checked against
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
    let result = run(ledger, &ACCOUNT_BALANCE_HISTORY, &Params::new().bind("account", account))?;
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
    let result = run(ledger, &ACCOUNT_DOCUMENTS, &Params::new().bind("account", account))?;
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::{ACCOUNT_BUILTINS, COMPILED};

    /// The `:name` parameters a query's text uses.
    fn parameters(bql: &str) -> BTreeSet<&str> {
        let mut found = BTreeSet::new();
        for (at, _) in bql.match_indices(':') {
            let name = &bql[at + 1..];
            let end = name.find(|it: char| !(it.is_ascii_alphanumeric() || it == '_')).unwrap_or(name.len());
            if end > 0 {
                found.insert(&name[..end]);
            }
        }
        found
    }

    #[test]
    fn every_builtin_compiles_with_the_parameters_it_declares() {
        assert_eq!(COMPILED.len(), ACCOUNT_BUILTINS.len(), "built-in names are unique");
        for builtin in ACCOUNT_BUILTINS {
            let declared = builtin.params.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
            assert_eq!(parameters(builtin.bql), declared, "{}", builtin.name);
            assert!(!builtin.description.is_empty(), "{}", builtin.name);
        }
    }

    #[test]
    fn every_builtin_is_documented_with_its_bql_in_both_languages() {
        let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/src/content/docs");
        for file in ["user-guide/built-in-queries.md", "zh-cn/user-guide/built-in-queries.md"] {
            let text = std::fs::read_to_string(docs.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"));
            for builtin in ACCOUNT_BUILTINS {
                assert!(text.contains(&format!("### {}\n", builtin.name)), "{file} lacks a section for {}", builtin.name);
                assert!(
                    text.contains(&format!("```sql\n{}\n```", builtin.bql)),
                    "{file} lacks the BQL of {}",
                    builtin.name
                );
            }
        }
    }
}
