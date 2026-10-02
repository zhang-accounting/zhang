//! Query parameters: `$1`, `$2`, ... (positional, 1-based) and `:name` (named).
//!
//! Internal callers bind typed [`Value`]s instead of formatting values into the query text.
//! Parameter types are fixed at compile time ([`ParamTypes`]) and checked again when a
//! compiled query is executed with concrete [`Params`].

use std::collections::BTreeMap;

use crate::value::{DataType, Value};

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
/// use chrono::NaiveDate;
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

    pub fn is_empty(&self) -> bool {
        self.positional.is_empty() && self.named.is_empty()
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
