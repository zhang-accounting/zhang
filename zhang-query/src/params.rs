//! Query parameters: `$1`, `$2`, ... (positional, 1-based) and `:name` (named).
//!
//! Internal callers bind typed [`Value`]s instead of formatting values into the query text.
//! Parameter types are fixed at compile time ([`ParamTypes`]) and checked again when a
//! compiled query is executed with concrete [`Params`].

use std::collections::BTreeMap;

use bigdecimal::BigDecimal;
use chrono::{Datelike, NaiveDate};

use crate::decimal::to_plain_string;
use crate::value::{DataType, Interval, Value};

/// A reference to a parameter inside the query text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamRef {
    /// `$n`, 1-based
    Positional(usize),
    /// `:name`
    Named(String),
}

impl std::fmt::Display for ParamRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamRef::Positional(idx) => write!(f, "${}", idx),
            ParamRef::Named(name) => write!(f, ":{}", name),
        }
    }
}

/// Values bound to the parameters of a query.
///
/// ```
/// use chrono::{Datelike, NaiveDate};
/// use zhang_query::Params;
///
/// let params = Params::new()
///     .bind("from", NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
///     .bind("to", NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
///     .push("^Expenses");
/// assert_eq!(params.types().len(), 3);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    positional: Vec<Value>,
    named: BTreeMap<String, Value>,
}

impl Params {
    pub fn new() -> Self {
        Params::default()
    }

    /// Append the next positional parameter (`$1`, then `$2`, ...).
    pub fn push(mut self, value: impl Into<Value>) -> Self {
        self.positional.push(value.into());
        self
    }

    /// Bind a named parameter (`:name`).
    pub fn bind(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.named.insert(name.into(), value.into());
        self
    }

    pub fn get(&self, param: &ParamRef) -> Option<&Value> {
        match param {
            ParamRef::Positional(idx) => idx.checked_sub(1).and_then(|idx| self.positional.get(idx)),
            ParamRef::Named(name) => self.named.get(name),
        }
    }

    /// The types of the bound values, for [`crate::Query::compile_with_params`].
    pub fn types(&self) -> ParamTypes {
        ParamTypes {
            positional: self.positional.iter().map(Value::data_type).collect(),
            named: self.named.iter().map(|(name, value)| (name.clone(), value.data_type())).collect(),
        }
    }
}

/// The declared types of query parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamTypes {
    positional: Vec<DataType>,
    named: BTreeMap<String, DataType>,
}

impl ParamTypes {
    pub fn new() -> Self {
        ParamTypes::default()
    }

    /// Declare the next positional parameter.
    pub fn push(mut self, ty: DataType) -> Self {
        self.positional.push(ty);
        self
    }

    /// Declare a named parameter.
    pub fn bind(mut self, name: impl Into<String>, ty: DataType) -> Self {
        self.named.insert(name.into(), ty);
        self
    }

    pub fn get(&self, param: &ParamRef) -> Option<DataType> {
        match param {
            ParamRef::Positional(idx) => idx.checked_sub(1).and_then(|idx| self.positional.get(idx)).copied(),
            ParamRef::Named(name) => self.named.get(name).copied(),
        }
    }

    pub fn len(&self) -> usize {
        self.positional.len() + self.named.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `value` written as BQL text that evaluates to it, as [`crate::Query::inline_params`] writes a
/// bound parameter into the query; `None` for amounts, positions, inventories and metadata,
/// which BQL has no way to write, and for a date outside the years 1 to 9999, which BQL dates
/// cannot reach.
///
/// The text keeps the type and the exact value: an integer stays an `int` and a decimal a
/// `decimal` of the same scale, a string is quoted so that it reads back unchanged whatever
/// quotes and backslashes it holds, and a set is a `set(...)` call. Negative numbers and
/// expressions are in parentheses, so the text can replace a parameter anywhere an
/// expression can be.
///
/// ```
/// use zhang_query::params::to_bql;
/// use zhang_query::Value;
///
/// assert_eq!(to_bql(&Value::Str("it's".into())).unwrap(), r#""it's""#);
/// assert_eq!(to_bql(&Value::Str(r#"it's "x""#.into())).unwrap(), r#"("it's " + '"x"')"#);
/// assert_eq!(to_bql(&Value::Int(-3)).unwrap(), "(-3)");
/// assert_eq!(to_bql(&Value::Set(["a".to_owned(), "b".to_owned()].into())).unwrap(), "set('a', 'b')");
/// assert_eq!(to_bql(&Value::Null).unwrap(), "NULL");
/// ```
pub fn to_bql(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => "NULL".to_owned(),
        Value::Bool(it) => if *it { "TRUE" } else { "FALSE" }.to_owned(),
        Value::Int(it) => int_to_bql(*it),
        Value::Decimal(it) => decimal_to_bql(it),
        Value::Str(it) => str_to_bql(it),
        Value::Date(it) => date_to_bql(*it)?,
        Value::Set(it) => format!("set({})", it.iter().map(|item| str_to_bql(item)).collect::<Vec<_>>().join(", ")),
        Value::Interval(it) => interval_to_bql(it),
        Value::Amount(_) | Value::Position(_) | Value::Inventory(_) | Value::Metas(_) => return None,
    })
}

fn int_to_bql(value: i64) -> String {
    if value == i64::MIN {
        // its magnitude does not fit in an int, so `-9223372036854775808` would be a decimal
        format!("({} - 1)", i64::MIN + 1)
    } else if value < 0 {
        // in parentheses, so a minus written before it cannot make a `--` comment
        format!("({})", value)
    } else {
        value.to_string()
    }
}

fn decimal_to_bql(value: &BigDecimal) -> String {
    let mut text = to_plain_string(value);
    if !text.contains('.') {
        // `12` would be an int; `12.` is the decimal 12, with the same scale of 0
        text.push('.');
    }
    if text.starts_with('-') {
        format!("({})", text)
    } else {
        text
    }
}

/// A BQL string has no escapes: it runs from its quote to the next one of the same kind, and
/// may hold the other kind and backslashes as they are. A text with both kinds of quotes is
/// written as a concatenation of strings that each hold one kind.
fn str_to_bql(text: &str) -> String {
    let quote = |chunk: &str, has_single: bool| {
        if has_single {
            format!("\"{}\"", chunk)
        } else {
            format!("'{}'", chunk)
        }
    };
    let mut parts = vec![];
    let mut chunk = String::new();
    let (mut has_single, mut has_double) = (false, false);
    for c in text.chars() {
        if (c == '\'' && has_double) || (c == '"' && has_single) {
            parts.push(quote(&chunk, has_single));
            chunk.clear();
            (has_single, has_double) = (false, false);
        }
        has_single |= c == '\'';
        has_double |= c == '"';
        chunk.push(c);
    }
    parts.push(quote(&chunk, has_single));
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        format!("({})", parts.join(" + "))
    }
}

/// A date literal has a four-digit year, and BQL's dates, Python's, run from the year 1 to
/// 9999, so a date outside them has no BQL text.
fn date_to_bql(date: NaiveDate) -> Option<String> {
    (1..=9999).contains(&date.year()).then(|| date.format("%Y-%m-%d").to_string())
}

fn interval_to_bql(interval: &Interval) -> String {
    let part = |number: i64, unit: &str| format!("interval('{} {}')", number, unit);
    match (interval.months, interval.days) {
        (0, days) => part(days, "day"),
        (months, 0) => part(months, "month"),
        (months, days) => format!("({} + {})", part(months, "month"), part(days, "day")),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;

    use super::to_bql;
    use crate::value::{Interval, Value};

    fn bql(value: impl Into<Value>) -> Option<String> {
        to_bql(&value.into())
    }

    #[test]
    fn values_are_written_as_bql() {
        assert_eq!(bql(Value::Null).unwrap(), "NULL");
        assert_eq!(bql(true).unwrap(), "TRUE");
        assert_eq!(bql(false).unwrap(), "FALSE");
        assert_eq!(bql(7i64).unwrap(), "7");
        assert_eq!(bql(-7i64).unwrap(), "(-7)");
        assert_eq!(bql(i64::MIN).unwrap(), "(-9223372036854775807 - 1)");
        for (number, written) in [("1.50", "1.50"), ("-1.50", "(-1.50)"), ("3", "3."), ("1E+2", "100."), ("0.000", "0.000")] {
            assert_eq!(bql(BigDecimal::from_str(number).unwrap()).unwrap(), written, "{}", number);
        }
        assert_eq!(bql("plain").unwrap(), "'plain'");
        assert_eq!(bql("").unwrap(), "''");
        assert_eq!(bql("O'Brien").unwrap(), "\"O'Brien\"");
        assert_eq!(bql(r#"say "hi""#).unwrap(), r#"'say "hi"'"#);
        assert_eq!(bql(r"C:\new\").unwrap(), r"'C:\new\'");
        assert_eq!(bql(r#"a'b"c'd"#).unwrap(), r#"("a'b" + '"c' + "'d")"#);
        assert_eq!(bql(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()).unwrap(), "2024-02-29");
        assert_eq!(bql(NaiveDate::from_ymd_opt(12, 1, 1).unwrap()).unwrap(), "0012-01-01");
        assert_eq!(bql(NaiveDate::from_ymd_opt(10000, 1, 1).unwrap()), None);
        let set = |items: &[&str]| Value::Set(items.iter().map(|it| it.to_string()).collect());
        assert_eq!(bql(set(&[])).unwrap(), "set()");
        assert_eq!(bql(set(&["b", "a'"])).unwrap(), "set(\"a'\", 'b')");
        assert_eq!(bql(Value::Interval(Interval::new(1, 0))).unwrap(), "interval('1 month')");
        assert_eq!(bql(Value::Interval(Interval::new(0, -3))).unwrap(), "interval('-3 day')");
        assert_eq!(bql(Value::Interval(Interval::new(0, 0))).unwrap(), "interval('0 day')");
        assert_eq!(
            bql(Value::Interval(Interval::new(14, 2))).unwrap(),
            "(interval('14 month') + interval('2 day'))"
        );
        assert_eq!(bql(Value::Amount(crate::Amount::new(BigDecimal::from(1), "USD"))), None);
        assert_eq!(bql(Value::Metas(vec![])), None);
    }
}
