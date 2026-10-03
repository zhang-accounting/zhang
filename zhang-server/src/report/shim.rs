//! Stand-ins for the shared built-in query helpers of Track I (`BuiltinQuery`, running a
//! built-in, `LedgerDateRange` and `calculated_amount`), with the same names and meaning, so
//! the report can switch to the shared ones by changing its imports.
//!
//! TODO(#479): replace with the shared module once it is on main.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Inventory, ParamTypes, Params, Query, QueryResult};

use crate::error::ServerError;
use crate::routes::query::{execute_options, max_result_values};
use crate::ServerResult;

/// A named, documented query that an endpoint runs, with its parameters and their types.
pub struct BuiltinQuery {
    pub name: &'static str,
    pub description: &'static str,
    pub bql: &'static str,
    pub params: &'static [(&'static str, DataType)],
}

impl BuiltinQuery {
    pub fn param_types(&self) -> ParamTypes {
        self.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, *ty))
    }
}

static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    super::QUERIES
        .iter()
        .map(|query| {
            let compiled = Query::compile_with_params(query.bql, &query.param_types())
                .unwrap_or_else(|error| panic!("built-in query {} does not compile: {}", query.name, error));
            (query.name, compiled)
        })
        .collect()
});

/// Run a built-in query over the ledger, under the time limit and the result size limit of
/// `/api/query`. The caller holds the ledger's read lock off the async workers.
pub fn run(ledger: &Ledger, query: &BuiltinQuery, params: &Params) -> ServerResult<QueryResult> {
    let compiled = COMPILED.get(query.name).expect("every report query is registered");
    Ok(compiled.execute_with_options(ledger, params, &execute_options(max_result_values()))?)
}

/// A report range in ledger dates, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerDateRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl LedgerDateRange {
    /// `from` and `to` as `YYYY-MM-DD` ledger dates, or as RFC 3339 instants, which are read as
    /// their date in the ledger's timezone.
    pub fn from_query(from: &str, to: &str, timezone: &Tz) -> ServerResult<LedgerDateRange> {
        Ok(LedgerDateRange {
            from: ledger_date(from, timezone)?,
            to: ledger_date(to, timezone)?,
        })
    }
}

fn ledger_date(value: &str, timezone: &Tz) -> ServerResult<NaiveDate> {
    if let Ok(date) = NaiveDate::from_str(value) {
        return Ok(date);
    }
    DateTime::parse_from_rfc3339(value)
        .map(|instant| instant.with_timezone(timezone).date_naive())
        .map_err(|_| ServerError::InvalidInput(format!("{:?} is neither a date (YYYY-MM-DD) nor an RFC 3339 instant", value)))
}

/// The response amount of an engine figure: `calculated` is the part of `value` in the
/// operating currency (what could not be converted is left out), `detail` the units per
/// currency.
pub fn calculated_amount(units: &Inventory, value: &Inventory, operating_currency: &str) -> CalculatedAmount {
    let total = value
        .positions()
        .filter(|position| position.units.commodity == operating_currency)
        .fold(BigDecimal::zero(), |total, position| total + position.units.number);
    let mut detail: HashMap<String, BigDecimal> = HashMap::new();
    for position in units.positions() {
        *detail.entry(position.units.commodity).or_insert_with(BigDecimal::zero) += position.units.number;
    }
    CalculatedAmount {
        calculated: Amount::new(total, operating_currency.to_owned()),
        detail,
    }
}
