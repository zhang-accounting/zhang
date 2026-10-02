//! The `budgets` table: one row per budget per month, with the figures the budget pages of
//! the web UI show for that month.
//!
//! zhang's budgets are envelopes: `budget-add` and `budget-transfer` put money in, postings to
//! the budget's accounts (those with a `budget: <name>` metadata entry) spend it, and what is
//! left carries over to the next month. A user asks a budgets table the questions the budget
//! page answers one month at a time, across months and budgets at once: how much is left in
//! each budget now, which budgets ran over in which months, how much was spent per category
//! this year, how much was put into a budget over a year. The columns follow from that.
//!
//! - **The UI's terms and figures.** `assigned`, `activity` and `available` are exactly what
//!   the budget page and `GET /api/budgets?year=&month=` show: `assigned` is what the month
//!   starts with (the available amount carried over from the previous month) plus what was
//!   added in the month, `activity` is what the budget's accounts spent in the month, and
//!   `available = assigned - activity` is what carries over. All three are amounts in the
//!   budget's commodity.
//! - **`added`, for sums over months.** Because `assigned` includes the carry-over, adding it
//!   up over months counts the same money several times. `added` is only what the month's
//!   `budget-add` and `budget-transfer` directives put in (a transfer out counts as negative),
//!   so `sum(added)` is how much was budgeted over a period, next to `sum(activity)`, what was
//!   spent. `assigned - added` is the carry-over.
//! - **Every month, not only months with entries.** The UI shows a budget for every month
//!   from its first one, carrying the available amount over months without entries, so the
//!   table does too: a query for one month lists every budget the UI lists, and a budget's rows
//!   form a gap-free monthly series. The series runs from the budget's first month through the
//!   ledger's last month, the month of its latest dated directive (or of the budget's latest
//!   entry, if later). The rows only depend on the ledger, not on the current date; a later
//!   month looks like the last row carried over, with nothing spent.
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

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Months, NaiveDate};
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Meta};
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::store::{BudgetDomain, BudgetIntervalDetail, Store};

use super::{ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static BUDGETS: Table = Table {
    name: "budgets",
    description: "One row per budget per month, from the budget's first month through the ledger's last month, with the assigned, activity and available amounts the budget pages show; ordered by name, then month.",
    columns: COLUMNS,
    wildcard: &["name", "date", "assigned", "activity", "available"],
    rows: Rows::Records(rows),
};

/// One month of a budget: a row of the `budgets` table.
pub(crate) struct BudgetMonth<'a> {
    budget: &'a BudgetDomain,
    /// the first day of the month
    month: NaiveDate,
    /// the month's detail as zhang recorded it, or, for a month without entries, the detail
    /// of the latest month before it, whose available amount carries over
    detail: &'a BudgetIntervalDetail,
    closed: bool,
    /// the budget's accounts; only collected when the `accounts` column is projected
    accounts: Rc<BTreeSet<String>>,
    /// the metadata of the `budget` directive
    meta: Option<&'a Meta>,
}

impl<'a> BudgetMonth<'a> {
    /// The metadata of the `budget` directive.
    pub(super) fn metadata(&self) -> Option<&'a Meta> {
        self.meta
    }

    /// Whether zhang recorded a detail for this very month (rather than carrying one over).
    fn recorded(&self) -> bool {
        self.detail.date == interval(self.month)
    }

    fn amount(&self, number: BigDecimal) -> Value {
        Value::Amount(Amount::new(number, self.budget.commodity.clone()))
    }

    /// What the month starts with plus what was added in it, as `budget_month_detail` and
    /// therefore the budget API compute it.
    fn assigned(&self) -> BigDecimal {
        if self.recorded() {
            self.detail.assigned_amount.number.clone()
        } else {
            &self.detail.assigned_amount.number - &self.detail.activity_amount.number
        }
    }

    fn activity(&self) -> BigDecimal {
        if self.recorded() {
            self.detail.activity_amount.number.clone()
        } else {
            BigDecimal::zero()
        }
    }

    fn added(&self) -> BigDecimal {
        if self.recorded() {
            self.detail.events.iter().map(|event| &event.amount.number).sum()
        } else {
            BigDecimal::zero()
        }
    }
}

/// zhang's key of a budget month: `year * 100 + month`.
fn interval(month: NaiveDate) -> u32 {
    month.year() as u32 * 100 + month.month()
}

/// The first day of the month of a `year * 100 + month` key.
fn month_of(interval: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt((interval / 100) as i32, interval % 100, 1)
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first day")
}

fn rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection) -> Vec<Record<'a>> {
    // the ledger's last month, and the `budget` and `budget-close` directives of each budget
    let mut last_date: Option<NaiveDate> = None;
    let mut metas: HashMap<&str, &Meta> = HashMap::new();
    let mut close_dates: HashMap<&str, NaiveDate> = HashMap::new();
    for directive in &ledger.directives {
        if let Some(datetime) = directive.datetime() {
            last_date = last_date.max(Some(datetime.date()));
        }
        match &directive.data {
            Directive::Budget(budget) => {
                metas.entry(budget.name.as_str()).or_insert(&budget.meta);
            }
            Directive::BudgetClose(close) => {
                let date = close.date.naive_date();
                close_dates.entry(close.name.as_str()).and_modify(|it| *it = (*it).min(date)).or_insert(date);
            }
            _ => {}
        }
    }
    let last_month = last_date.map(first_of_month);

    let mut accounts: HashMap<&str, BTreeSet<String>> = HashMap::new();
    if projects(projection, "accounts") {
        for meta in &store.metas {
            if meta.meta_type == MetaType::AccountMeta.as_ref() && meta.key == "budget" {
                accounts.entry(meta.value.as_str()).or_default().insert(meta.type_identifier.clone());
            }
        }
    }

    let mut budgets = store.budgets.values().collect::<Vec<_>>();
    budgets.sort_by(|a, b| a.name.cmp(&b.name));

    let mut records = vec![];
    for budget in budgets {
        let details = budget
            .detail
            .values()
            .filter_map(|detail| Some((month_of(detail.date)?, detail)))
            .collect::<Vec<_>>();
        let (Some((first, _)), Some((last, _))) = (details.first(), details.last()) else {
            continue;
        };
        let end = last_month.map_or(*last, |month| month.max(*last));
        // zhang records a close, but not when; the `budget-close` directive has the date
        let closed_from = budget.closed.then(|| close_dates.get(budget.name.as_str()).map(|date| first_of_month(*date)));
        let budget_accounts = Rc::new(accounts.remove(budget.name.as_str()).unwrap_or_default());
        let meta = metas.get(budget.name.as_str()).copied();

        let mut next = 0;
        let mut month = *first;
        while month <= end {
            while next < details.len() && details[next].0 <= month {
                next += 1;
            }
            records.push(Record::Budget(BudgetMonth {
                budget,
                month,
                detail: details[next - 1].1,
                closed: closed_from.is_some_and(|from| from.is_none_or(|from| from <= month)),
                accounts: Rc::clone(&budget_accounts),
                meta,
            }));
            month = month + Months::new(1);
        }
    }
    records
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
        budget_month(record).map_or(Value::Null, |it| Value::Str(it.budget.name.clone()))
    }),
    ColumnDef::record(
        "alias",
        DataType::Str,
        "Display name of the budget (its alias metadata), or NULL if it has none.",
        |_, record| budget_month(record).and_then(|it| it.budget.alias.clone()).map_or(Value::Null, Value::Str),
    ),
    ColumnDef::record(
        "category",
        DataType::Str,
        "Category the budget is grouped under (its category metadata), or NULL if it has none.",
        |_, record| budget_month(record).and_then(|it| it.budget.category.clone()).map_or(Value::Null, Value::Str),
    ),
    ColumnDef::record("currency", DataType::Str, "Commodity the budget is kept in.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| Value::Str(it.budget.commodity.clone()))
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
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(it.assigned())),
    ),
    ColumnDef::record(
        "added",
        DataType::Amount,
        "Amount the month's budget-add and budget-transfer directives put into the budget; a transfer out counts as negative.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(it.added())),
    ),
    ColumnDef::record("activity", DataType::Amount, "Amount the budget's accounts spent in the month.", |_, record| {
        budget_month(record).map_or(Value::Null, |it| it.amount(it.activity()))
    }),
    ColumnDef::record(
        "available",
        DataType::Amount,
        "Amount left at the end of the month (assigned minus activity); it carries over to the next month.",
        |_, record| budget_month(record).map_or(Value::Null, |it| it.amount(it.assigned() - it.activity())),
    ),
    ColumnDef::record(
        "accounts",
        DataType::Set,
        "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
        |_, record| budget_month(record).map_or(Value::Null, |it| Value::Set(it.accounts.as_ref().clone())),
    ),
    ColumnDef::record(
        "closed",
        DataType::Bool,
        "Whether the budget was closed (budget-close) in or before the month.",
        |_, record| budget_month(record).map_or(Value::Null, |it| Value::Bool(it.closed)),
    ),
];
