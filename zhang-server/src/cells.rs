//! Reading the rows of a built-in query's result by column name, for the thin mappings of the
//! endpoints into their response types.
//!
//! A built-in query selects every column its mapping reads, so a column the result does not have
//! is a bug of the server. Every accessor answers it with [`ServerError::MissingColumn`], an HTTP
//! 500 that names the query and the column: never a panic, which drops the connection, and never
//! a silent NULL, which shows users empty fields. A cell that is NULL or of another type than
//! asked for is `None`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_ast::amount::Amount;
use zhang_query::{QueryResult, Value};

use crate::error::ServerError;
use crate::ServerResult;

/// A result's column indexes, shared by all its rows, with the name of the query that produced it.
#[derive(Clone)]
pub(crate) struct Columns(Arc<Inner>);

struct Inner {
    query: String,
    index: HashMap<String, usize>,
}

impl Columns {
    pub(crate) fn of(query: &str, result: &QueryResult) -> Self {
        Self(Arc::new(Inner {
            query: query.to_owned(),
            index: result.columns.iter().enumerate().map(|(index, column)| (column.name.clone(), index)).collect(),
        }))
    }

    /// The index of the column `name`; a [`ServerError::MissingColumn`] if the query has no such column.
    fn index(&self, name: &str) -> ServerResult<usize> {
        self.0.index.get(name).copied().ok_or_else(|| ServerError::MissingColumn {
            query: self.0.query.clone(),
            column: name.to_owned(),
        })
    }

    /// The cell of the column `name` of a row.
    pub(crate) fn get<'a>(&self, values: &'a [Value], name: &str) -> ServerResult<&'a Value> {
        Ok(&values[self.index(name)?])
    }

    /// Move the cell of the column `name` out of an owned row.
    pub(crate) fn take(&self, values: &mut [Value], name: &str) -> ServerResult<Value> {
        Ok(std::mem::replace(&mut values[self.index(name)?], Value::Null))
    }
}

/// One row of a query result.
pub(crate) struct Row<'a> {
    columns: Columns,
    values: &'a [Value],
}

/// The rows of the result of the built-in query `query`.
pub(crate) fn rows<'a>(query: &str, result: &'a QueryResult) -> impl Iterator<Item = Row<'a>> {
    let columns = Columns::of(query, result);
    result.rows.iter().map(move |values| Row {
        columns: columns.clone(),
        values,
    })
}

/// The first row of the result of the built-in query `query`, if it has one.
pub(crate) fn first_row<'a>(query: &str, result: &'a QueryResult) -> Option<Row<'a>> {
    rows(query, result).next()
}

impl Row<'_> {
    /// the cell of the column `name`
    pub(crate) fn get(&self, name: &str) -> ServerResult<&Value> {
        self.columns.get(self.values, name)
    }

    pub(crate) fn str(&self, name: &str) -> ServerResult<Option<String>> {
        Ok(match self.get(name)? {
            Value::Str(value) => Some(value.clone()),
            _ => None,
        })
    }

    pub(crate) fn int(&self, name: &str) -> ServerResult<Option<i64>> {
        Ok(match self.get(name)? {
            Value::Int(value) => Some(*value),
            _ => None,
        })
    }

    pub(crate) fn bool(&self, name: &str) -> ServerResult<Option<bool>> {
        Ok(match self.get(name)? {
            Value::Bool(value) => Some(*value),
            _ => None,
        })
    }

    pub(crate) fn decimal(&self, name: &str) -> ServerResult<Option<BigDecimal>> {
        Ok(match self.get(name)? {
            Value::Decimal(value) => Some(value.clone()),
            Value::Int(value) => Some(BigDecimal::from(*value)),
            _ => None,
        })
    }

    pub(crate) fn date(&self, name: &str) -> ServerResult<Option<NaiveDate>> {
        Ok(match self.get(name)? {
            Value::Date(value) => Some(*value),
            _ => None,
        })
    }

    pub(crate) fn amount(&self, name: &str) -> ServerResult<Option<Amount>> {
        Ok(match self.get(name)? {
            Value::Amount(value) => Some(value.clone()),
            _ => None,
        })
    }

    pub(crate) fn set(&self, name: &str) -> ServerResult<Option<BTreeSet<String>>> {
        Ok(match self.get(name)? {
            Value::Set(value) => Some(value.clone()),
            _ => None,
        })
    }

    /// the date of the column `date` at the time of day of the column `time` (`HH:MM:SS`), the
    /// way the engine writes a directive's date and time in the ledger's timezone
    pub(crate) fn datetime(&self, date: &str, time: &str) -> ServerResult<Option<NaiveDateTime>> {
        let time = self
            .str(time)?
            .and_then(|it| NaiveTime::parse_from_str(&it, "%H:%M:%S").ok())
            .unwrap_or(NaiveTime::MIN);
        Ok(self.date(date)?.map(|date| date.and_time(time)))
    }
}

#[cfg(test)]
mod test {
    use zhang_query::{ColumnInfo, DataType, QueryResult, Value};

    use super::{first_row, rows};
    use crate::error::ServerError;

    fn result() -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnInfo {
                    name: "account".to_owned(),
                    ty: DataType::Str,
                },
                ColumnInfo {
                    name: "units".to_owned(),
                    ty: DataType::Decimal,
                },
            ],
            rows: vec![vec![Value::Str("Assets:Bank".to_owned()), Value::Null]],
            total: None,
        }
    }

    #[test]
    fn a_column_the_query_does_not_have_is_an_error_naming_both() {
        let result = result();
        let row = first_row("accounts.list", &result).unwrap();
        assert_eq!(row.str("account").unwrap().as_deref(), Some("Assets:Bank"));
        // NULL, or another type than asked for, is none
        assert_eq!(row.decimal("units").unwrap(), None);
        assert_eq!(row.int("account").unwrap(), None);
        let error = row.str("balance").unwrap_err();
        assert!(matches!(&error, ServerError::MissingColumn { query, column } if query == "accounts.list" && column == "balance"));
        assert_eq!(error.to_string(), "the built-in query accounts.list has no column balance");
        assert_eq!(rows("accounts.list", &result).count(), 1);
    }
}
