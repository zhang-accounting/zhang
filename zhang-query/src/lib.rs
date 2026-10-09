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
#[cfg(test)]
mod equivalence_tests;
pub mod error;
mod executor;
pub mod export;
pub mod functions;
mod optimizer;
pub mod params;
mod parser;
mod period;
mod pivot;
pub mod prices;
mod projector;
mod running;
mod statements;
pub mod table;
pub mod value;

use std::time::Duration;

use chrono::NaiveDate;
pub use zhang_ast::amount::Amount;
use zhang_core::ledger::Ledger;

pub use crate::error::{QueryError, QueryErrorKind};
pub use crate::params::{Bindings, ParamRef, ParamTypes, Params};
pub use crate::parser::{MAX_DEPTH, MAX_NAME_PARTS, MAX_QUERY_LENGTH};
pub use crate::prices::PriceMap;
pub use crate::value::{Cost, DataType, Interval, Inventory, Metas, Position, Value};

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
    /// The number of rows the query has before `LIMIT` and `OFFSET` (and before `PIVOT BY`
    /// reshapes them), when [`ExecuteOptions::count_total`] asks for it; `None` otherwise.
    pub total: Option<u64>,
}

/// A compiled query: parse and type-check once, execute many times.
pub struct Query {
    source: String,
    plan: compiler::Plan,
    /// the columns the plan reads: executions only build these parts of the rows
    projection: projector::Projection,
    /// whether an execution turns the bound parameters into constants first
    /// ([`optimizer::bind`]); only the naive reference of the tests does not
    bind_params: bool,
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
        let mut plan = optimizer::optimize(plan).map_err(|err| err.resolve(query))?;
        projector::plan_running(&mut plan);
        let projection = projector::project(&plan);
        Ok(Query {
            source: query.to_owned(),
            plan,
            projection,
            bind_params: true,
        })
    }

    /// Compile without the execution decisions of the optimizer and the projector: every
    /// expression is evaluated for every row (parameters, `IN` lists and string tests
    /// included, as they are written) and LIMIT and OFFSET apply to the sorted rows. Tests
    /// compare it with [`Query::compile_with_params`], which must return the same results.
    #[cfg(test)]
    pub(crate) fn compile_naive(query: &str, params: &ParamTypes) -> Result<Query, QueryError> {
        let select = parser::parse(query)?;
        let plan = compiler::compile(query, &select, params).map_err(|err| err.resolve(query))?;
        let plan = optimizer::optimize_naive(plan).map_err(|err| err.resolve(query))?;
        let projection = projector::project(&plan);
        Ok(Query {
            source: query.to_owned(),
            plan,
            projection,
            bind_params: false,
        })
    }

    /// The query text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// An `EXPLAIN`-style description of the optimized plan: the table (unless it is the
    /// default `postings`), one line per target, aggregate, the filter, grouping, HAVING,
    /// ordering, limit and pivot, then the projected columns of the table (the only ones an
    /// execution computes).
    pub fn explain(&self) -> String {
        format!("{}project: {}\n", self.plan, self.projection)
    }

    /// The columns of the query's [table](Query::table) it reads, in name order.
    pub fn referenced_columns(&self) -> Vec<&'static str> {
        self.plan.referenced_columns().into_iter().collect()
    }

    /// The name of the table the query reads (`FROM #name`), `postings` by default.
    pub fn table(&self) -> &'static str {
        self.plan.table.name
    }

    /// The result columns.
    ///
    /// The columns of a `PIVOT BY` query depend on the data: this lists the columns before
    /// the pivot, and [`QueryResult::columns`] of each execution the pivoted ones.
    pub fn columns(&self) -> Vec<ColumnInfo> {
        self.plan.targets[..self.plan.visible]
            .iter()
            .map(|target| ColumnInfo {
                name: target.name.clone(),
                ty: target.ty,
            })
            .collect()
    }

    /// Execute against a ledger, with `today()` read from the ledger's clock in its timezone
    /// ([`Ledger::today`]: the system clock, unless the ledger was loaded with a fixed one), the
    /// [`DEFAULT_TIMEOUT`] and the [`DEFAULT_MAX_RESULT_VALUES`].
    ///
    /// The system clock is missing on some targets (e.g. `wasm32-unknown-unknown`); use
    /// [`Query::execute_at`] there.
    pub fn execute(&self, ledger: &Ledger, params: &Params) -> Result<QueryResult, QueryError> {
        self.execute_with_options(
            ledger,
            params,
            &ExecuteOptions {
                today: None,
                timeout: Some(DEFAULT_TIMEOUT),
                max_result_values: Some(DEFAULT_MAX_RESULT_VALUES),
                count_total: false,
            },
        )
    }

    /// Execute against a ledger with a fixed date for `today()`, no time limit and the
    /// [`DEFAULT_MAX_RESULT_VALUES`]. It never reads the clock.
    pub fn execute_at(&self, ledger: &Ledger, params: &Params, today: NaiveDate) -> Result<QueryResult, QueryError> {
        self.execute_with_options(
            ledger,
            params,
            &ExecuteOptions {
                today: Some(today),
                timeout: None,
                max_result_values: Some(DEFAULT_MAX_RESULT_VALUES),
                count_total: false,
            },
        )
    }

    /// The parameters the query uses, each once, in the order they first appear in its text,
    /// with the types they were compiled with.
    pub fn params(&self) -> Vec<(ParamRef, DataType)> {
        let mut uses = self.plan.params.iter().collect::<Vec<_>>();
        uses.sort_by_key(|(_, _, span)| span.start);
        let mut params: Vec<(ParamRef, DataType)> = vec![];
        for (param, ty, _) in uses {
            if !params.iter().any(|(seen, _)| seen == param) {
                params.push((param.clone(), *ty));
            }
        }
        params
    }

    /// The query's text with every parameter replaced by its value in `params`, written as BQL
    /// by [`params::to_bql`]: a query that runs without parameters, e.g. to show it to a user
    /// who can run it over HTTP, where parameters cannot be bound. Compiling the text and
    /// executing it gives the same result as executing this query with `params`, except for
    /// the name of a target written without `AS`, which is its text and now shows the value.
    ///
    /// The parameters must be bound with the types the query was compiled with, as for an
    /// execution, and a value BQL cannot write (an amount, position, inventory or metadata, or
    /// a date outside the years 1 to 9999) is an error. Where the grammar takes a literal
    /// rather than an expression (the pattern of `JOURNAL`, the dates of `OPEN ON` and
    /// `CLOSE ON`, the counts of `LIMIT` and `OFFSET`), a value that [`params::to_bql`] writes
    /// as an expression does not compile: a string with both kinds of quotes, or a negative
    /// count.
    pub fn inline_params(&self, params: &Params) -> Result<String, QueryError> {
        self.check_params(params)?;
        // every use the compiler recorded, in text order; a use it visited twice is one
        let mut uses = self.plan.params.iter().map(|(param, _, span)| (*span, param)).collect::<Vec<_>>();
        uses.sort_by_key(|(span, _)| span.start);
        uses.dedup_by_key(|(span, _)| span.start);
        let mut text = String::with_capacity(self.source.len());
        let mut copied = 0;
        for (span, param) in uses {
            let located = |message: String| error::LocatedError::compile(message, span).resolve(&self.source);
            let value = params.get(param).ok_or_else(|| located(format!("parameter {} is not bound", param)))?;
            let literal =
                params::to_bql(value).ok_or_else(|| located(format!("parameter {} is of type {}, which has no BQL literal", param, value.data_type())))?;
            text.push_str(&self.source[copied..span.start]);
            text.push_str(&literal);
            copied = span.end;
        }
        text.push_str(&self.source[copied..]);
        Ok(text)
    }

    /// Every parameter of the query is bound, to a value of its compiled type or NULL.
    fn check_params(&self, params: &Params) -> Result<(), QueryError> {
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
        Ok(())
    }

    /// Execute against a ledger with explicit [`ExecuteOptions`].
    pub fn execute_with_options(&self, ledger: &Ledger, params: &Params, options: &ExecuteOptions) -> Result<QueryResult, QueryError> {
        // start the clock before building the rows, which is part of the work
        let deadline = options.timeout.map(executor::Deadline::after);
        self.check_params(params)?;
        let window = self.plan.window(params).map_err(|err| err.resolve(&self.source))?;
        let period = self
            .plan
            .period
            .as_ref()
            .map(|period| period.resolve(params))
            .transpose()
            .map_err(|err| err.resolve(&self.source))?;
        // the parameters are constants of this execution: patterns, sets and needles bound to
        // them are prepared once, like literals
        let bound;
        let plan = if self.bind_params && !self.plan.params.is_empty() {
            bound = optimizer::bind(&self.plan, params);
            &bound
        } else {
            &self.plan
        };
        // the instant of this execution by the ledger's clock in its timezone: `today()` is its date and `now()` its
        // time of day; a fixed `today` makes it midnight of that date
        let now = match options.today {
            Some(today) => today.and_time(chrono::NaiveTime::MIN),
            None => ledger.now().with_timezone(&ledger.options.timezone).naive_local(),
        };
        let today = now.date();
        let cache = table::LedgerCache::of(ledger);
        let equity;
        let mut budget = executor::Budget::new(options.max_result_values);
        let data = match &period {
            None => {
                let scope = self.plan.execution.scope.as_ref().map(|scope| scope.resolve(params)).unwrap_or_default();
                let until = self.plan.execution.until.as_ref().map(|until| until.resolve(params, today));
                let mut limits = table::Limits::new(deadline.as_ref(), &mut budget);
                table::Dataset::build(ledger, now, self.projection, &scope, until, &mut limits).map_err(|err| err.resolve(&self.source))?
            }
            Some(period) => {
                equity = period::EquityAccounts::from_options(&ledger.options.values);
                let data = table::Dataset::postings(ledger, cache, now, self.projection, &table::Scope::All);
                period.apply(data, ledger, &equity)
            }
        };
        let run = executor::Run {
            window,
            count_total: options.count_total,
        };
        let output = executor::execute_within(plan, &data, params, deadline, budget, run).map_err(|err| err.resolve(&self.source))?;
        Ok(QueryResult {
            columns: output.columns.unwrap_or_else(|| self.columns()),
            rows: output.rows,
            total: output.total,
        })
    }
}

/// The time limit [`Query::execute`] applies.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The result size [`Query::execute`], [`Query::execute_at`] and [`ExecuteOptions::default`]
/// allow, in values: every cell is one value, and every position of a position or inventory
/// cell, element of a set, pair of a `metas` value and 64 bytes of text (also of those
/// elements and pairs) one more (see [`ExecuteOptions::max_result_values`]).
///
/// One million values is about five times the largest result of the fava demo ledger (its
/// whole `JOURNAL`: 3,209 rows whose running balances hold 178,576 positions, about 210,000
/// values), and bounds a result to roughly 400 MB in memory (an inventory position takes
/// about 370 bytes) and 120 MB of JSON.
pub const DEFAULT_MAX_RESULT_VALUES: u64 = 1_000_000;

/// Options of one execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteOptions {
    /// the date returned by `today()`, with `now()` the midnight of it; `None` reads the ledger's clock in its
    /// timezone ([`Ledger::today`]), whose date `today()` and time of day `now()` give
    pub today: Option<NaiveDate>,
    /// stop with a [`QueryErrorKind::Timeout`] error once the execution has run this long
    /// (checked every few hundred rows); `None` for no limit. A limit reads the monotonic
    /// clock, so leave it `None` on targets without one.
    pub timeout: Option<Duration>,
    /// stop with a [`QueryErrorKind::TooLarge`] error as soon as the execution would hold more
    /// than this many values of its result: one per cell, plus one per position of a position
    /// or inventory, per element of a set, per pair of a `metas` value and per 64 bytes of
    /// text (also of those elements and pairs). It covers the rows before
    /// ORDER BY, DISTINCT and LIMIT apply (a LIMIT without ORDER BY stops early) and the
    /// groups of an aggregate query while they are built. `None` for no limit.
    pub max_result_values: Option<u64>,
    /// also count the rows the query has before `LIMIT` and `OFFSET`, into
    /// [`QueryResult::total`]: the total for paging. Rows past the window are counted
    /// without being built (DISTINCT still tells them apart, and an aggregate query without
    /// ORDER BY collects the keys of the groups past the window).
    pub count_total: bool,
}

/// No time limit, today from the clock, the [`DEFAULT_MAX_RESULT_VALUES`] and no total.
impl Default for ExecuteOptions {
    fn default() -> Self {
        ExecuteOptions {
            today: None,
            timeout: None,
            max_result_values: Some(DEFAULT_MAX_RESULT_VALUES),
            count_total: false,
        }
    }
}

/// Compile and execute a query without parameters.
pub fn execute(ledger: &Ledger, query: &str) -> Result<QueryResult, QueryError> {
    Query::compile(query)?.execute(ledger, &Params::default())
}

/// Compile and execute a query, binding `params` (their types are taken from the values).
pub fn execute_with_params(ledger: &Ledger, query: &str, params: &Params) -> Result<QueryResult, QueryError> {
    Query::compile_with_params(query, &params.types())?.execute(ledger, params)
}

/// A column of a table, for documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDoc {
    pub name: &'static str,
    pub ty: DataType,
    pub description: &'static str,
}

/// A table a query can read with `FROM #name`, for documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDoc {
    /// the name, without `#`
    pub name: &'static str,
    pub description: &'static str,
    pub columns: Vec<ColumnDoc>,
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

/// The queryable tables, columns and functions, generated from the engine's registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// the columns of the `postings` table, the default table
    pub columns: Vec<ColumnDoc>,
    pub functions: Vec<FunctionDoc>,
    /// every table, `postings` first
    pub tables: Vec<TableDoc>,
    /// every word the parser reads as a keyword, lower case
    pub keywords: Vec<&'static str>,
}

fn column_docs(table: &table::Table) -> Vec<ColumnDoc> {
    table
        .columns
        .iter()
        .map(|column| ColumnDoc {
            name: column.name,
            ty: column.ty,
            description: column.description,
        })
        .collect()
}

/// Describe every table and every function overload.
pub fn schema() -> Schema {
    let columns = column_docs(&table::POSTINGS);
    let tables = table::tables()
        .iter()
        .map(|table| TableDoc {
            name: table.name,
            description: table.description,
            columns: column_docs(table),
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
        tables,
        keywords: parser::KEYWORDS.to_vec(),
    }
}
