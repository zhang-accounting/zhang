//! Query parameters: `$1`, `$2`, ... (positional, 1-based) and `:name` (named).
//!
//! Internal callers bind typed [`Value`]s instead of formatting values into the query text.
//! Parameter types are fixed at compile time ([`ParamTypes`]) and checked again when a
//! compiled query is executed with concrete [`Params`]. A caller that gets its values as JSON,
//! such as the HTTP API's built-in queries or a plugin, reads them with [`from_json`] and
//! [`bind_json`].

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{Datelike, NaiveDate};
use zhang_ast::amount::{number_in_range, MAX_NUMBER_SIZE};

use crate::decimal::to_plain_string;
use crate::value::{parse_date, DataType, Interval, Value};

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

/// What a query's parameters are bound to: a [`Value`] each to execute it ([`Params`]), or a
/// [`DataType`] each to compile it ([`ParamTypes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings<T> {
    positional: Vec<T>,
    named: BTreeMap<String, T>,
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
pub type Params = Bindings<Value>;

/// The declared types of query parameters.
pub type ParamTypes = Bindings<DataType>;

impl<T> Default for Bindings<T> {
    fn default() -> Self {
        Self {
            positional: vec![],
            named: BTreeMap::new(),
        }
    }
}

impl<T> Bindings<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the next positional parameter (`$1`, then `$2`, ...).
    pub fn push(mut self, value: impl Into<T>) -> Self {
        self.positional.push(value.into());
        self
    }

    /// Bind a named parameter (`:name`).
    pub fn bind(mut self, name: impl Into<String>, value: impl Into<T>) -> Self {
        self.named.insert(name.into(), value.into());
        self
    }

    pub fn get(&self, param: &ParamRef) -> Option<&T> {
        match param {
            ParamRef::Positional(idx) => idx.checked_sub(1).and_then(|idx| self.positional.get(idx)),
            ParamRef::Named(name) => self.named.get(name),
        }
    }

    pub fn len(&self) -> usize {
        self.positional.len() + self.named.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Params {
    /// The types of the bound values, for [`crate::Query::compile_with_params`].
    pub fn types(&self) -> ParamTypes {
        ParamTypes {
            positional: self.positional.iter().map(Value::data_type).collect(),
            named: self.named.iter().map(|(name, value)| (name.clone(), value.data_type())).collect(),
        }
    }
}

/// The types a parameter given as JSON can have: the engine's types a JSON value can give,
/// named `bool`, `int`, `decimal`, `str`, `date` and `set` ([`DataType::name`]).
pub const JSON_TYPES: [DataType; 6] = [DataType::Bool, DataType::Int, DataType::Decimal, DataType::Str, DataType::Date, DataType::Set];

/// Why a JSON value is not a parameter's value ([`from_json`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonError {
    /// the value has the wrong shape; what the type expects instead, such as `"a date string
    /// YYYY-MM-DD or null"`
    Expected(&'static str),
    /// a decimal beyond [`MAX_NUMBER_SIZE`], the bound of every number zhang reads: it would be
    /// compared, summed or written out with as many digits as its exponent says
    OutOfRange,
}

impl JsonError {
    /// the error for the parameter `name`: `parameter :from must be a date string YYYY-MM-DD or
    /// null`, or `parameter :min is out of range`
    pub fn for_param(self, name: &str) -> String {
        match self {
            JsonError::Expected(expected) => format!("parameter :{name} must be {expected}"),
            JsonError::OutOfRange => format!("parameter :{name} is out of range"),
        }
    }
}

/// what a `decimal` parameter expects
const DECIMAL_EXPECTED: &str = "a number, a decimal string such as \"12.50\", or null";

/// A parameter's value of type `ty` from JSON, or why it is not one ([`JsonError`]). `null` is
/// NULL for any type; otherwise a `bool` takes a boolean, an `int` an integer, a `decimal` an
/// integer, a number or a string such as `"12.50"`, a `str` a string, a `date` a string
/// `YYYY-MM-DD` read as BQL's `date(text)` reads it ([`parse_date`]), and a `set` a list of
/// strings. A decimal is bounded as a number literal in a ledger is ([`MAX_NUMBER_SIZE`] digits,
/// and an exponent within it either way); beyond that it is out of range.
///
/// ```
/// use serde_json::json;
/// use zhang_query::params::{from_json, JsonError};
/// use zhang_query::{DataType, Value};
///
/// assert_eq!(from_json(DataType::Str, &json!("x")), Ok(Value::Str("x".into())));
/// assert_eq!(from_json(DataType::Int, &json!(null)), Ok(Value::Null));
/// assert_eq!(from_json(DataType::Decimal, &json!("12.50")).unwrap().to_string(), "12.50");
/// assert_eq!(from_json(DataType::Date, &json!(20240101)), Err(JsonError::Expected("a date string YYYY-MM-DD or null")));
/// assert_eq!(from_json(DataType::Decimal, &json!("1e100000000")), Err(JsonError::OutOfRange));
/// ```
pub fn from_json(ty: DataType, value: &serde_json::Value) -> Result<Value, JsonError> {
    use serde_json::Value as Json;
    if value.is_null() {
        return Ok(Value::Null);
    }
    let parsed = match (ty, value) {
        (DataType::Bool, Json::Bool(it)) => Some(Value::Bool(*it)),
        (DataType::Int, Json::Number(it)) => it.as_i64().map(Value::Int),
        (DataType::Decimal, Json::Number(it)) => match it.as_i64() {
            Some(int) => Some(Value::Decimal(int.into())),
            // the shortest text that reads back as the same double, the number as it was written
            None => it.as_f64().map(|float| decimal(&float.to_string())).transpose()?,
        },
        (DataType::Decimal, Json::String(it)) => Some(decimal(it)?),
        (DataType::Str, Json::String(it)) => Some(Value::Str(it.clone())),
        // the engine's one text-to-date rule, as `date(text)` reads it
        (DataType::Date, Json::String(it)) => parse_date(it).map(Value::Date),
        (DataType::Set, Json::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<BTreeSet<String>>>()
            .map(Value::Set),
        _ => None,
    };
    parsed.ok_or(JsonError::Expected(match ty {
        DataType::Bool => "a boolean or null",
        DataType::Int => "an integer or null",
        DataType::Decimal => DECIMAL_EXPECTED,
        DataType::Str => "a string or null",
        DataType::Date => "a date string YYYY-MM-DD or null",
        DataType::Set => "a list of strings or null",
        _ => "of a type a JSON value cannot give",
    }))
}

/// `text` as a decimal, trimmed, within [`MAX_NUMBER_SIZE`]. The digits are counted before the
/// text is parsed, so that a text of millions of digits is refused as quickly as `1e100000000`,
/// which parses in no time but would be compared or written out with as many digits as its
/// exponent says.
fn decimal(text: &str) -> Result<Value, JsonError> {
    let text = text.trim();
    if text.bytes().filter(u8::is_ascii_digit).count() as u64 > MAX_NUMBER_SIZE {
        return Err(JsonError::OutOfRange);
    }
    let number = BigDecimal::from_str(text).map_err(|_| JsonError::Expected(DECIMAL_EXPECTED))?;
    if !number_in_range(&number) {
        return Err(JsonError::OutOfRange);
    }
    Ok(Value::Decimal(number))
}

/// Named parameters given as JSON, `(name, type name, value)` each, as a plugin gives them to
/// `zhang_query_params`: the declared types, to compile the query with
/// ([`crate::Query::compile_with_params`]), and the values, to execute it with. The type is
/// one of [`JSON_TYPES`] by name and the value is read by [`from_json`]. An error names the
/// parameter: `parameter :from must be a date string YYYY-MM-DD or null`, `parameter :min is
/// out of range`, or `parameter :x has the type "money", which is not one of bool, int,
/// decimal, str, date, set`.
///
/// ```
/// use serde_json::json;
/// use zhang_query::params::bind_json;
/// use zhang_query::{DataType, ParamRef, Value};
///
/// let (types, values) = bind_json([("from", "date", &json!("2024-01-01")), ("re", "str", &json!(null))]).unwrap();
/// assert_eq!(types.get(&ParamRef::Named("from".into())), Some(&DataType::Date));
/// assert_eq!(values.get(&ParamRef::Named("re".into())), Some(&Value::Null));
/// ```
pub fn bind_json<'a>(params: impl IntoIterator<Item = (&'a str, &'a str, &'a serde_json::Value)>) -> Result<(ParamTypes, Params), String> {
    let mut types = ParamTypes::new();
    let mut values = Params::new();
    for (name, type_name, value) in params {
        let ty = JSON_TYPES.into_iter().find(|ty| ty.name() == type_name).ok_or_else(|| {
            let names = JSON_TYPES.map(|ty| ty.name()).join(", ");
            format!("parameter :{name} has the type {type_name:?}, which is not one of {names}")
        })?;
        let value = from_json(ty, value).map_err(|error| error.for_param(name))?;
        types = types.bind(name, ty);
        values = values.bind(name, value);
    }
    Ok((types, values))
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
    use serde_json::json;

    use super::{bind_json, from_json, to_bql, JsonError, JSON_TYPES};
    use crate::value::{DataType, Interval, Value};
    use crate::ParamRef;

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

    #[test]
    fn json_values_bind_with_the_declared_types() {
        let ok = |ty, value| from_json(ty, &value).unwrap();
        let expected = |ty, value| match from_json(ty, &value) {
            Err(JsonError::Expected(expected)) => expected,
            other => panic!("{:?}", other),
        };
        assert_eq!(ok(DataType::Bool, json!(false)), Value::Bool(false));
        assert_eq!(ok(DataType::Int, json!(-4)), Value::Int(-4));
        assert_eq!(ok(DataType::Decimal, json!(12)).to_string(), "12");
        assert_eq!(ok(DataType::Decimal, json!(12.5)).to_string(), "12.5");
        assert_eq!(ok(DataType::Decimal, json!(0.1)).to_string(), "0.1");
        assert_eq!(ok(DataType::Decimal, json!(12.0)).to_string(), "12");
        assert_eq!(ok(DataType::Decimal, json!(u64::MAX)).to_string(), "18446744073709552000");
        assert_eq!(ok(DataType::Decimal, json!(" 12.50 ")).to_string(), "12.50");
        assert_eq!(ok(DataType::Str, json!("x")), Value::Str("x".into()));
        assert_eq!(ok(DataType::Date, json!("2024-02-29")).to_string(), "2024-02-29");
        assert_eq!(ok(DataType::Set, json!(["b", "a", "b"])).to_string(), "a, b");
        for ty in JSON_TYPES {
            assert_eq!(from_json(ty, &json!(null)).unwrap(), Value::Null);
        }
        assert_eq!(expected(DataType::Int, json!(1.5)), "an integer or null");
        assert_eq!(expected(DataType::Int, json!(12.0)), "an integer or null");
        assert_eq!(expected(DataType::Int, json!(u64::MAX)), "an integer or null");
        assert_eq!(expected(DataType::Int, json!("1")), "an integer or null");
        assert_eq!(expected(DataType::Date, json!("2024-02-30")), "a date string YYYY-MM-DD or null");
        assert_eq!(expected(DataType::Date, json!("2024-02-01T00:00:00Z")), "a date string YYYY-MM-DD or null");
        assert_eq!(expected(DataType::Date, json!(20240101)), "a date string YYYY-MM-DD or null");
        assert_eq!(expected(DataType::Set, json!("a")), "a list of strings or null");
        assert_eq!(expected(DataType::Set, json!(["a", 1])), "a list of strings or null");
        assert_eq!(expected(DataType::Bool, json!(1)), "a boolean or null");
        assert_eq!(
            expected(DataType::Decimal, json!("twelve")),
            "a number, a decimal string such as \"12.50\", or null"
        );
        assert_eq!(expected(DataType::Str, json!(1)), "a string or null");
        assert_eq!(expected(DataType::Amount, json!("1 CNY")), "of a type a JSON value cannot give");
    }

    /// A `decimal` parameter is bounded as a number literal in a ledger is: a huge exponent, which parses
    /// in no time but is then compared or written out with as many digits as it says, and a huge digit
    /// count are both refused, quickly, and the error names the parameter.
    #[test]
    fn a_decimal_parameter_is_bounded_as_a_literal_is() {
        let started = std::time::Instant::now();
        assert_eq!(from_json(DataType::Decimal, &json!("1e100000000")), Err(JsonError::OutOfRange));
        assert_eq!(from_json(DataType::Decimal, &json!("1E-100000000")), Err(JsonError::OutOfRange));
        assert_eq!(from_json(DataType::Decimal, &json!("1e1000000")), Err(JsonError::OutOfRange));
        let digits = "9".repeat(1_000_000);
        assert_eq!(from_json(DataType::Decimal, &json!(digits)), Err(JsonError::OutOfRange));
        assert_eq!(from_json(DataType::Decimal, &json!(format!("0.{digits}"))), Err(JsonError::OutOfRange));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "refused in {:?}", started.elapsed());
        // the bound itself still reads, as it does in a ledger
        assert!(from_json(DataType::Decimal, &json!("1e999999")).is_ok());
        assert!(from_json(DataType::Decimal, &json!("1e-999999")).is_ok());
        // exponents a double can carry stay within it
        assert!(from_json(DataType::Decimal, &json!(1e300)).is_ok());
        assert!(from_json(DataType::Decimal, &json!(5e-324)).is_ok());
        let huge = json!("1e100000000");
        assert_eq!(bind_json([("min", "decimal", &huge)]).unwrap_err(), "parameter :min is out of range");
    }

    /// A `date` parameter given as text binds exactly the texts BQL's `date(text)` reads (beanquery's
    /// `strptime(text, '%Y-%m-%d')`), as the same day; no other reading of text as a date.
    #[test]
    fn a_date_parameter_reads_text_as_bql_date_reads_it() {
        let bind = |text: &str| from_json(DataType::Date, &json!(text)).ok();
        let day = Some(Value::Date(NaiveDate::from_ymd_opt(2024, 2, 9).unwrap()));
        for text in ["2024-02-09", "2024-2-9", "2024-02- 9"] {
            assert_eq!(bind(text), day, "{:?}", text);
            assert_eq!(crate::value::parse_date(text).map(Value::Date), day, "{:?}", text);
        }
        for text in [" 2024-02-09", "+2024-02-09", "24-02-09", "2024-02-09 ", "0000-02-09", "2024-02-30"] {
            assert_eq!(bind(text), None, "{:?}", text);
        }
    }

    #[test]
    fn json_params_bind_by_name_with_their_declared_types() {
        let from = json!("2024-01-01");
        let min = json!(12.5);
        let tags = json!(["a", "b"]);
        let null = json!(null);
        let (types, values) = bind_json([("from", "date", &from), ("min", "decimal", &min), ("tags", "set", &tags), ("re", "str", &null)]).unwrap();
        let named = |name: &str| ParamRef::Named(name.to_owned());
        assert_eq!(types.len(), 4);
        assert_eq!(types.get(&named("from")), Some(&DataType::Date));
        assert_eq!(types.get(&named("min")), Some(&DataType::Decimal));
        assert_eq!(types.get(&named("tags")), Some(&DataType::Set));
        // a NULL keeps its declared type, so the query compiles against it
        assert_eq!(types.get(&named("re")), Some(&DataType::Str));
        assert_eq!(values.get(&named("from")).map(ToString::to_string), Some("2024-01-01".to_owned()));
        assert_eq!(values.get(&named("min")).map(ToString::to_string), Some("12.5".to_owned()));
        assert_eq!(values.get(&named("tags")).map(ToString::to_string), Some("a, b".to_owned()));
        assert_eq!(values.get(&named("re")), Some(&Value::Null));
        assert!(bind_json([]).unwrap().0.is_empty());

        let error = |params: [(&str, &str, &serde_json::Value); 1]| bind_json(params).unwrap_err();
        assert_eq!(error([("from", "date", &min)]), "parameter :from must be a date string YYYY-MM-DD or null");
        assert_eq!(
            error([("x", "money", &min)]),
            "parameter :x has the type \"money\", which is not one of bool, int, decimal, str, date, set"
        );
        // the engine's other types have no JSON form
        assert_eq!(
            error([("x", "amount", &min)]),
            "parameter :x has the type \"amount\", which is not one of bool, int, decimal, str, date, set"
        );
    }
}
