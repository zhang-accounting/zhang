//! A BQL-compatible query engine for zhang.
//!
//! Queries are parsed with `nom`, type-checked into a plan, and evaluated directly over
//! the ledger's in-memory store under its read lock, with no copy into another database.
//! Numbers are exact [`bigdecimal::BigDecimal`]s, and amounts always keep their currency.
//!
//! The crate has no HTTP or JSON concerns: results are typed [`Value`]s, and the JSON
//! encoding lives in `zhang-server`.
//!
//! # Compile once, execute many
//!
//! [`Query::compile_with_params`] parses and type-checks a query once. The compiled
//! [`Query`] is `Send + Sync` and can be stored and executed for every request with
//! different typed [`Params`]. Values are never formatted into the query text.
//!
//! ```no_run
//! use chrono::NaiveDate;
//! use zhang_core::ledger::Ledger;
//! use zhang_query::{DataType, ParamTypes, Params, Query, Value};
//!
//! # fn report(ledger: &Ledger) -> Result<(), zhang_query::QueryError> {
//! // monthly expenses per root account between two dates
//! let query = Query::compile_with_params(
//!     "SELECT year, month, root(account, 2) AS category, sum(position) AS total \
//!      WHERE account ~ '^Expenses' AND date >= :from AND date < :to \
//!      GROUP BY year, month, category ORDER BY year, month, category",
//!     &ParamTypes::new().bind("from", DataType::Date).bind("to", DataType::Date),
//! )?;
//!
//! let params = Params::new()
//!     .bind("from", NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
//!     .bind("to", NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
//! let result = query.execute(ledger, &params)?;
//!
//! for row in &result.rows {
//!     if let (Value::Str(category), Value::Inventory(total)) = (&row[2], &row[3]) {
//!         for position in total.positions() {
//!             println!("{} {}-{}: {}", category, row[0], row[1], position.units);
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod ast;
mod compiler;
pub mod decimal;
pub mod error;
mod executor;
pub mod functions;
mod optimizer;
pub mod params;
mod parser;
pub mod prices;
mod projector;
mod statements;
pub mod table;
pub mod value;

use std::time::Duration;

use chrono::{NaiveDate, Utc};
pub use zhang_ast::amount::Amount;
use zhang_core::ledger::Ledger;

pub use crate::error::{QueryError, QueryErrorKind};
pub use crate::params::{ParamRef, ParamTypes, Params};
pub use crate::parser::{MAX_DEPTH, MAX_QUERY_LENGTH};
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
    source: String,
    plan: compiler::Plan,
    /// the columns the plan reads: executions only build these parts of the rows
    projection: projector::Projection,
}

impl Query {
    /// Compile a query without parameters.
    pub fn compile(query: &str) -> Result<Query, QueryError> {
        Query::compile_with_params(query, &ParamTypes::default())
    }

    /// Compile a query whose `$n` / `:name` parameters have the given types.
    pub fn compile_with_params(query: &str, params: &ParamTypes) -> Result<Query, QueryError> {
        let select = parser::parse(query)?;
        let plan = compiler::compile(query, &select, params).map_err(|err| err.resolve(query))?;
        let plan = optimizer::optimize(plan).map_err(|err| err.resolve(query))?;
        let projection = projector::project(&plan);
        Ok(Query {
            source: query.to_owned(),
            plan,
            projection,
        })
    }

    /// The query text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// An `EXPLAIN`-style description of the optimized plan: one line per target, aggregate,
    /// the filter, grouping, ordering and limit, then the projected columns (the only ones
    /// an execution computes).
    pub fn explain(&self) -> String {
        format!("{}project: {}\n", self.plan, self.projection)
    }

    /// The `postings` columns the query reads, in name order.
    pub fn referenced_columns(&self) -> Vec<&'static str> {
        self.plan.referenced_columns().into_iter().collect()
    }

    /// The result columns.
    pub fn columns(&self) -> Vec<ColumnInfo> {
        self.plan.targets[..self.plan.visible]
            .iter()
            .map(|target| ColumnInfo {
                name: target.name.clone(),
                ty: target.ty,
            })
            .collect()
    }

    /// Execute against a ledger, with `today()` read from the system clock in the ledger's
    /// timezone and the [`DEFAULT_TIMEOUT`].
    ///
    /// This reads the system clock; on targets without one (e.g. `wasm32-unknown-unknown`)
    /// use [`Query::execute_at`].
    pub fn execute(&self, ledger: &Ledger, params: &Params) -> Result<QueryResult, QueryError> {
        self.execute_with_options(
            ledger,
            params,
            &ExecuteOptions {
                today: None,
                timeout: Some(DEFAULT_TIMEOUT),
            },
        )
    }

    /// Execute against a ledger with a fixed date for `today()` and no time limit. It never
    /// reads the clock.
    pub fn execute_at(&self, ledger: &Ledger, params: &Params, today: NaiveDate) -> Result<QueryResult, QueryError> {
        self.execute_with_options(
            ledger,
            params,
            &ExecuteOptions {
                today: Some(today),
                timeout: None,
            },
        )
    }

    /// Execute against a ledger with explicit [`ExecuteOptions`].
    pub fn execute_with_options(&self, ledger: &Ledger, params: &Params, options: &ExecuteOptions) -> Result<QueryResult, QueryError> {
        // start the clock before building the rows, which is part of the work
        let deadline = options.timeout.map(executor::Deadline::after);
        for (param, declared, span) in &self.plan.params {
            let located = |message: String| error::LocatedError::compile(message, *span).resolve(&self.source);
            match params.get(param) {
                None => return Err(located(format!("parameter {} is not bound", param))),
                Some(value) if !value.is_null() && value.data_type() != *declared => {
                    return Err(located(format!(
                        "parameter {} was compiled as {} but is bound to a {}",
                        param,
                        declared,
                        value.data_type()
                    )))
                }
                Some(_) => {}
            }
        }
        let today = options.today.unwrap_or_else(|| Utc::now().with_timezone(&ledger.options.timezone).date_naive());
        let store = ledger
            .store
            .read()
            .map_err(|_| QueryError::new(QueryErrorKind::Eval, "the ledger store is not readable"))?;
        let data = table::Dataset::new(ledger, &store, today, self.projection);
        let rows = executor::execute(&self.plan, &data, params, deadline).map_err(|err| err.resolve(&self.source))?;
        Ok(QueryResult { columns: self.columns(), rows })
    }
}

/// The time limit [`Query::execute`] applies.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Options of one execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecuteOptions {
    /// the date returned by `today()`; `None` reads the system clock in the ledger's timezone
    pub today: Option<NaiveDate>,
    /// stop with a [`QueryErrorKind::Timeout`] error once the execution has run this long
    /// (checked every few hundred rows); `None` for no limit. A limit reads the monotonic
    /// clock, so leave it `None` on targets without one.
    pub timeout: Option<Duration>,
}

/// Compile and execute a query without parameters.
pub fn execute(ledger: &Ledger, query: &str) -> Result<QueryResult, QueryError> {
    Query::compile(query)?.execute(ledger, &Params::default())
}

/// Compile and execute a query, binding `params` (their types are taken from the values).
pub fn execute_with_params(ledger: &Ledger, query: &str, params: &Params) -> Result<QueryResult, QueryError> {
    Query::compile_with_params(query, &params.types())?.execute(ledger, params)
}

/// A column of the `postings` table, for documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDoc {
    pub name: &'static str,
    pub ty: DataType,
    pub description: &'static str,
}

/// One function overload, for documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDoc {
    pub name: &'static str,
    /// e.g. `root(str, int) -> str`
    pub signature: String,
    pub description: &'static str,
    pub aggregate: bool,
}

/// The queryable columns and functions, generated from the engine's registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub columns: Vec<ColumnDoc>,
    pub functions: Vec<FunctionDoc>,
}

/// Describe the `postings` table and every function overload.
pub fn schema() -> Schema {
    let columns = table::COLUMNS
        .iter()
        .map(|column| ColumnDoc {
            name: column.name,
            ty: column.ty,
            description: column.description,
        })
        .collect();
    let aggregates = functions::aggregates::AGGREGATE_FUNCTIONS.iter().map(|function| FunctionDoc {
        name: function.name,
        signature: function.signature(),
        description: function.description,
        aggregate: true,
    });
    let scalars = functions::SCALAR_FUNCTIONS.iter().map(|function| FunctionDoc {
        name: function.name,
        signature: function.signature(),
        description: function.description,
        aggregate: false,
    });
    Schema {
        columns,
        functions: aggregates.chain(scalars).collect(),
    }
}
