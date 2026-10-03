//! The budget tables: `#budgets`, one row per budget per month with the figures the budget
//! pages of the web UI show for that month, and `#budget_events`, one row per effect of a budget
//! directive.
//!
//! zhang's budgets are envelopes: `budget-add` and `budget-transfer` put money in, postings to
//! the budget's accounts (those with a `budget: <name>` metadata entry) spend it, and what is
//! left carries over to the next month. A user asks a budgets table the questions the budget
//! page answers one month at a time, across months and budgets at once: how much is left in
//! each budget now, which budgets ran over in which months, how much was spent per category
//! this year, how much was put into a budget over a year. The columns follow from that.
//!
//! - **The UI's terms and figures.** `assigned`, `activity` and `available` are what the budget
//!   page shows: `assigned` is what the month starts with (the available amount carried over
//!   from the previous month) plus what was added in the month, `activity` is what the
//!   budget's accounts spent in the month, and `available = assigned - activity` is what
//!   carries over. All three are amounts in the budget's commodity.
//! - **Computed from the ledger.** The figures are computed here from the budget directives
//!   and the booked postings of the ledger's cache ([`LedgerCache`]), in the order the store
//!   folded them (the order of the ledger's directives), with the rules zhang applies while
//!   loading: a budget exists from its first `budget` directive on
//!   (a second one of the same name is a duplicate zhang reports), a `budget-add`,
//!   `budget-transfer` or `budget-close` naming a budget that does not exist yet is an error
//!   without effect, and only the postings folded after the budget's definition are its
//!   activity.
//! - **In the budget's commodity.** `activity` adds up the booked postings of the budget's
//!   accounts (the rows of the postings table), each converted to the budget's commodity at
//!   the posting's date as `convert(position, commodity, date)` does, with the prices of the
//!   ledger. An amount of a `budget-add` or `budget-transfer` in another commodity is
//!   converted the same way at the directive's date. A posting or amount that no price
//!   converts is left out rather than added as a number of another commodity. Postings to an
//!   Income, Liabilities or Equity account count negated, as zhang counts them.
//! - **`added`, for sums over months.** Because `assigned` includes the carry-over, adding it
//!   up over months counts the same money several times. `added` is only what the month's
//!   `budget-add` and `budget-transfer` directives put in (a transfer out counts as negative),
//!   so `sum(added)` is how much was budgeted over a period, next to `sum(activity)`, what was
//!   spent. `assigned - added` is the carry-over.
//! - **Every month, not only months with entries.** The UI shows a budget for every month
//!   from its first one, carrying the available amount over months without entries, so the
//!   table does too: a query for one month lists every budget the UI lists, and a budget's rows
//!   form a gap-free monthly series. The series runs from the month of the budget's
//!   definition through the later of the budget's own last month (its last `budget-add`,
//!   `budget-transfer` or `budget-close`) and the ledger's last month with a transaction (the
//!   last month the budget can have spending in): the budget page is about money coming in and
//!   going out, so the series follows the transactions, and a budget planned ahead with a
//!   `budget-add` in a future month shows that month too. Other directives, such as prices,
//!   events, notes or balance assertions, do not extend it, so a date typo on one of them cannot
//!   add centuries of months. The rows only depend on the ledger, not on the current date; a
//!   later month looks like the last row carried over, with nothing spent.
//! - **Bounded.** The months are generated, not read from the ledger, so a typo in the date of
//!   a transaction or a budget directive could still ask for a very long series: every
//!   generated row is charged to the result budget and the deadline is checked while they are
//!   built (see [`Limits`]), so such a query stops with a "too large" or "time limit" error
//!   instead of exhausting memory. Filters run after the rows are generated, so advice to
//!   narrow the query would not help: the "too large" error names the budget with the longest
//!   series and the directive whose date ends it, which is where the typo usually is.
//! - **Months are dates.** `date` is the first day of the month, so the date literals,
//!   comparisons and functions of the other tables work (`WHERE date >= 2024-01-01`), and
//!   `year` and `month` are there for grouping, as in the `postings` table.
//! - **Who the budget is.** `name` is the name directives use; `alias` and `category` are the
//!   display name and the group the UI shows (`NULL` if unset), and `accounts` the accounts
//!   whose postings count as the budget's activity. `meta()` reads the metadata of the
//!   `budget` directive.
//! - **Closing.** `closed` is whether the budget was closed (`budget-close`) in or before the
//!   month, so `WHERE NOT closed` lists the budgets that were open at the time. A closed
//!   budget keeps its rows, as the UI keeps showing it.
//!
//! Rows are ordered by name, then month. `SELECT *` gives the budget page's figures: `name`,
//! `date`, `assigned`, `activity` and `available`.
//!
//! `#budget_events` lists what the budget directives did, in ledger order: a `budget-add` is
//! an `assign` of its amount, a `budget-transfer` a `transfer_out` of the budget it takes from
//! (a negative amount) followed by a `transfer_in` of the budget it gives to, and a
//! `budget-close` a `close` without an amount. Directives without effect have no row. The
//! amounts are as written; `#budgets` converts them to the budget's commodity.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Months, NaiveDate, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Date, Directive, Meta, Spanned};
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::directives::date_of;
use super::{ledger_file, ColumnDef, LedgerCache, Limits, Record, Rows, Table};
use crate::error::{LocatedError, QueryErrorKind};
use crate::prices::PriceMap;
use crate::projector::Projection;
use crate::value::{Cost, DataType, Position, Value};

pub(super) static BUDGETS: Table = Table {
    name: "budgets",
    description: "One row per budget per month, from the budget's first month through its last entry or the ledger's last month with a transaction, whichever is later, with the assigned, activity and available amounts the budget pages show, in the budget's commodity; ordered by name, then month.",
    columns: COLUMNS,
    wildcard: &["name", "date", "assigned", "activity", "available"],
    rows: Rows::Generated(rows),
};

pub(super) static BUDGET_EVENTS: Table = Table {
    name: "budget_events",
    description: "One row per effect of a budget directive, in ledger order: the amount a budget-add assigns, the amount a \
                  budget-transfer moves out of one budget and into another, and a budget-close.",
    columns: EVENT_COLUMNS,
    wildcard: &["name", "date", "time", "timestamp", "type", "amount"],
    rows: Rows::Records(event_rows),
};

/// A budget, as its `budget` directive defines it.
struct Budget<'a> {
    name: &'a str,
    commodity: &'a str,
    /// the metadata of the `budget` directive
    meta: &'a Meta,
    /// the index of the `budget` directive in the ledger's directives, the order the store
    /// folds them in: only the transactions after it are the budget's activity
    defined_at: usize,
    /// the month of the definition, the first of the budget's series
    first: NaiveDate,
    /// the first month the budget is closed in
    closed_from: Option<NaiveDate>,
    /// the latest directive with an effect on the budget, and its date
    latest: (NaiveDate, &'a Spanned<Directive>),
}

impl<'a> Budget<'a> {
    fn saw(&mut self, directive: &'a Spanned<Directive>, date: NaiveDate) {
        if self.latest.0 <= date {
            self.latest = (date, directive);
        }
    }
}

/// What a budget directive does to a budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Assign,
    TransferIn,
    TransferOut,
    Close,
}

impl EventKind {
    fn name(self) -> &'static str {
        match self {
            EventKind::Assign => "assign",
            EventKind::TransferIn => "transfer_in",
            EventKind::TransferOut => "transfer_out",
            EventKind::Close => "close",
        }
    }
}

/// One effect of a budget directive: a row of the `budget_events` table.
pub(crate) struct BudgetEvent<'a> {
    name: &'a str,
    pub(super) directive: &'a Spanned<Directive>,
    kind: EventKind,
    /// the amount put into the budget, negative for a transfer out, as written; `None` for a close
    amount: Option<Amount>,
    /// the directive's time of day in the ledger's timezone
    time: NaiveTime,
    /// the Unix time of the directive's date and time in the ledger's timezone
    timestamp: i64,
}

impl BudgetEvent<'_> {
    fn date(&self) -> Option<NaiveDate> {
        date_of(&self.directive.data)
    }
}

/// The budgets of the ledger and the effects of their directives, folded in the store's order.
struct Budgets<'a> {
    budgets: HashMap<&'a str, Budget<'a>>,
    /// the effects of the budget directives, in ledger order
    events: Vec<BudgetEvent<'a>>,
}

/// Fold the budget directives the way zhang does while loading (see the module docs), in the
/// order of the ledger's directives, which the store folds them in.
fn budgets(ledger: &Ledger) -> Budgets<'_> {
    let timezone = &ledger.options.timezone;
    let event = |name, directive, kind, amount, date: &Date| {
        let datetime = date.to_timezone_datetime(timezone);
        BudgetEvent {
            name,
            directive,
            kind,
            amount,
            time: datetime.time(),
            timestamp: datetime.timestamp(),
        }
    };
    let mut budgets: HashMap<&str, Budget<'_>> = HashMap::new();
    let mut events = vec![];
    for (idx, directive) in ledger.directives.iter().enumerate() {
        match &directive.data {
            Directive::Budget(budget) => {
                let date = budget.date.naive_date();
                budgets.entry(budget.name.as_str()).or_insert_with(|| Budget {
                    name: &budget.name,
                    commodity: &budget.commodity,
                    meta: &budget.meta,
                    defined_at: idx,
                    first: first_of_month(date),
                    closed_from: None,
                    latest: (date, directive),
                });
            }
            Directive::BudgetAdd(add) => {
                if let Some(budget) = budgets.get_mut(add.name.as_str()) {
                    budget.saw(directive, add.date.naive_date());
                    events.push(event(&add.name, directive, EventKind::Assign, Some(add.amount.clone()), &add.date));
                }
            }
            Directive::BudgetTransfer(transfer) => {
                if !budgets.contains_key(transfer.from.as_str()) || !budgets.contains_key(transfer.to.as_str()) {
                    continue;
                }
                for (name, kind, amount) in [
                    (&transfer.from, EventKind::TransferOut, transfer.amount.neg()),
                    (&transfer.to, EventKind::TransferIn, transfer.amount.clone()),
                ] {
                    if let Some(budget) = budgets.get_mut(name.as_str()) {
                        budget.saw(directive, transfer.date.naive_date());
                    }
                    events.push(event(name, directive, kind, Some(amount), &transfer.date));
                }
            }
            Directive::BudgetClose(close) => {
                if let Some(budget) = budgets.get_mut(close.name.as_str()) {
                    let date = close.date.naive_date();
                    budget.saw(directive, date);
                    budget.closed_from.get_or_insert(first_of_month(date));
                    events.push(event(&close.name, directive, EventKind::Close, None, &close.date));
                }
            }
            _ => {}
        }
    }
    Budgets { budgets, events }
}

/// The last transaction the store keeps, not counting the corrections of balance assertions,
/// and its date: of the latest date, the last one folded.
fn last_transaction<'a>(ledger: &'a Ledger, store: &Store) -> Option<(NaiveDate, &'a Spanned<Directive>)> {
    let cache = LedgerCache::of(ledger, store);
    let (postings, entries) = (cache.postings(ledger, store), cache.entries(ledger, store));
    postings
        .entries
        .iter()
        .filter_map(|entry| Some((entry.date, &ledger.directives[entries.rows[entry.entry? as usize].directive as usize])))
        .max_by_key(|(date, _)| *date)
}

/// `units` held at `cost` in `commodity` as of `date`, as `convert(position, commodity, date)`
/// values the position ([`Position::convert`]); `None` when no price converts it.
fn in_commodity<'u>(units: &'u Amount, cost: Option<&Cost>, commodity: &str, prices: &PriceMap, date: NaiveDate) -> Option<Cow<'u, BigDecimal>> {
    if units.commodity == commodity {
        return Some(Cow::Borrowed(&units.number));
    }
    let converted = Position::new(units.clone(), cost.cloned()).convert(commodity, prices, Some(date));
    (converted.commodity == commodity).then_some(Cow::Owned(converted.number))
}

/// The activity of every budget per month (`(budget, first day of the month)`): the booked
/// postings of its accounts folded after its definition, converted to its commodity at their
/// date, counted negated on an Income, Liabilities or Equity account, as zhang counts them.
/// Only the cached rows of the budgets' accounts are read.
fn activity<'a>(
    ledger: &'a Ledger, store: &'a Store, budgets: &HashMap<&'a str, Budget<'a>>, accounts: &HashMap<&'a str, BTreeSet<String>>, prices: &PriceMap,
) -> HashMap<(&'a str, NaiveDate), BigDecimal> {
    // account -> (budget, whether its postings count negated)
    let mut owners: HashMap<&str, Vec<(&Budget<'a>, bool)>> = HashMap::new();
    for (name, budget_accounts) in accounts {
        let Some(budget) = budgets.get(name) else { continue };
        for account in budget_accounts {
            let negated = account.parse::<Account>().is_ok_and(|account| account.get_account_sign() < 0);
            owners.entry(account.as_str()).or_default().push((budget, negated));
        }
    }
    let mut activity: HashMap<(&str, NaiveDate), BigDecimal> = HashMap::new();
    if owners.is_empty() {
        return activity;
    }
    let cache = LedgerCache::of(ledger, store);
    let (postings, entries) = (cache.postings(ledger, store), cache.entries(ledger, store));
    for (account, owners) in &owners {
        let rows = postings.account_rows(account);
        for (budget, negated) in owners {
            // the rows come in date order: the sum of the current month, added to the budget's
            // month when the month changes
            let mut current: Option<(NaiveDate, BigDecimal)> = None;
            let mut flush = |current: Option<(NaiveDate, BigDecimal)>| {
                if let Some((month, sum)) = current {
                    *activity.entry((budget.name, month)).or_insert_with(BigDecimal::zero) += sum;
                }
            };
            for row in rows.iter().map(|idx| &postings.rows[*idx as usize]) {
                let entry = &postings.entries[row.entry as usize];
                // where the store folded the transaction, among the ledger's directives
                let folded_at = entry.entry.map(|seq| entries.rows[seq as usize].directive as usize);
                if folded_at.is_some_and(|at| at < budget.defined_at) {
                    continue;
                }
                let cost = row.lot.as_ref().and_then(|lot| lot.cost.as_ref());
                let Some(number) = in_commodity(&row.units, cost, budget.commodity, prices, entry.date) else {
                    continue;
                };
                let month = first_of_month(entry.date);
                if current.as_ref().is_none_or(|(it, _)| *it != month) {
                    flush(current.replace((month, BigDecimal::zero())));
                }
                if let Some((_, sum)) = current.as_mut() {
                    if *negated {
                        *sum -= number.as_ref();
                    } else {
                        *sum += number.as_ref();
                    }
                }
            }
            flush(current);
        }
    }
    activity
}

/// What the budget directives put into every budget per month (`(budget, first day of the
/// month)`), converted to the budget's commodity at the directive's date.
fn added<'a>(budgets: &HashMap<&'a str, Budget<'a>>, events: &[BudgetEvent<'a>], prices: &PriceMap) -> HashMap<(&'a str, NaiveDate), BigDecimal> {
    let mut added: HashMap<(&str, NaiveDate), BigDecimal> = HashMap::new();
    for event in events {
        let (Some(budget), Some(amount), Some(date)) = (budgets.get(event.name), &event.amount, event.date()) else {
            continue;
        };
        if let Some(number) = in_commodity(amount, None, budget.commodity, prices, date) {
            *added.entry((budget.name, first_of_month(date))).or_insert_with(BigDecimal::zero) += number.as_ref();
        }
    }
    added
}

/// What the months of a budget share.
struct BudgetInfo<'a> {
    name: &'a str,
    commodity: &'a str,
    /// the metadata of the `budget` directive
    meta: &'a Meta,
    /// the budget's accounts; only collected when the `accounts` column or the activity is
    /// projected
    accounts: BTreeSet<String>,
}

/// One month of a budget: a row of the `budgets` table. The amounts are only computed when
/// the projection reads them (zero otherwise).
pub(crate) struct BudgetMonth<'a> {
    budget: Rc<BudgetInfo<'a>>,
    /// the first day of the month
    month: NaiveDate,
    assigned: BigDecimal,
    added: BigDecimal,
    activity: BigDecimal,
    closed: bool,
}

impl<'a> BudgetMonth<'a> {
    /// The metadata of the `budget` directive.
    pub(super) fn metadata(&self) -> Option<&'a Meta> {
        Some(self.budget.meta)
    }

    fn amount(&self, number: &BigDecimal) -> Value {
        Value::Amount(Amount::new(number.clone(), self.budget.commodity))
    }

    fn meta(&self, key: &str) -> Value {
        self.budget.meta.get_one(key).map_or(Value::Null, |value| Value::Str(value.as_str().to_owned()))
    }
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first day")
}

/// The month series of one budget.
struct Series<'s, 'a> {
    budget: &'s Budget<'a>,
    end: NaiveDate,
    /// the directive whose date sets `end`
    end_by: &'a Spanned<Directive>,
}

impl Series<'_, '_> {
    fn months(&self) -> u64 {
        let index = |month: NaiveDate| i64::from(month.year()) * 12 + i64::from(month.month0());
        u64::try_from(index(self.end) - index(self.budget.first) + 1).unwrap_or(0)
    }
}

fn rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection, limits: &mut Limits<'_>) -> Result<Vec<Record<'a>>, LocatedError> {
    let Budgets { budgets, events } = budgets(ledger);
    let last_transaction = last_transaction(ledger, store);
    let wants_activity = ["activity", "assigned", "available"].into_iter().any(|name| projects(projection, name));
    let wants_added = ["added", "assigned", "available"].into_iter().any(|name| projects(projection, name));

    let mut accounts: HashMap<&str, BTreeSet<String>> = HashMap::new();
    if wants_activity || projects(projection, "accounts") {
        for meta in &store.metas {
            if meta.meta_type == MetaType::AccountMeta.as_ref() && meta.key == "budget" {
                accounts.entry(meta.value.as_str()).or_default().insert(meta.type_identifier.clone());
            }
        }
    }
    let prices = (wants_activity || wants_added).then(|| LedgerCache::of(ledger, store).prices(store));
    let mut activity = match &prices {
        Some(prices) if wants_activity => activity(ledger, store, &budgets, &accounts, prices),
        _ => HashMap::new(),
    };
    let mut added = match &prices {
        Some(prices) if wants_added => added(&budgets, &events, prices),
        _ => HashMap::new(),
    };

    // a series ends with the budget's own last month, or the last month with a transaction
    let last_transaction_month = last_transaction.map(|(date, _)| first_of_month(date));
    let mut sorted = budgets.values().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.name.cmp(b.name));
    let all = sorted
        .into_iter()
        .map(|budget| {
            let own = first_of_month(budget.latest.0);
            match (last_transaction, last_transaction_month) {
                (Some((_, transaction)), Some(month)) if month > own => Series {
                    budget,
                    end: month,
                    end_by: transaction,
                },
                _ => Series {
                    budget,
                    end: own,
                    end_by: budget.latest.1,
                },
            }
        })
        .collect::<Vec<_>>();

    let mut records = vec![];
    for one in &all {
        let budget = one.budget;
        let info = Rc::new(BudgetInfo {
            name: budget.name,
            commodity: budget.commodity,
            meta: budget.meta,
            accounts: accounts.remove(budget.name).unwrap_or_default(),
        });
        let mut available = BigDecimal::zero();
        let mut month = budget.first;
        while month <= one.end {
            limits.row().map_err(|err| too_many_months(err, ledger, &all))?;
            let added = added.remove(&(budget.name, month)).unwrap_or_else(BigDecimal::zero);
            let activity = activity.remove(&(budget.name, month)).unwrap_or_else(BigDecimal::zero);
            let assigned = &available + &added;
            available = &assigned - &activity;
            records.push(Record::Budget(BudgetMonth {
                budget: Rc::clone(&info),
                month,
                assigned,
                added,
                activity,
                closed: budget.closed_from.is_some_and(|from| from <= month),
            }));
            let Some(following) = month.checked_add_months(Months::new(1)) else {
                break;
            };
            month = following;
        }
    }
    Ok(records)
}

/// The error of a series too long for the result budget. Its rows are generated before any
/// filter runs, so the generic advice to narrow the query cannot help: name the budget with
/// the longest series and the directive whose date sets its end, which is usually a typo.
fn too_many_months(err: LocatedError, ledger: &Ledger, all: &[Series<'_, '_>]) -> LocatedError {
    if err.kind != QueryErrorKind::TooLarge {
        return err;
    }
    let total = all.iter().map(Series::months).sum::<u64>();
    let Some(longest) = all.iter().rev().max_by_key(|series| series.months()) else {
        return err;
    };
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    let directive = longest.end_by;
    let kind = match &directive.data {
        Directive::Transaction(_) => Some("a transaction"),
        Directive::Budget(_) => Some("its budget directive"),
        Directive::BudgetAdd(_) => Some("a budget-add"),
        Directive::BudgetTransfer(_) => Some("a budget-transfer"),
        Directive::BudgetClose(_) => Some("a budget-close"),
        _ => None,
    };
    let cause = kind
        .zip(date_of(&directive.data))
        .map(|(kind, date)| {
            let file = directive
                .span
                .filename
                .as_deref()
                .map(|path| format!(" ({})", ledger_file(ledger, path).display()))
                .unwrap_or_default();
            format!(" because of {} dated {}{}; check that date", kind, date, file)
        })
        .unwrap_or_default();
    LocatedError {
        kind: QueryErrorKind::TooLarge,
        message: format!(
            "the #budgets table would generate too many rows ({} months in all, more than the result size limit): \
             budget '{}' runs from {} until {}{}",
            total,
            longest.budget.name,
            month(longest.budget.first),
            month(longest.end),
            cause
        ),
        span: None,
    }
}

/// Whether the column `name` of this table is projected.
fn projects(projection: Projection, name: &str) -> bool {
    BUDGETS.column(name).is_some_and(|column| projection.contains(column))
}

fn budget_month<'r, 'a>(record: &'r Record<'a>) -> Option<&'r BudgetMonth<'a>> {
    match record {
        Record::Budget(month) => Some(month),
        _ => None,
    }
}

static COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("name", DataType::Str, "Name of the budget, as written in its directives.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Str(it.budget.name.to_owned()))
    }),
    ColumnDef::record(
        "alias",
        DataType::Str,
        "Display name of the budget (its alias metadata), or NULL if it has none.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.meta("alias")),
    ),
    ColumnDef::record(
        "category",
        DataType::Str,
        "Category the budget is grouped under (its category metadata), or NULL if it has none.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.meta("category")),
    ),
    ColumnDef::record("currency", DataType::Str, "Commodity the budget is kept in.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Str(it.budget.commodity.to_owned()))
    }),
    ColumnDef::record("date", DataType::Date, "First day of the month.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Date(it.month))
    }),
    ColumnDef::record("year", DataType::Int, "Year of the month.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Int(i64::from(it.month.year())))
    }),
    ColumnDef::record("month", DataType::Int, "Month of the year, from 1 to 12.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Int(i64::from(it.month.month())))
    }),
    ColumnDef::record(
        "assigned",
        DataType::Amount,
        "Amount assigned to the budget for the month: the available amount carried over from the previous month plus what was added in the month.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(&it.assigned)),
    ),
    ColumnDef::record(
        "added",
        DataType::Amount,
        "Amount the month's budget-add and budget-transfer directives put into the budget, converted to its commodity at their date; a transfer out counts as negative.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(&it.added)),
    ),
    ColumnDef::record(
        "activity",
        DataType::Amount,
        "Amount the budget's accounts spent in the month: their booked postings, each converted to the budget's commodity at its date; postings no price converts are left out.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(&it.activity)),
    ),
    ColumnDef::record(
        "available",
        DataType::Amount,
        "Amount left at the end of the month (assigned minus activity); it carries over to the next month.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(&(&it.assigned - &it.activity))),
    ),
    ColumnDef::record(
        "accounts",
        DataType::Set,
        "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
        |_, record| budget_month(record).map_or(Value::Null, |it| Value::Set(it.budget.accounts.clone())),
    ),
    ColumnDef::record(
        "closed",
        DataType::Bool,
        "Whether the budget was closed (budget-close) in or before the month.",
        |_, record| budget_month(record).map_or(Value::Null, |it| Value::Bool(it.closed)),
    ),
];

// ---------------------------------------------------------------------------------------
// #budget_events

fn event_rows<'a>(ledger: &'a Ledger, _store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    budgets(ledger).events.into_iter().map(Record::BudgetEvent).collect()
}

fn budget_event<'r, 'a>(record: &'r Record<'a>) -> Option<&'r BudgetEvent<'a>> {
    match record {
        Record::BudgetEvent(event) => Some(event),
        _ => None,
    }
}

static EVENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("name", DataType::Str, "Name of the budget.", |_, record| {
        budget_event(record).map_or(Value::Null, |it| Value::Str(it.name.to_owned()))
    }),
    ColumnDef::record("date", DataType::Date, "Date of the budget directive.", |_, record| {
        budget_event(record).and_then(BudgetEvent::date).map_or(Value::Null, Value::Date)
    }),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the budget directive in the ledger's timezone, as HH:MM:SS; 00:00:00 without a time.",
        |_, record| budget_event(record).map_or(Value::Null, |it| Value::Str(it.time.format("%H:%M:%S").to_string())),
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time in seconds of the budget directive's date and time in the ledger's timezone.",
        |_, record| budget_event(record).map_or(Value::Null, |it| Value::Int(it.timestamp)),
    ),
    ColumnDef::record(
        "type",
        DataType::Str,
        "What the directive does to the budget: 'assign' (budget-add), 'transfer_out' and 'transfer_in' (budget-transfer) or 'close' (budget-close).",
        |_, record| budget_event(record).map_or(Value::Null, |it| Value::Str(it.kind.name().to_owned())),
    ),
    ColumnDef::record(
        "amount",
        DataType::Amount,
        "Amount put into the budget, as written; negative for a transfer out, NULL for a close.",
        |_, record| budget_event(record).and_then(|it| it.amount.clone()).map_or(Value::Null, Value::Amount),
    ),
];
