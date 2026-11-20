//! Series tables: the rows the host generates per query from a snapshot's facts.
//!
//! A partition's series runs from its first fact through its end, by period (a month). The end is decided per query,
//! with fixed tie rules, which `#budgets` has always had (each pinned by #835's `months*` cases):
//!
//! - the provider sets `until`, with the entry that sets it ([`Partition::until_cause`]): for `#budgets`, the
//!   budget's own last month (of its records of the latest date, the one folded last) or, when it is strictly later,
//!   the month of the ledger's last transaction (of the latest date, the one folded last);
//! - the current period replaces it when it is strictly later and the table extends its series to today
//!   ([`SeriesSpec::extend_to_today`]);
//! - the query's date bound ([`crate::optimizer::date_bound`]) replaces it when it is strictly earlier.
//!
//! A period with a fact is that fact's row; any other period is generated from the last fact before it by the fill
//! policies of the columns ([`crate::extension::Fill`]), which only copy values. Every row is charged to the query's
//! limits before it is built ([`Limits::row`]), so a series too long for the result stops with an error that names
//! the partition with the most periods (the first in table order among equals) and what sets its end
//! ([`super::compat::too_long`]).

use chrono::{Datelike, Months, NaiveDate};
use zhang_ast::Meta;
use zhang_core::ledger::Ledger;

use super::snapshot::{Cause, FactRow, Partition};
use super::{compat, PeriodUnit};
use crate::catalog::TableRef;
use crate::error::{LocatedError, QueryErrorKind};
use crate::table::{CellRead, FillRead, Generation, Limits, Record};
use crate::value::{first_of_month, month_index, Value};

/// How the host serves a series table.
#[derive(Debug, Clone)]
pub(crate) struct SeriesSpec {
    /// the index of the period column among the table's columns
    pub period: usize,
    pub unit: PeriodUnit,
    /// the indexes of the partition columns
    pub partition: Vec<usize>,
    pub extend_to_today: bool,
}

/// One row of a series table: a fact of the snapshot, or a period after it that the host generates from it.
#[derive(Clone, Copy)]
pub(crate) struct SeriesRow<'a> {
    /// the fact of the period, or the last fact before it
    fact: &'a FactRow,
    /// the first day of the period
    period: NaiveDate,
    /// whether the period is generated from `fact` rather than being its own
    generated: bool,
}

impl<'a> SeriesRow<'a> {
    /// The metadata of the row: its fact's, which the periods generated from it keep.
    pub fn metadata(&self) -> Option<&'a Meta> {
        self.fact.meta.as_deref()
    }

    /// The cell `read` reads.
    pub fn cell(&self, read: CellRead) -> Value {
        let cell = |index: u32| self.fact.cells.get(index as usize).cloned().unwrap_or(Value::Null);
        match read.fill {
            FillRead::Period => Value::Date(self.period),
            FillRead::Year => Value::Int(i64::from(self.period.year())),
            FillRead::Month => Value::Int(i64::from(self.period.month())),
            _ if !self.generated => cell(read.index),
            FillRead::Value | FillRead::Carry => cell(read.index),
            FillRead::Zero => zero_like(&cell(read.index)),
            FillRead::Previous(source) => cell(source),
        }
    }
}

/// Zero, in the commodity of `value` when it is an amount; NULL stays NULL.
fn zero_like(value: &Value) -> Value {
    match value {
        Value::Amount(amount) => Value::Amount(zhang_ast::amount::Amount::new(bigdecimal::BigDecimal::from(0), amount.commodity.clone())),
        Value::Decimal(_) => Value::Decimal(bigdecimal::BigDecimal::from(0)),
        Value::Int(_) => Value::Int(0),
        _ => Value::Null,
    }
}

/// What sets the last period of a series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndBy {
    /// the date of an entry: the provider's `until`
    Entry(Option<Cause>),
    /// the current period
    CurrentPeriod,
    /// the last period the query's filter keeps
    Query,
}

/// The extent of one partition's series in one query.
pub(crate) struct Extent<'a> {
    pub partition: &'a Partition,
    /// the first day of the first period
    pub first: NaiveDate,
    /// the first day of the last period
    pub end: NaiveDate,
    pub end_by: EndBy,
}

impl Extent<'_> {
    /// The number of periods.
    pub fn periods(&self) -> u64 {
        u64::try_from(month_index(self.end) - month_index(self.first) + 1).unwrap_or(0)
    }
}

/// The extent of every partition for an execution of `generation`.
fn extents<'a>(partitions: &'a [Partition], spec: &SeriesSpec, generation: Generation) -> Vec<Extent<'a>> {
    let PeriodUnit::Month = spec.unit;
    let current = first_of_month(generation.today);
    let bound = generation.until.map(first_of_month);
    partitions
        .iter()
        .filter_map(|partition| {
            let first = partition.facts.first()?.period;
            let mut extent = Extent {
                partition,
                first,
                end: partition.until,
                end_by: EndBy::Entry(partition.until_cause),
            };
            if spec.extend_to_today && current > extent.end {
                (extent.end, extent.end_by) = (current, EndBy::CurrentPeriod);
            }
            if let Some(bound) = bound.filter(|bound| *bound < extent.end) {
                (extent.end, extent.end_by) = (bound, EndBy::Query);
            }
            Some(extent)
        })
        .collect()
}

/// The rows of the series table `table` (of the provider `provider`) for one execution: every partition's periods in
/// order, each charged to `limits` before it is built.
pub(crate) fn rows<'a>(
    ledger: &Ledger, table: &TableRef, provider: &str, spec: &SeriesSpec, partitions: &'a [Partition], generation: Generation, limits: &mut Limits<'_>,
) -> Result<Vec<Record<'a>>, LocatedError> {
    let all = extents(partitions, spec, generation);
    let mut records = vec![];
    for extent in &all {
        let mut facts = extent.partition.facts.iter().peekable();
        let mut last = facts.next().expect("a partition with an extent has a fact");
        let mut period = extent.first;
        while period <= extent.end {
            limits.row().map_err(|err| too_long(err, ledger, table, provider, spec, &all))?;
            while let Some(next) = facts.next_if(|fact| fact.period <= period) {
                last = next;
            }
            records.push(Record::Series(SeriesRow {
                fact: last,
                period,
                generated: last.period != period,
            }));
            let Some(following) = period.checked_add_months(Months::new(1)) else {
                break;
            };
            period = following;
        }
    }
    Ok(records)
}

/// The error of a series too long for the result: its rows are generated before any filter runs, so the generic
/// advice to narrow the query cannot help. It names the partition with the most periods and what sets its end, which
/// is usually a date typo.
fn too_long(err: LocatedError, ledger: &Ledger, table: &TableRef, provider: &str, spec: &SeriesSpec, all: &[Extent<'_>]) -> LocatedError {
    if err.kind != QueryErrorKind::TooLarge {
        return err;
    }
    let total = all.iter().map(Extent::periods).sum::<u64>();
    // the first of the longest, in table order
    let Some(longest) = all.iter().rev().max_by_key(|extent| extent.periods()) else {
        return err;
    };
    LocatedError {
        kind: QueryErrorKind::TooLarge,
        message: compat::too_long(ledger, table, provider, spec, total, longest),
        span: None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::executor::Budget;

    fn day(year: i32, month: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, 1).unwrap()
    }

    fn fact(period: NaiveDate, value: i64) -> FactRow {
        FactRow {
            period,
            cells: vec![Value::Str("p".to_owned()), Value::Null, Value::Int(value), Value::Int(value * 10)].into_boxed_slice(),
            meta: None,
        }
    }

    fn partition(facts: Vec<FactRow>, until: NaiveDate) -> Partition {
        Partition {
            facts,
            until,
            until_cause: Some(Cause { seq: 7 }),
            first_cause: Some(Cause { seq: 1 }),
        }
    }

    const SPEC: SeriesSpec = SeriesSpec {
        period: 1,
        unit: PeriodUnit::Month,
        partition: vec![],
        extend_to_today: true,
    };

    fn ends(partition: &Partition, spec: &SeriesSpec, today: NaiveDate, until: Option<NaiveDate>) -> (NaiveDate, EndBy) {
        let extents = extents(std::slice::from_ref(partition), spec, Generation { today, until });
        (extents[0].end, extents[0].end_by)
    }

    #[test]
    fn the_end_of_a_series_follows_the_tie_rules() {
        let series = partition(vec![fact(day(2024, 1), 1)], day(2024, 6));
        let cause = EndBy::Entry(Some(Cause { seq: 7 }));
        // the provider's end, unless the current month is strictly later
        assert_eq!(ends(&series, &SPEC, day(2024, 6), None), (day(2024, 6), cause));
        assert_eq!(ends(&series, &SPEC, NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(), None), (day(2024, 6), cause));
        assert_eq!(ends(&series, &SPEC, day(2024, 7), None), (day(2024, 7), EndBy::CurrentPeriod));
        // a table that does not extend its series to today keeps the provider's end
        let fixed = SeriesSpec {
            extend_to_today: false,
            ..SPEC
        };
        assert_eq!(ends(&series, &fixed, day(2030, 1), None), (day(2024, 6), cause));
        // the query's bound only when strictly earlier, as a month
        assert_eq!(ends(&series, &SPEC, day(2024, 1), Some(day(2024, 6))), (day(2024, 6), cause));
        assert_eq!(
            ends(&series, &SPEC, day(2024, 1), Some(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap())),
            (day(2024, 6), cause)
        );
        assert_eq!(ends(&series, &SPEC, day(2024, 1), Some(day(2024, 5))), (day(2024, 5), EndBy::Query));
        assert_eq!(ends(&series, &SPEC, day(2030, 1), Some(day(2024, 9))), (day(2024, 9), EndBy::Query));
        assert_eq!(ends(&series, &SPEC, day(2030, 1), Some(day(2031, 1))), (day(2030, 1), EndBy::CurrentPeriod));
    }

    fn generate(partitions: &[Partition], today: NaiveDate, until: Option<NaiveDate>, max: Option<u64>) -> Result<Vec<(NaiveDate, Vec<Value>)>, String> {
        let ledger = crate::extension::tests::ledger("");
        let table = crate::catalog::Catalog::core().postings().clone();
        let mut budget = Budget::new(max);
        let mut limits = Limits::new(None, &mut budget);
        let rows = rows(&ledger, &table, "test:series", &SPEC, partitions, Generation { today, until }, &mut limits).map_err(|err| err.message)?;
        let read = |index: u32, fill: FillRead| CellRead { index, fill };
        Ok(rows
            .iter()
            .map(|row| {
                let Record::Series(row) = row else { panic!("a series row") };
                let cells = [
                    read(0, FillRead::Carry),
                    read(2, FillRead::Zero),
                    read(3, FillRead::Carry),
                    read(3, FillRead::Previous(2)),
                ];
                (row.period, cells.into_iter().map(|it| row.cell(it)).collect())
            })
            .collect())
    }

    #[test]
    fn generated_periods_copy_the_last_fact_by_the_fill_policies() {
        let series = partition(vec![fact(day(2024, 1), 1), fact(day(2024, 3), 2)], day(2024, 4));
        let rows = generate(&[series], day(2024, 1), None, None).unwrap();
        let p = || Value::Str("p".to_owned());
        // a fact reads its own cells; a generated period carries the last fact's, zeroes in its commodity, and takes
        // the last fact's value of another column
        assert_eq!(
            rows,
            [
                (day(2024, 1), vec![p(), Value::Int(1), Value::Int(10), Value::Int(10)]),
                (day(2024, 2), vec![p(), Value::Int(0), Value::Int(10), Value::Int(1)]),
                (day(2024, 3), vec![p(), Value::Int(2), Value::Int(20), Value::Int(20)]),
                (day(2024, 4), vec![p(), Value::Int(0), Value::Int(20), Value::Int(2)]),
            ]
        );
        // the period and its parts are generated for every row
        let row = SeriesRow {
            fact: &fact(day(2024, 1), 1),
            period: day(2025, 11),
            generated: true,
        };
        let read = |fill| row.cell(CellRead { index: 1, fill });
        assert_eq!(
            [read(FillRead::Period), read(FillRead::Year), read(FillRead::Month)],
            [Value::Date(day(2025, 11)), Value::Int(2025), Value::Int(11)]
        );
    }

    #[test]
    fn a_zero_keeps_the_commodity_of_an_amount() {
        let amount = Value::Amount(zhang_ast::amount::Amount::new("12.50".parse().unwrap(), "CNY"));
        assert_eq!(zero_like(&amount).to_string(), "0 CNY");
        assert_eq!(zero_like(&Value::Null), Value::Null);
    }

    #[test]
    fn a_series_too_long_names_its_longest_partition_first_in_order() {
        let a = partition(vec![fact(day(2024, 1), 1)], day(2024, 3));
        let b = partition(vec![fact(day(2023, 11), 1)], day(2024, 1));
        let c = partition(vec![fact(day(2024, 1), 1)], day(2024, 12));
        // a and b have three months each, c twelve
        assert!(generate(&[a, b, c], day(2024, 1), None, Some(18)).is_ok());
        let a = partition(vec![fact(day(2024, 1), 1)], day(2024, 3));
        let b = partition(vec![fact(day(2023, 11), 1)], day(2024, 1));
        let message = generate(&[a, b], day(2024, 1), None, Some(5)).unwrap_err();
        assert!(
            message.starts_with("the #postings table would generate too many rows (6 months in all, more than the result size limit): "),
            "{message}"
        );
        assert!(message.contains("runs from 2024-01 until 2024-03"), "{message}");
    }
}
