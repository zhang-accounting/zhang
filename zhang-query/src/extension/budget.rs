//! `zhang:budget`, the native provider of the budget tables: `#budgets`, one row per budget per month with the figures
//! the budget pages of the web UI show for that month, `#budget_events`, one row per effect of a budget directive,
//! `#budget_definitions`, one row per budget, and the `budgets` column of `postings`.
//!
//! The provider serves them as extension tables ([`crate::extension`]), from the native budget code below, until the
//! official plugin `zhang:budget` takes them over (RFC #698): `#budget_definitions` and `#budget_events` are record
//! tables, `#budgets` a series table whose facts are the months with a definition, an amount put in, activity or a
//! close, and `budgets` a label column whose predicate is the membership rule below, one label per budget.
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
//!   and the booked postings of the ledger's cache ([`LedgerCache`]), in the order the load
//!   folded them (the order of the ledger's directives), with the rules zhang applies while
//!   loading: a budget exists from its first `budget` directive on
//!   (a second one of the same name is a duplicate zhang reports), a `budget-add`,
//!   `budget-transfer` or `budget-close` naming a budget that does not exist yet is an error
//!   without effect, and only the postings folded after the budget's definition are its
//!   activity.
//! - **One rule of what counts.** A posting counts toward the budgets of its account's `open` in
//!   effect at its date and time, from the budget's definition until its close, when a price
//!   converts it ([`Budget::counts`]). The `budgets` column of `#postings`, which the budget page's
//!   posting list reads, follows the same rule, written as its label predicate ([`PREDICATE`]), so the postings of a
//!   budget's month add up to its `activity`.
//! - **In the budget's commodity.** `activity` adds up the booked postings of the budget's
//!   accounts (the rows of the postings table), each converted to the budget's commodity at
//!   the posting's date as `convert(position, commodity, date)` does, with the prices of the
//!   ledger. An amount of a `budget-add` or `budget-transfer` in another commodity is
//!   converted the same way at the directive's date. A posting or amount that no price
//!   converts is left out rather than added as a number of another commodity; zhang reports
//!   it as `BudgetCommodityMismatch`. Postings to an Income, Liabilities or Equity account
//!   count negated, as zhang counts them.
//! - **`added`, for sums over months.** Because `assigned` includes the carry-over, adding it
//!   up over months counts the same money several times. `added` is only what the month's
//!   `budget-add` and `budget-transfer` directives put in (a transfer out counts as negative),
//!   so `sum(added)` is how much was budgeted over a period, next to `sum(activity)`, what was
//!   spent. `assigned - added` is the carry-over.
//! - **Every month, not only months with entries.** The UI shows a budget for every month
//!   from its first one, carrying the available amount over months without entries, so the
//!   table does too: a query for one month lists every budget the UI lists, and a budget's rows
//!   form a gap-free monthly series. The series runs from the month of the budget's
//!   definition through the latest of the budget's own last month (its last `budget-add`,
//!   `budget-transfer` or `budget-close`), the ledger's last month with a transaction (the
//!   last month the budget can have spending in) and the current month (of `today()`, in the
//!   ledger's timezone): the budget page is about money coming in and going out, so the series
//!   follows the transactions, a budget planned ahead with a `budget-add` in a future month
//!   shows that month too, and "this month" (`WHERE date = yearmonth(today())`) lists every
//!   budget even before its first transaction of the month. Other directives, such as prices,
//!   events, notes or balance assertions, do not extend it, so a date typo on one of them cannot
//!   add centuries of months. A month after the series looks like its last row carried over,
//!   with nothing spent.
//! - **Bounded.** The months are generated, not read from the ledger, so a typo in the date of
//!   a transaction or a budget directive could still ask for a very long series: every
//!   generated row is charged to the result budget and the deadline is checked while they are
//!   built (see [`crate::table::Limits`]), so such a query stops with a "too large" or "time limit" error
//!   instead of exhausting memory. A query whose filter keeps only the months up to a date
//!   (`date <= :month`, `yearmonth(date) = :month`, see [`crate::optimizer::date_bound`])
//!   generates no later month, so the budget pages, which ask for one month, keep working
//!   whatever typo the ledger holds. Other filters run after the rows are generated, so advice
//!   to narrow the query would not help: the "too large" error names the budget with the
//!   longest series and the directive whose date ends it, which is where the typo usually is.
//! - **Months are dates.** `date` is the first day of the month, so the date literals,
//!   comparisons and functions of the other tables work (`WHERE date >= 2024-01-01`), and
//!   `year` and `month` are there for grouping, as in the `postings` table.
//! - **Who the budget is.** `name` is the name directives use; `alias` and `category` are the
//!   display name and the group the UI shows (`NULL` if unset), and `accounts` the accounts
//!   whose postings count as the budget's activity. `meta()` reads the metadata of the
//!   `budget` directive.
//! - **Closing.** `closed` is whether the budget was closed (`budget-close`) in or before the
//!   month, so `WHERE NOT closed` lists the budgets that were open at the time. A closed
//!   budget keeps its rows, as the UI keeps showing it, but takes no activity after its close:
//!   a budget is open through the whole day of a `budget-close` written with a date only, and
//!   until the time of one written with a time ([`Date::close_precedes`]). zhang reports the
//!   postings it leaves out as `BudgetClosed`. Its `budget-add` and `budget-transfer`
//!   directives still count, so what is left can be moved to another budget.
//!
//! Rows are ordered by name, then month. `SELECT *` gives the budget page's figures: `name`,
//! `date`, `assigned`, `activity` and `available`.
//!
//! `#budget_definitions` has one row per budget, as its directives define it: its commodity,
//! display name, category, accounts and the date and time of its first `budget-close`. It has no months
//! and reads no transaction, so what a budget is can be asked whatever dates the ledger holds.
//!
//! `#budget_events` lists what the budget directives did, in ledger order: a `budget-add` is
//! an `assign` of its amount, a `budget-transfer` a `transfer_out` of the budget it takes from
//! (a negative amount) followed by a `transfer_in` of the budget it gives to, and a
//! `budget-close` a `close` without an amount. Directives without effect have no row. The
//! amounts are as written; `#budgets` converts them to the budget's commodity.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Date, Directive, Meta, Spanned};
use zhang_core::ledger::Ledger;

use super::{
    Cause, ColumnDecl, DatePart, FactRow, Fill, Label, LabelDecl, Manifest, NamedQuery, NativeProvider, Partition, PeriodUnit, RecordOrder, RecordRow,
    SeriesDecl, TableData, TableDecl, TableKind,
};
use crate::params::Params;
use crate::prices::PriceMap;
use crate::table::{date_of, time_value, LedgerCache};
use crate::value::{first_of_month, Cost, DataType, Position, Value};

/// The provider.
pub(crate) static PROVIDER: NativeProvider = NativeProvider { manifest, table, labels };

/// The predicate of the `budgets` column of `postings`, the one rule of membership ([`Budget::counts`]) in BQL: the
/// row's account is one of the budget's (a fast test the index of the label column uses), its transaction was folded
/// after the budget's definition, the `budget` metadata of its account's `open` in effect at its date and time names
/// the budget (`open_meta_has`, which reads the `open` and the row's time in place), the budget is not closed by then,
/// and a price converts it to the budget's commodity.
pub(crate) const PREDICATE: &str = "account IN :accounts AND seq > :defined_seq \
    AND open_meta_has(account, 'budget', :label, date, time) \
    AND (:close_date IS NULL OR (:close_time IS NULL AND date <= :close_date) \
         OR (:close_time IS NOT NULL AND (date < :close_date OR (date = :close_date AND time <= :close_time)))) \
    AND currency(convert(position, :commodity, date)) = :commodity";

fn column(name: &str, ty: DataType, description: &str, fill: Option<Fill>) -> ColumnDecl {
    ColumnDecl {
        name: name.to_owned(),
        ty,
        description: description.to_owned(),
        fill,
    }
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// `budgets.month` without its order, and with a filter `budgets.budget_month`.
macro_rules! budgets_as_of_month {
    ($($filter:literal)?) => { concat!("SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, \
              CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned, \
              CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE ", $($filter, " AND ",)? "date <= :month GROUP BY name") };
}

/// The named queries of the provider, those the budget pages of the web UI and the app run, which zhang's built-in
/// queries have always had, with their text unchanged. A month is the first day of the month (`2024-06-01`).
fn queries() -> Vec<NamedQuery> {
    use DataType::{Date, Str};
    let query = |name: &str, description: &str, bql: &str, params: &[(&str, DataType)]| NamedQuery {
        name: name.to_owned(),
        description: description.to_owned(),
        bql: bql.to_owned(),
        params: params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect(),
    };
    vec![
        query(
            "budgets.month",
            "Every budget as of a month (its first day): its last month in #budgets up to that month, carried over to the month when it is later.",
            concat!(budgets_as_of_month!(), " ORDER BY name"),
            &[("month", Date)],
        ),
        query(
            "budgets.budget",
            "One budget: its display name, category, commodity, the accounts whose postings are its activity, and the date and time of its close.",
            "SELECT name, alias, category, currency, accounts, close, close_time \
              FROM #budget_definitions \
              WHERE name = :name",
            &[("name", Str)],
        ),
        query(
            "budgets.budget_month",
            "One budget as of a month (its first day), as in budgets.month.",
            budgets_as_of_month!("name = :name"),
            &[("name", Str), ("month", Date)],
        ),
        query(
            "budgets.events",
            "What the budget-add and budget-transfer directives put into a budget in a month (its first day), newest first.",
            "SELECT date, time, timestamp, type, amount \
              FROM #budget_events \
              WHERE name = :name AND type != 'close' AND yearmonth(date) = :month \
              ORDER BY timestamp DESC",
            &[("name", Str), ("month", Date)],
        ),
        query(
            "budgets.postings",
            "The postings that count toward a budget in a month (its first day), newest first, with each account's balance after them: they add up to the month's activity.",
            "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units, \
              only(currency, account_balance) AS balance \
              WHERE yearmonth(date) = :month AND :name IN budgets \
              ORDER BY timestamp DESC",
            &[("name", Str), ("month", Date)],
        ),
    ]
}

/// The tables, the label column and the named queries of the provider, as the budget tables and the budget pages'
/// queries have always described them.
fn manifest() -> Manifest {
    use DataType::{Amount, Bool, Date, Int, Set, Str};
    let carry = || Some(Fill::Carry);
    let budgets = TableDecl {
        name: "budgets".to_owned(),
        published_as: "budgets".to_owned(),
        description: "One row per budget per month, from the budget's first month through its last entry, the ledger's last month with a transaction or the current month, whichever is latest, with the assigned, activity and available amounts the budget pages show, in the budget's commodity; ordered by name, then month.".to_owned(),
        kind: TableKind::Series(SeriesDecl {
            period: "date".to_owned(),
            unit: PeriodUnit::Month,
            derive: vec![("year".to_owned(), DatePart::Year), ("month".to_owned(), DatePart::Month)],
            partition: names(&["name"]),
            extend_to_today: true,
        }),
        columns: vec![
            column("name", Str, "Name of the budget, as written in its directives.", carry()),
            column("alias", Str, "Display name of the budget (its alias metadata), or NULL if it has none.", carry()),
            column("category", Str, "Category the budget is grouped under (its category metadata), or NULL if it has none.", carry()),
            column("currency", Str, "Commodity the budget is kept in.", carry()),
            column("date", Date, "First day of the month.", None),
            column("year", Int, "Year of the month.", None),
            column("month", Int, "Month of the year, from 1 to 12.", None),
            column(
                "assigned",
                Amount,
                "Amount assigned to the budget for the month: the available amount carried over from the previous month plus what was added in the month.",
                Some(Fill::Previous("available".to_owned())),
            ),
            column(
                "added",
                Amount,
                "Amount the month's budget-add and budget-transfer directives put into the budget, converted to its commodity at their date; a transfer out counts as negative.",
                Some(Fill::Zero),
            ),
            column(
                "activity",
                Amount,
                "Amount the budget's accounts spent in the month: their booked postings, each converted to the budget's commodity at its date; postings no price converts, and those after the budget's close, are left out.",
                Some(Fill::Zero),
            ),
            column(
                "available",
                Amount,
                "Amount left at the end of the month (assigned minus activity); it carries over to the next month.",
                carry(),
            ),
            column(
                "accounts",
                Set,
                "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
                carry(),
            ),
            column("closed", Bool, "Whether the budget was closed (budget-close) in or before the month.", carry()),
        ],
        wildcard: names(&["name", "date", "assigned", "activity", "available"]),
    };
    let events = TableDecl {
        name: "budget_events".to_owned(),
        published_as: "budget_events".to_owned(),
        description: "One row per effect of a budget directive, in ledger order: the amount a budget-add assigns, the amount a \
                      budget-transfer moves out of one budget and into another, and a budget-close."
            .to_owned(),
        kind: TableKind::Records(RecordOrder::Published),
        columns: vec![
            column("name", Str, "Name of the budget.", None),
            column("date", Date, "Date of the budget directive.", None),
            column(
                "time",
                Str,
                "Time of day of the budget directive in the ledger's timezone, as HH:MM:SS; 00:00:00 without a time.",
                None,
            ),
            column(
                "timestamp",
                Int,
                "Unix time in seconds of the budget directive's date and time in the ledger's timezone.",
                None,
            ),
            column(
                "type",
                Str,
                "What the directive does to the budget: 'assign' (budget-add), 'transfer_out' and 'transfer_in' (budget-transfer) or 'close' (budget-close).",
                None,
            ),
            column(
                "amount",
                Amount,
                "Amount put into the budget, as written; negative for a transfer out, NULL for a close.",
                None,
            ),
        ],
        wildcard: names(&["name", "date", "time", "timestamp", "type", "amount"]),
    };
    let definitions = TableDecl {
        name: "budget_definitions".to_owned(),
        published_as: "budget_definitions".to_owned(),
        description: "One row per budget, as its directives define it: its commodity, display name, category, the accounts of its activity and the date and time it is closed; ordered by name. It has no months, so it reads no transaction.".to_owned(),
        kind: TableKind::Records(RecordOrder::Columns(names(&["name"]))),
        columns: vec![
            column("name", Str, "Name of the budget, as written in its directives.", None),
            column("date", Date, "Date of the budget directive, from which the budget exists.", None),
            column("currency", Str, "Commodity the budget is kept in.", None),
            column("alias", Str, "Display name of the budget (its alias metadata), or NULL if it has none.", None),
            column("category", Str, "Category the budget is grouped under (its category metadata), or NULL if it has none.", None),
            column(
                "accounts",
                Set,
                "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
                None,
            ),
            column(
                "close",
                Date,
                "Date of the budget's first budget-close, which closes it, or NULL while it is open.",
                None,
            ),
            column(
                "close_time",
                Str,
                "Time of day of the budget's first budget-close, as HH:MM:SS, until which the budget takes activity on its close day; NULL for a budget-close without a time, which leaves the budget open through its whole close day, or while it is open.",
                None,
            ),
        ],
        wildcard: names(&["name", "date", "currency", "alias", "category", "accounts", "close", "close_time"]),
    };
    let postings_budgets = LabelDecl {
        table: "postings".to_owned(),
        name: "budgets".to_owned(),
        published_as: "budgets".to_owned(),
        description: "The budgets the posting counts toward, as the budget pages count it: those the budget metadata of its \
        account's open in effect at its date and time names, defined before it and not closed by then, into \
        whose commodity a price converts it. The postings of a budget in a month add up to its activity in \
        #budgets. A zhang extension."
            .to_owned(),
        predicate: PREDICATE.to_owned(),
        params: vec![
            ("accounts".to_owned(), Set),
            ("defined_seq".to_owned(), Int),
            ("close_date".to_owned(), Date),
            ("close_time".to_owned(), Str),
            ("commodity".to_owned(), Str),
        ],
    };
    Manifest {
        provider: super::compat::BUDGET.to_owned(),
        tables: vec![budgets, events, definitions],
        labels: vec![postings_budgets],
        queries: queries(),
    }
}

/// The rows of the table `name` of `ledger`.
fn table(ledger: &Ledger, name: &str) -> TableData {
    match name {
        "budgets" => TableData::Series(months(ledger)),
        "budget_events" => TableData::Records(event_rows(ledger)),
        "budget_definitions" => TableData::Records(definition_rows(ledger)),
        _ => TableData::Records(vec![]),
    }
}

/// The labels of the label column `name` of `ledger`: of `budgets`, one per budget, by name.
fn labels(ledger: &Ledger, name: &str) -> Vec<Label> {
    if name != "budgets" {
        return vec![];
    }
    let mut accounts = budget_accounts(ledger);
    let mut budgets = budgets(ledger).budgets.into_values().collect::<Vec<_>>();
    budgets.sort_by(|a, b| a.name.cmp(b.name));
    budgets
        .into_iter()
        .map(|budget| {
            let (close_date, close_time) = match budget.close {
                None => (Value::Null, Value::Null),
                Some(Date::Date(date)) => (Value::Date(*date), Value::Null),
                Some(Date::DateHour(datetime) | Date::Datetime(datetime)) => (Value::Date(datetime.date()), time_value(datetime.time())),
            };
            let params = Params::new()
                .bind("accounts", Value::Set(accounts.remove(budget.name).unwrap_or_default()))
                .bind("defined_seq", seq_value(ledger, budget.defined_at))
                .bind("close_date", close_date)
                .bind("close_time", close_time)
                .bind("commodity", budget.commodity);
            Label {
                label: budget.name.to_owned(),
                params,
            }
        })
        .collect()
}

/// The `seq` of the directive at `index` in the ledger's directives, as an int; NULL when it is no entry.
fn seq_value(ledger: &Ledger, index: usize) -> Value {
    seq_of(ledger, index).map_or(Value::Null, |seq| Value::Int(seq.into()))
}

/// The `seq` of the directive at `index` in the ledger's directives: its row of `#entries`.
fn seq_of(ledger: &Ledger, index: usize) -> Option<u32> {
    ledger.outcomes.get(index).and_then(|outcome| outcome.seq)
}

/// A budget, as its `budget` directive defines it.
struct Budget<'a> {
    name: &'a str,
    commodity: &'a str,
    /// the metadata of the `budget` directive
    meta: &'a Meta,
    /// the index of the `budget` directive in the ledger's directives, the order the load
    /// folds them in: only the transactions after it are the budget's activity
    defined_at: usize,
    /// the date of the definition
    defined: NaiveDate,
    /// the month of the definition, the first of the budget's series
    first: NaiveDate,
    /// the date of the budget's first `budget-close`, which closes it
    close: Option<&'a Date>,
    /// the latest directive with an effect on the budget (its index in the ledger's directives), and its date
    latest: (NaiveDate, usize),
}

impl<'a> Budget<'a> {
    fn saw(&mut self, index: usize, date: NaiveDate) {
        if self.latest.0 <= date {
            self.latest = (date, index);
        }
    }

    /// What `posting` counts toward the budget: its units in the budget's commodity, or `None` when it does not count.
    ///
    /// The one rule of budget membership, which the activity of `#budgets` and the `budgets` column of `#postings` (the
    /// budget page's posting list, whose label predicate [`PREDICATE`] is this rule in BQL) share, so the list adds up
    /// to the activity. A posting counts toward the budget when
    /// - the `budget` metadata of its account's `open` in effect at its date and time names the budget
    ///   ([`Ledger::account_budgets`], the rule the load reports `BudgetDoesNotExist` and `BudgetClosed` with);
    /// - the load folded it after the budget's definition;
    /// - the budget is not closed by then ([`Date::close_precedes`]);
    /// - a price converts its units to the budget's commodity at its date (zhang reports one that does not as
    ///   `BudgetCommodityMismatch`).
    fn counts<'u>(&self, ledger: &Ledger, prices: &PriceMap, posting: &BudgetPosting<'u>) -> Option<Cow<'u, BigDecimal>> {
        if !ledger
            .account_budgets(posting.account, posting.at)
            .is_some_and(|budgets| budgets.contains(self.name))
        {
            return None;
        }
        if posting.folded_at.is_some_and(|at| at < self.defined_at) {
            return None;
        }
        // a closed budget takes no activity after its close (#499)
        if self.close.is_some_and(|close| close.close_precedes(posting.at)) {
            return None;
        }
        in_commodity(posting.units, posting.cost, self.commodity, prices, posting.at.date())
    }
}

/// A posting, as [`Budget::counts`] reads it.
struct BudgetPosting<'u> {
    account: &'u str,
    /// the wall-clock date and time of its transaction in the ledger's timezone
    at: NaiveDateTime,
    /// where the load folded its transaction, among the ledger's directives
    folded_at: Option<usize>,
    units: &'u Amount,
    /// the cost of its lot
    cost: Option<&'u Cost>,
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
struct BudgetEvent<'a> {
    name: &'a str,
    directive: &'a Spanned<Directive>,
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

/// The budgets of the ledger and the effects of their directives, folded in the load's order.
struct Budgets<'a> {
    budgets: HashMap<&'a str, Budget<'a>>,
    /// the effects of the budget directives, in ledger order
    events: Vec<BudgetEvent<'a>>,
}

/// Fold the budget directives the way zhang does while loading (see the module docs), in the
/// order of the ledger's directives, which the load folds them in.
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
                    defined: date,
                    first: first_of_month(date),
                    close: None,
                    latest: (date, idx),
                });
            }
            Directive::BudgetAdd(add) => {
                if let Some(budget) = budgets.get_mut(add.name.as_str()) {
                    budget.saw(idx, add.date.naive_date());
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
                        budget.saw(idx, transfer.date.naive_date());
                    }
                    events.push(event(name, directive, kind, Some(amount), &transfer.date));
                }
            }
            Directive::BudgetClose(close) => {
                if let Some(budget) = budgets.get_mut(close.name.as_str()) {
                    let date = close.date.naive_date();
                    budget.saw(idx, date);
                    // a budget closes with its first budget-close
                    budget.close.get_or_insert(&close.date);
                    events.push(event(&close.name, directive, EventKind::Close, None, &close.date));
                }
            }
            _ => {}
        }
    }
    Budgets { budgets, events }
}

/// The last transaction the load accepted, its date and its `seq`: of the latest date, the last one folded.
fn last_transaction(ledger: &Ledger) -> Option<(NaiveDate, u32)> {
    let postings = LedgerCache::of(ledger).postings(ledger);
    postings
        .entries
        .iter()
        .filter_map(|entry| Some((entry.date, entry.entry?)))
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
/// postings of its accounts that count toward it ([`Budget::counts`]), in its commodity, counted
/// negated on an Income, Liabilities or Equity account, as zhang counts them. Only the cached rows
/// of the budgets' accounts are read.
fn activity<'a>(
    ledger: &'a Ledger, budgets: &HashMap<&'a str, Budget<'a>>, accounts: &HashMap<String, BTreeSet<String>>, prices: &PriceMap,
) -> HashMap<(&'a str, NaiveDate), BigDecimal> {
    // account -> (budget, whether its postings count negated)
    let mut owners: HashMap<&str, Vec<(&Budget<'a>, bool)>> = HashMap::new();
    for (name, budget_accounts) in accounts {
        let Some(budget) = budgets.get(name.as_str()) else { continue };
        for account in budget_accounts {
            let negated = account.parse::<Account>().is_ok_and(|account| account.get_account_sign() < 0);
            owners.entry(account.as_str()).or_default().push((budget, negated));
        }
    }
    let mut activity: HashMap<(&str, NaiveDate), BigDecimal> = HashMap::new();
    if owners.is_empty() {
        return activity;
    }
    let cache = LedgerCache::of(ledger);
    let (postings, entries) = (cache.postings(ledger), cache.entries(ledger));
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
            for (row, entry, _) in postings.at(rows.iter().map(|idx| *idx as usize)) {
                let posting = BudgetPosting {
                    account,
                    at: entry.date.and_time(entry.datetime.time()),
                    folded_at: entry.entry.map(|seq| entries.rows[seq as usize].directive as usize),
                    units: &row.units,
                    cost: row.lot.as_ref().and_then(|lot| lot.cost.as_ref()),
                };
                let Some(number) = budget.counts(ledger, prices, &posting) else {
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

/// The series of every budget, by name: its facts are the months from its first one that have something, its
/// definition, an amount put in, activity or its close, with the running figures the budget page shows; a month
/// without one carries them over, as the series generates it ([`crate::extension::series`]). The series runs through
/// the budget's own last month (its latest record, the later folded on a tie), or the ledger's last month with a
/// transaction when that is strictly later; the current month and the query's bound are the host's.
fn months(ledger: &Ledger) -> Vec<Partition> {
    let Budgets { budgets, events } = budgets(ledger);
    let mut accounts = budget_accounts(ledger);
    let prices = LedgerCache::of(ledger).prices(ledger);
    let mut activity = activity(ledger, &budgets, &accounts, prices);
    let mut added = added(&budgets, &events, prices);
    let last_transaction = last_transaction(ledger);
    // the months each budget has an amount put in or activity in
    let mut months: HashMap<&str, BTreeSet<NaiveDate>> = HashMap::new();
    for (name, month) in added.keys().chain(activity.keys()) {
        months.entry(*name).or_default().insert(*month);
    }

    let mut sorted = budgets.values().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.name.cmp(b.name));
    sorted
        .into_iter()
        .map(|budget| {
            let accounts = Value::Set(accounts.remove(budget.name).unwrap_or_default());
            let meta_value = |key: &str| budget.meta.get_one(key).map_or(Value::Null, |value| Value::Str(value.as_str().to_owned()));
            // the cells every month of the budget has, cloned into each fact
            // TODO(P1.6): share them per partition (they are the same in every fact) once cells can be shared
            let (name, alias, category, currency) = (
                Value::Str(budget.name.to_owned()),
                meta_value("alias"),
                meta_value("category"),
                Value::Str(budget.commodity.to_owned()),
            );
            let amount = |number: BigDecimal| Value::Amount(Amount::new(number, budget.commodity));
            let close_month = budget.close.map(|close| first_of_month(close.naive_date()));
            // the months with something, from the first one on: the figures of the others are the carried-over ones
            let mut facts = months.remove(budget.name).unwrap_or_default();
            facts.retain(|month| *month >= budget.first);
            facts.insert(budget.first);
            facts.extend(close_month.filter(|month| *month >= budget.first));
            let meta = Arc::new(budget.meta.clone());
            let mut available = BigDecimal::zero();
            let facts = facts
                .into_iter()
                .map(|month| {
                    let added = added.remove(&(budget.name, month)).unwrap_or_else(BigDecimal::zero);
                    let activity = activity.remove(&(budget.name, month)).unwrap_or_else(BigDecimal::zero);
                    let assigned = &available + &added;
                    available = &assigned - &activity;
                    let cells = vec![
                        name.clone(),
                        alias.clone(),
                        category.clone(),
                        currency.clone(),
                        Value::Date(month),
                        Value::Int(i64::from(month.year())),
                        Value::Int(i64::from(month.month())),
                        amount(assigned),
                        amount(added),
                        amount(activity),
                        amount(available.clone()),
                        accounts.clone(),
                        Value::Bool(close_month.is_some_and(|close| close <= month)),
                    ];
                    FactRow {
                        period: month,
                        cells: cells.into_boxed_slice(),
                        meta: Some(meta.clone()),
                    }
                })
                .collect();
            // a series ends with the budget's own last month, or the last month with a transaction when it is later
            let mut until = (first_of_month(budget.latest.0), seq_of(ledger, budget.latest.1));
            if let Some((date, seq)) = last_transaction {
                if first_of_month(date) > until.0 {
                    until = (first_of_month(date), Some(seq));
                }
            }
            Partition {
                facts,
                until: until.0,
                until_cause: until.1.map(|seq| Cause { seq }),
                first_cause: seq_of(ledger, budget.defined_at).map(|seq| Cause { seq }),
            }
        })
        .collect()
}

/// The accounts of every budget: those whose `open` has a `budget` metadata entry naming it, at
/// any time. Every entry counts, so an account whose `open` names two budgets is an account of
/// both (#479 decision 7); the load keeps only the last value of a repeated key.
fn budget_accounts(ledger: &Ledger) -> HashMap<String, BTreeSet<String>> {
    ledger.open_meta_accounts("budget")
}

/// The rows of `#budget_definitions`: every budget as its directives define it, by name.
fn definition_rows(ledger: &Ledger) -> Vec<RecordRow> {
    let mut accounts = budget_accounts(ledger);
    let mut budgets = budgets(ledger).budgets.into_values().collect::<Vec<_>>();
    budgets.sort_by(|a, b| a.name.cmp(b.name));
    budgets
        .into_iter()
        .map(|budget| {
            let meta = |key: &str| budget.meta.get_one(key).map_or(Value::Null, |value| Value::Str(value.as_str().to_owned()));
            let close_time = budget
                .close
                .and_then(|close| match close {
                    Date::Date(_) => None,
                    Date::DateHour(datetime) | Date::Datetime(datetime) => Some(time_value(datetime.time())),
                })
                .unwrap_or(Value::Null);
            let cells = vec![
                Value::Str(budget.name.to_owned()),
                Value::Date(budget.defined),
                Value::Str(budget.commodity.to_owned()),
                meta("alias"),
                meta("category"),
                Value::Set(accounts.remove(budget.name).unwrap_or_default()),
                budget.close.map_or(Value::Null, |close| Value::Date(close.naive_date())),
                close_time,
            ];
            RecordRow {
                cells: cells.into_boxed_slice(),
                meta: Some(Arc::new(budget.meta.clone())),
            }
        })
        .collect()
}

/// The rows of `#budget_events`: every effect of a budget directive, in ledger order.
fn event_rows(ledger: &Ledger) -> Vec<RecordRow> {
    budgets(ledger)
        .events
        .into_iter()
        .map(|event| {
            let cells = vec![
                Value::Str(event.name.to_owned()),
                event.date().map_or(Value::Null, Value::Date),
                time_value(event.time),
                Value::Int(event.timestamp),
                Value::Str(event.kind.name().to_owned()),
                event.amount.clone().map_or(Value::Null, Value::Amount),
            ];
            RecordRow {
                cells: cells.into_boxed_slice(),
                meta: event.directive.data.meta().cloned().map(Arc::new),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::extension::tests::ledger;
    use crate::{Params, Query};

    fn month(year: i32, month: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, 1).unwrap()
    }

    /// The `seq` of every entry of the type `kind`, in ledger order.
    fn seqs(ledger: &Ledger, kind: &str) -> Vec<u32> {
        let query = Query::compile(&format!("SELECT seq FROM #entries WHERE type = '{kind}'")).unwrap();
        let rows = query.execute_at(ledger, &Params::new(), month(2024, 1)).unwrap().rows;
        rows.iter()
            .map(|row| match row[0] {
                Value::Int(seq) => seq as u32,
                _ => panic!("a seq"),
            })
            .collect()
    }

    const OPEN: &str = "1970-01-01 open Expenses:Food\n  budget: Food\n1970-01-01 open Assets:Bank\n";

    #[test]
    fn a_budgets_own_end_is_its_record_of_the_latest_date_folded_last() {
        // an add and a close of the latest date: the one written last ends the series
        let close_last = ledger(&format!(
            "{OPEN}2024-01-01 budget Food CNY\n2024-03-10 budget-add Food 10 CNY\n2024-03-10 budget-close Food\n"
        ));
        let series = months(&close_last);
        assert_eq!(series[0].until, month(2024, 3));
        assert_eq!(
            series[0].until_cause,
            Some(Cause {
                seq: seqs(&close_last, "budget-close")[0]
            })
        );
        assert_eq!(
            series[0].first_cause,
            Some(Cause {
                seq: seqs(&close_last, "budget")[0]
            })
        );
        let add_last = ledger(&format!(
            "{OPEN}2024-01-01 budget Food CNY\n2024-03-10 budget-close Food\n2024-03-10 budget-add Food 10 CNY\n"
        ));
        let series = months(&add_last);
        assert_eq!(
            series[0].until_cause,
            Some(Cause {
                seq: seqs(&add_last, "budget-add")[0]
            })
        );
        // an earlier record of a later month still ends it
        let earlier = ledger(&format!(
            "{OPEN}2024-01-01 budget Food CNY\n2024-05-01 budget-add Food 10 CNY\n2024-03-10 budget-close Food\n"
        ));
        let series = months(&earlier);
        assert_eq!(
            (series[0].until, series[0].until_cause),
            (
                month(2024, 5),
                Some(Cause {
                    seq: seqs(&earlier, "budget-add")[0]
                })
            )
        );
    }

    #[test]
    fn the_last_transaction_ends_a_series_only_in_a_later_month() {
        let txn = |date: &str, payee: &str| format!("{date} * \"{payee}\"\n  Expenses:Food 1 CNY\n  Assets:Bank -1 CNY\n");
        // two transactions of the latest date: the one folded last
        let later = ledger(&format!(
            "{OPEN}2024-01-01 budget Food CNY\n{}{}{}",
            txn("2024-06-02", "a"),
            txn("2024-06-02", "b"),
            txn("2024-02-01", "c")
        ));
        let series = months(&later);
        let transactions = seqs(&later, "transaction");
        assert_eq!(series[0].until, month(2024, 6));
        assert_eq!(
            series[0].until_cause,
            Some(Cause {
                seq: *transactions.iter().max().unwrap()
            })
        );
        // a transaction in the budget's own last month does not replace it
        let same = ledger(&format!(
            "{OPEN}2024-01-01 budget Food CNY\n2024-01-01 budget-add Food 5 CNY\n{}",
            txn("2024-01-20", "a")
        ));
        let series = months(&same);
        assert_eq!(series[0].until, month(2024, 1));
        assert_eq!(
            series[0].until_cause,
            Some(Cause {
                seq: seqs(&same, "budget-add")[0]
            })
        );
    }

    #[test]
    fn the_budgets_column_is_indexed_by_the_budgets_accounts() {
        let provider = &crate::extension::native_providers()[0];
        let column = crate::extension::LabelColumn::compile(
            &std::sync::Arc::from(provider.manifest.provider.as_str()),
            provider.digest(),
            provider.source,
            &provider.manifest.labels[0],
        )
        .unwrap();
        let (indexed, param) = column.index.as_ref().expect("an index");
        assert_eq!((indexed.name(), param.to_string()), ("account", ":accounts".to_owned()));
    }

    #[test]
    fn the_facts_are_the_months_with_something() {
        let text = format!(
            "{OPEN}2024-01-15 budget Food CNY\n  alias: \"Groceries\"\n\
             2024-03-01 budget-add Food 100.00 CNY\n\
             2024-05-02 * \"lunch\"\n  Expenses:Food 30 CNY\n  Assets:Bank -30 CNY\n\
             2024-08-01 budget-close Food\n"
        );
        let series = months(&ledger(&text));
        let facts = &series[0].facts;
        assert_eq!(
            facts.iter().map(|fact| fact.period).collect::<Vec<_>>(),
            [month(2024, 1), month(2024, 3), month(2024, 5), month(2024, 8)]
        );
        let cells = |at: usize| facts[at].cells.iter().map(ToString::to_string).collect::<Vec<_>>();
        // name, alias, category, currency, date, year, month, assigned, added, activity, available, accounts, closed
        assert_eq!(cells(1)[7..11], ["100.00 CNY", "100.00 CNY", "0 CNY", "100.00 CNY"]);
        assert_eq!(cells(2)[7..11], ["100.00 CNY", "0 CNY", "30 CNY", "70.00 CNY"]);
        assert_eq!(cells(3)[10..], ["70.00 CNY", "Expenses:Food", "TRUE"]);
        assert_eq!(cells(0)[..4], ["Food", "Groceries", "NULL", "CNY"]);
        assert!(facts.iter().all(|fact| fact.meta.as_ref().is_some_and(|meta| meta.get_one("alias").is_some())));
    }
}

/// The manifest of the native provider as an analyzer declares it, which the example analyzer of `zhang-plugin-sdk`
/// standing in for `zhang:budget` reads (`zhang-plugin-sdk/examples/budget/manifest.json`): the same tables, columns,
/// descriptions, predicate and named queries, so that the catalog it gives a ledger is the native one, served from its
/// snapshot.
#[cfg(test)]
mod as_an_analyzer {
    use serde_json::{json, Value};

    use super::*;

    fn fill(fill: &Option<Fill>) -> Value {
        match fill {
            None => Value::Null,
            Some(Fill::Carry) => json!("carry"),
            Some(Fill::Zero) => json!("zero"),
            Some(Fill::Previous(column)) => json!({ "previous": column }),
        }
    }

    fn table(decl: &TableDecl) -> Value {
        let columns = decl
            .columns
            .iter()
            .map(|column| {
                let mut value = json!({"name": column.name, "type": column.ty.name(), "description": column.description});
                if column.fill.is_some() {
                    value["fill"] = fill(&column.fill);
                }
                value
            })
            .collect::<Vec<_>>();
        let mut value = json!({
            "name": decl.published_as, "kind": "records", "description": decl.description, "columns": columns, "wildcard": decl.wildcard,
        });
        match &decl.kind {
            TableKind::Records(RecordOrder::Published) => value["order"] = json!("published"),
            TableKind::Records(RecordOrder::Columns(columns)) => value["order"] = json!(columns),
            TableKind::Series(series) => {
                value["kind"] = json!("series");
                let derive = series
                    .derive
                    .iter()
                    .map(|(column, part)| {
                        let part = match part {
                            DatePart::Year => "year",
                            DatePart::Month => "month",
                        };
                        (column.clone(), json!(part))
                    })
                    .collect::<serde_json::Map<_, _>>();
                value["period"] = json!({"column": series.period, "unit": "month", "derive": derive});
                value["partition"] = json!(series.partition);
                value["extend_to_today"] = json!(series.extend_to_today);
            }
        }
        value
    }

    fn query(query: &NamedQuery) -> Value {
        let params = query
            .params
            .iter()
            .map(|(name, ty)| json!({"name": name, "type": ty.name()}))
            .collect::<Vec<_>>();
        json!({"name": query.name, "description": query.description, "bql": query.bql, "params": params})
    }

    #[test]
    fn the_budget_analyzer_example_declares_the_native_tables() {
        let manifest = manifest();
        let labels = manifest
            .labels
            .iter()
            .map(|label| {
                let params = label
                    .params
                    .iter()
                    .map(|(name, ty)| (name.clone(), json!(ty.name())))
                    .collect::<serde_json::Map<_, _>>();
                json!({"table": label.table, "name": label.published_as, "description": label.description, "predicate": label.predicate, "params": params})
            })
            .collect::<Vec<_>>();
        let declared = json!({
            "analysis_abi": 1,
            "consumes": ["budget", "budget-add", "budget-transfer", "budget-close"],
            "open_meta": ["budget"],
            "diagnostics": [
                {"kind": "BudgetDoesNotExist"}, {"kind": "DefineDuplicatedBudget"}, {"kind": "BudgetClosed"},
                {"kind": "BudgetCommodityMismatch", "phase": "after_fold"}
            ],
            "tables": manifest.tables.iter().map(table).collect::<Vec<_>>(),
            "columns": labels,
            "queries": manifest.queries.iter().map(query).collect::<Vec<_>>(),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&declared).unwrap());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../zhang-plugin-sdk/examples/budget/manifest.json");
        if matches!(std::env::var("UPDATE_GOLDEN").as_deref(), Ok("1" | "true")) {
            std::fs::write(path, &text).unwrap();
        }
        assert_eq!(std::fs::read_to_string(path).unwrap_or_default(), text, "UPDATE_GOLDEN=1 rewrites {path}");
        // and it reads back as the native manifest, but for the order of the parts of a period and of the parameters
        // of a label, which a JSON object does not keep
        let read: zhang_shared::plugin_abi::analysis::Manifest = serde_json::from_str(&text).unwrap();
        let converted = super::super::abi::manifest(BUDGET_PROVIDER, &read, true).unwrap();
        let mut native = manifest.clone();
        for table in &mut native.tables {
            if let TableKind::Series(series) = &mut table.kind {
                series.derive.sort_by(|a, b| a.0.cmp(&b.0));
            }
        }
        for label in &mut native.labels {
            label.params.sort_by(|a, b| a.0.cmp(&b.0));
        }
        assert_eq!(format!("{:?}", converted), format!("{:?}", native));
    }

    const BUDGET_PROVIDER: &str = super::super::compat::BUDGET;
}
