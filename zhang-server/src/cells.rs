//! Reading the rows of a built-in query's result by column name, for the thin mappings of the
//! endpoints into their response types.

use std::collections::BTreeSet;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_query::{QueryResult, Value};

/// One row of a query result.
pub(crate) struct Row<'a> {
    result: &'a QueryResult,
    values: &'a [Value],
}

/// The rows of a query result.
pub(crate) fn rows(result: &QueryResult) -> impl Iterator<Item = Row<'_>> {
    result.rows.iter().map(move |values| Row { result, values })
}

/// The first row of a query result, if it has one.
pub(crate) fn first_row(result: &QueryResult) -> Option<Row<'_>> {
    rows(result).next()
}

impl Row<'_> {
    /// the value of the column `name`. Built-in queries are tested to have the columns their
    /// mappings read, so a missing column is a bug
    pub(crate) fn get(&self, name: &str) -> &Value {
        let idx = self
            .result
            .columns
            .iter()
            .position(|column| column.name == name)
            .unwrap_or_else(|| panic!("the query result has no column {}", name));
        &self.values[idx]
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
