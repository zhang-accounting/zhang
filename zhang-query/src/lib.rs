//! A BQL-compatible query engine for zhang.
//!
//! Queries are parsed with `nom`, type-checked into a plan, and evaluated directly over
//! the ledger's in-memory store (no copy into another database). Numbers are exact
//! [`bigdecimal::BigDecimal`]s and amounts always keep their currency.
//!
//! The crate has no HTTP or JSON concerns: results are typed [`Value`]s. JSON encoding
//! lives in `zhang-server`.

// the compiler/evaluator that uses the crate-internal registry helpers lands in the next commit
#![allow(dead_code)]

pub mod decimal;
pub mod error;
pub mod functions;
pub mod params;
pub mod prices;
pub mod value;

use zhang_core::ledger::Ledger;

pub use zhang_ast::amount::Amount;

pub use crate::error::{QueryError, QueryErrorKind};
pub use crate::params::{ParamRef, ParamTypes, Params};
pub use crate::prices::PriceMap;
pub use crate::value::{Cost, DataType, Inventory, Position, Value};

/// Name and static type of a result column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub ty: DataType,
}

/// The typed result of a query.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
}

/// A compiled query: parse and type-check once, execute many times.
pub struct Query {
    #[allow(dead_code)]
    source: String,
}

impl Query {
    /// Compile a query without parameters.
    pub fn compile(query: &str) -> Result<Query, QueryError> {
        Query::compile_with_params(query, &ParamTypes::default())
    }

    /// Compile a query whose `$n` / `:name` parameters have the given types.
    pub fn compile_with_params(_query: &str, _params: &ParamTypes) -> Result<Query, QueryError> {
        Err(QueryError::new(QueryErrorKind::Compile, "the query engine is not implemented yet"))
    }

    /// Execute against a ledger.
    pub fn execute(&self, _ledger: &Ledger, _params: &Params) -> Result<QueryResult, QueryError> {
        Err(QueryError::new(QueryErrorKind::Eval, "the query engine is not implemented yet"))
    }
}

/// Compile and execute a query without parameters.
pub fn execute(ledger: &Ledger, query: &str) -> Result<QueryResult, QueryError> {
    Query::compile(query)?.execute(ledger, &Params::default())
}

/// Compile and execute a query, binding `params`.
pub fn execute_with_params(ledger: &Ledger, query: &str, params: &Params) -> Result<QueryResult, QueryError> {
    Query::compile_with_params(query, &params.types())?.execute(ledger, params)
}
