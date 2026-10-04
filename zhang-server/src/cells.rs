//! Reading the rows of a built-in query's result by column name, for the thin mappings of the
//! endpoints into their response types.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_query::{QueryResult, Value};

/// A result's column indexes, shared by all its rows.
#[derive(Clone)]
pub(crate) struct Columns(Arc<HashMap<String, usize>>);

impl Columns {
    pub(crate) fn of(result: &QueryResult) -> Self {
        Self(Arc::new(
            result.columns.iter().enumerate().map(|(index, column)| (column.name.clone(), index)).collect(),
        ))
    }

    /// Built-in queries are tested to select every column their mappings read.
    pub(crate) fn get<'a>(&self, values: &'a [Value], name: &str) -> &'a Value {
        &values[*self.0.get(name).unwrap_or_else(|| panic!("the query result has no column {}", name))]
    }

    /// Move a cell out of an owned row; NULL if the column is absent.
    pub(crate) fn take(&self, values: &mut [Value], name: &str) -> Value {
        match self.0.get(name).and_then(|index| values.get_mut(*index)) {
            Some(value) => std::mem::replace(value, Value::Null),
            None => Value::Null,
        }
    }
}

/// One row of a query result.
pub(crate) struct Row<'a> {
    columns: Columns,
    values: &'a [Value],
}

/// The rows of a query result.
pub(crate) fn rows(result: &QueryResult) -> impl Iterator<Item = Row<'_>> {
    let columns = Columns::of(result);
    result.rows.iter().map(move |values| Row {
        columns: columns.clone(),
        values,
    })
}

/// The first row of a query result, if it has one.
pub(crate) fn first_row(result: &QueryResult) -> Option<Row<'_>> {
    rows(result).next()
}

impl Row<'_> {
    /// the value of the column `name`. Built-in queries are tested to have the columns their
    /// mappings read, so a missing column is a bug
    pub(crate) fn get(&self, name: &str) -> &Value {
        self.columns.get(self.values, name)
    }

    pub(crate) fn str(&self, name: &str) -> Option<String> {
        match self.get(name) {
            Value::Str(value) => Some(value.clone()),
            _ => None,
        }
    }

    pub(crate) fn int(&self, name: &str) -> Option<i64> {
        match self.get(name) {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name) {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn decimal(&self, name: &str) -> Option<BigDecimal> {
        match self.get(name) {
            Value::Decimal(value) => Some(value.clone()),
            Value::Int(value) => Some(BigDecimal::from(*value)),
            _ => None,
        }
    }

    pub(crate) fn date(&self, name: &str) -> Option<NaiveDate> {
        match self.get(name) {
            Value::Date(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn amount(&self, name: &str) -> Option<Amount> {
        match self.get(name) {
            Value::Amount(value) => Some(value.clone()),
            _ => None,
        }
    }

    pub(crate) fn set(&self, name: &str) -> Option<BTreeSet<String>> {
        match self.get(name) {
            Value::Set(value) => Some(value.clone()),
            _ => None,
        }
    }

    /// the date of the column `date` at the time of day of the column `time` (`HH:MM:SS`), the
    /// way the engine writes a directive's date and time in the ledger's timezone
    pub(crate) fn datetime(&self, date: &str, time: &str) -> Option<NaiveDateTime> {
        let time = self
            .str(time)
            .and_then(|it| NaiveTime::parse_from_str(&it, "%H:%M:%S").ok())
            .unwrap_or(NaiveTime::MIN);
        self.date(date).map(|date| date.and_time(time))
    }
}
