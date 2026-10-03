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
//!   form a gap-free monthly series. The series runs from the budget's first month through
//!   the later of the budget's own last month (its last `budget-add`, `budget-transfer`,
//!   `budget-close` or spending) and the ledger's last month with a transaction: the budget
//!   page is about money coming in and going out, so the series follows the transactions, and
//!   a budget planned ahead with a `budget-add` in a future month shows that month too. Other
//!   directives, such as prices, events or notes, do not extend it, so a date typo on one of
//!   them cannot add centuries of months. The rows only depend on the ledger, not on the
//!   current date; a later month looks like the last row carried over, with nothing spent.
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

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Months, NaiveDate};
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Meta, Spanned};
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::store::{BudgetDomain, BudgetIntervalDetail, Store};

use super::{ledger_file, ColumnDef, Limits, Record, Rows, Table};
use crate::error::{LocatedError, QueryErrorKind};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static BUDGETS: Table = Table {
    name: "budgets",
    description: "One row per budget per month, from the budget's first month through its last entry or the ledger's last month with a transaction, whichever is later, with the assigned, activity and available amounts the budget pages show; ordered by name, then month.",
    columns: COLUMNS,
    wildcard: &["name", "date", "assigned", "activity", "available"],
    rows: Rows::Generated(rows),
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

/// The month series of one budget.
struct Series<'a> {
    budget: &'a BudgetDomain,
    /// the months zhang recorded a detail for, in order
    details: Vec<(NaiveDate, &'a BudgetIntervalDetail)>,
    end: NaiveDate,
    /// the directive whose date sets `end`
    end_by: Option<&'a Spanned<Directive>>,
    /// the first month the budget is closed in; `Some(None)` for a close without a date
    closed_from: Option<Option<NaiveDate>>,
}

impl Series<'_> {
    fn first(&self) -> NaiveDate {
        self.details[0].0
    }

    fn months(&self) -> u64 {
        let index = |month: NaiveDate| i64::from(month.year()) * 12 + i64::from(month.month0());
        u64::try_from(index(self.end) - index(self.first()) + 1).unwrap_or(0)
    }
}

fn rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection, limits: &mut Limits<'_>) -> Result<Vec<Record<'a>>, LocatedError> {
    // the ledger's last transaction, and the directives of each budget: the metadata of its
    // `budget` directive, the date of its close and its latest directive
    let mut last_transaction: Option<(NaiveDate, &Spanned<Directive>)> = None;
    let mut metas: HashMap<&str, &Meta> = HashMap::new();
    let mut close_dates: HashMap<&str, NaiveDate> = HashMap::new();
    let mut latest: HashMap<&str, (NaiveDate, &Spanned<Directive>)> = HashMap::new();
    let mut seen = |name: &'a str, date: NaiveDate, directive: &'a Spanned<Directive>| {
        if latest.get(name).is_none_or(|(it, _)| *it <= date) {
            latest.insert(name, (date, directive));
        }
    };
    for directive in &ledger.directives {
        match &directive.data {
            Directive::Transaction(transaction) => {
                let date = transaction.date.naive_date();
                if last_transaction.is_none_or(|(it, _)| it <= date) {
                    last_transaction = Some((date, directive));
                }
            }
            Directive::Budget(budget) => {
                metas.entry(budget.name.as_str()).or_insert(&budget.meta);
                seen(&budget.name, budget.date.naive_date(), directive);
            }
            Directive::BudgetAdd(add) => seen(&add.name, add.date.naive_date(), directive),
            Directive::BudgetTransfer(transfer) => {
                seen(&transfer.from, transfer.date.naive_date(), directive);
                seen(&transfer.to, transfer.date.naive_date(), directive);
            }
            Directive::BudgetClose(close) => {
                let date = close.date.naive_date();
                close_dates.entry(close.name.as_str()).and_modify(|it| *it = (*it).min(date)).or_insert(date);
                seen(&close.name, date, directive);
            }
            _ => {}
        }
    }
    let last_transaction_month = last_transaction.map(|(date, _)| first_of_month(date));

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
    let all = budgets
        .into_iter()
        .filter_map(|budget| {
            let details = budget
                .detail
                .values()
                .filter_map(|detail| Some((month_of(detail.date)?, detail)))
                .collect::<Vec<_>>();
            let last = details.last()?.0;
            // zhang records a close, but not when; the `budget-close` directive has the date
            let closed_from = budget.closed.then(|| close_dates.get(budget.name.as_str()).map(|date| first_of_month(*date)));
            // the budget's own last month (its details cover the months of its `budget`,
            // `budget-add` and `budget-transfer` directives and of its spending), its close,
            // and the last month with a transaction
            let end = [Some(last), closed_from.flatten(), last_transaction_month].into_iter().flatten().max()?;
            let end_by = if last_transaction_month == Some(end) {
                last_transaction.map(|(_, directive)| directive)
            } else {
                latest.get(budget.name.as_str()).map(|(_, directive)| *directive)
            };
            Some(Series {
                budget,
                details,
                end,
                end_by,
                closed_from,
            })
        })
        .collect::<Vec<_>>();

    let mut records = vec![];
    for one in &all {
        let budget_accounts = Rc::new(accounts.remove(one.budget.name.as_str()).unwrap_or_default());
        let meta = metas.get(one.budget.name.as_str()).copied();
        let mut next = 0;
        let mut month = one.first();
        while month <= one.end {
            limits.row().map_err(|err| too_many_months(err, ledger, &all))?;
            while next < one.details.len() && one.details[next].0 <= month {
                next += 1;
            }
            records.push(Record::Budget(BudgetMonth {
                budget: one.budget,
                month,
                detail: one.details[next - 1].1,
                closed: one.closed_from.is_some_and(|from| from.is_none_or(|from| from <= month)),
                accounts: Rc::clone(&budget_accounts),
                meta,
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
fn too_many_months(err: LocatedError, ledger: &Ledger, all: &[Series<'_>]) -> LocatedError {
    if err.kind != QueryErrorKind::TooLarge {
        return err;
    }
    let total = all.iter().map(Series::months).sum::<u64>();
    let Some(longest) = all.iter().rev().max_by_key(|series| series.months()) else {
        return err;
    };
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    let cause = longest
        .end_by
        .and_then(|directive| {
            let kind = match &directive.data {
                Directive::Transaction(_) => "a transaction",
                Directive::Budget(_) => "its budget directive",
                Directive::BudgetAdd(_) => "a budget-add",
                Directive::BudgetTransfer(_) => "a budget-transfer",
                Directive::BudgetClose(_) => "a budget-close",
                _ => return None,
            };
            let date = directive.datetime()?.date();
            let file = directive
                .span
                .filename
                .as_deref()
                .map(|path| format!(" ({})", ledger_file(ledger, path).display()))
                .unwrap_or_default();
            Some(format!(" because of {} dated {}{}; check that date", kind, date, file))
        })
        .unwrap_or_default();
    LocatedError {
        kind: QueryErrorKind::TooLarge,
        message: format!(
            "the #budgets table would generate too many rows ({} months in all, more than the result size limit): \
             budget '{}' runs from {} until {}{}",
            total,
            longest.budget.name,
            month(longest.first()),
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
