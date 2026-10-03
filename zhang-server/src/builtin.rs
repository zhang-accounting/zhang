//! Built-in queries: the named, documented BQL behind the figures the app shows (#479).
//!
//! Interim version for the budgets and commodities slice, with the interface of Track I's module:
//! a [`BuiltinQuery`] per query in the [`BUILTINS`] registry, compiled once, and [`execute`],
//! which runs one by name inside [`with_ledger`], the path of every query the server runs.
//! Track I's module replaces it.

use std::collections::HashMap;
use std::sync::LazyLock;

use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, ParamTypes, Params, Query, QueryResult};

use crate::error::ServerError;
pub use crate::routes::query::with_ledger;
use crate::routes::query::{execute_options, max_result_values};
use crate::ServerResult;

/// A named BQL query the app runs to compute what it shows.
#[derive(Debug)]
pub struct BuiltinQuery {
    /// unique, dotted and lower case, e.g. `budgets.month`
    pub name: &'static str,
    /// one sentence on what the query returns
    pub description: &'static str,
    /// the query, with its parameters written `:name`
    pub bql: &'static str,
    /// every parameter the query uses, with its type
    pub params: &'static [(&'static str, DataType)],
}

impl BuiltinQuery {
    /// The declared types of the parameters, to compile the query with.
    pub fn param_types(&self) -> ParamTypes {
        self.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, *ty))
    }
}

/// Every built-in query, grouped by the endpoints that run it.
pub static BUILTINS: &[BuiltinQuery] = &[
    // ---- budgets and commodities: /api/budgets/*, /api/commodities/* ----
    BuiltinQuery {
        name: "budgets.month",
        description: "Every budget as of a month (its first day): its last month in #budgets up to that month.",
        bql: "SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, last(assigned) AS assigned, last(activity) AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE date <= :month \
              GROUP BY name \
              ORDER BY name",
        params: &[("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.budget",
        description: "One budget: its display name, category, commodity, and the accounts whose postings are its activity.",
        bql: "SELECT name, first(alias) AS alias, first(category) AS category, first(currency) AS currency, \
              first(accounts) AS accounts \
              FROM #budgets \
              WHERE name = :name \
              GROUP BY name",
        params: &[("name", DataType::Str)],
    },
    BuiltinQuery {
        name: "budgets.budget_month",
        description: "One budget as of a month (its first day), as in budgets.month: its last month in #budgets up to that month.",
        bql: "SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, last(assigned) AS assigned, last(activity) AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE name = :name AND date <= :month \
              GROUP BY name",
        params: &[("name", DataType::Str), ("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.events",
        description: "What the budget-add and budget-transfer directives put into a budget in a month (its first day), newest first.",
        bql: "SELECT date, time, timestamp, type, amount \
              FROM #budget_events \
              WHERE name = :name AND type != 'close' AND yearmonth(date) = :month \
              ORDER BY timestamp DESC",
        params: &[("name", DataType::Str), ("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.postings",
        description: "The postings of a budget's accounts in a month (its first day), newest first, with each account's balance after them.",
        bql: "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units, \
              only(currency, account_balance) AS balance \
              WHERE account IN :accounts AND yearmonth(date) = :month \
              ORDER BY timestamp DESC",
        params: &[("accounts", DataType::Set), ("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "commodities.totals",
        description: "How many units of each commodity the Assets and Liabilities accounts hold, for the commodities they hold.",
        bql: "SELECT currency, sum(number) AS total \
              WHERE root(account, 1) IN ('Assets', 'Liabilities') \
              GROUP BY currency \
              HAVING sum(number) != 0 \
              ORDER BY currency",
        params: &[],
    },
    BuiltinQuery {
        name: "commodities.total",
        description: "How many units of a commodity the Assets and Liabilities accounts hold; no row if they hold none.",
        bql: "SELECT currency, sum(number) AS total \
              WHERE currency = :commodity AND root(account, 1) IN ('Assets', 'Liabilities') \
              GROUP BY currency \
              HAVING sum(number) != 0",
        params: &[("commodity", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_prices",
        description: "The latest price of each commodity quoted in a currency, with its date and time.",
        bql: "SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price \
              FROM #prices \
              WHERE currency(amount) = :currency \
              GROUP BY currency \
              ORDER BY currency",
        params: &[("currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_price",
        description: "The latest price of a commodity quoted in a currency, with its date and time.",
        bql: "SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price \
              FROM #prices \
              WHERE currency = :commodity AND currency(amount) = :currency \
              GROUP BY currency",
        params: &[("commodity", DataType::Str), ("currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.lots",
        description: "The lots of a commodity the Assets and Liabilities accounts hold, by account, then oldest first.",
        bql: "SELECT account, cost_date, cost_number, cost_currency, sum(number) AS units \
              WHERE currency = :commodity AND root(account, 1) IN ('Assets', 'Liabilities') \
              GROUP BY account, cost_date, cost_number, cost_currency \
              HAVING sum(number) != 0 \
              ORDER BY account, cost_date, cost_number",
        params: &[("commodity", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.prices",
        description: "Every price of a commodity, in any currency, oldest first.",
        bql: "SELECT date, time, amount \
              FROM #prices \
              WHERE currency = :commodity \
              ORDER BY date, time",
        params: &[("commodity", DataType::Str)],
    },
];

/// The built-in query named `name`.
pub fn get(name: &str) -> Option<&'static BuiltinQuery> {
    BUILTINS.iter().find(|query| query.name == name)
}

/// Every built-in query, compiled once, on first use.
static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    BUILTINS
        .iter()
        .map(|builtin| {
            // the tests compile every built-in query, so this cannot fail in a release
            let query = Query::compile_with_params(builtin.bql, &builtin.param_types())
                .unwrap_or_else(|err| panic!("the built-in query {} does not compile: {}", builtin.name, err));
            (builtin.name, query)
        })
        .collect()
});

/// The compiled built-in query named `name`.
pub fn compiled(name: &str) -> ServerResult<&'static Query> {
    COMPILED
        .get(name)
        .ok_or_else(|| ServerError::InvalidInput(format!("there is no built-in query {}", name)))
}

/// Execute the built-in query `name` on `ledger`, with the limits of every query the server
/// runs (the time limit and the result size limit of `POST /api/query`). With `count_total`
/// the result's `total` is the number of rows before `LIMIT` and `OFFSET`.
///
/// This blocks: call it inside [`with_ledger`], which also runs several queries under one read
/// lock.
pub fn execute(ledger: &Ledger, name: &str, params: &Params, count_total: bool) -> ServerResult<QueryResult> {
    let options = ExecuteOptions {
        count_total,
        ..execute_options(max_result_values())
    };
    Ok(compiled(name)?.execute_with_options(ledger, params, &options)?)
}

#[cfg(test)]
mod test {
    use super::{compiled, BUILTINS};

    #[test]
    fn every_builtin_compiles_and_declares_exactly_its_parameters() {
        for builtin in BUILTINS {
            let query = compiled(builtin.name).unwrap();
            assert_eq!(query.source(), builtin.bql);
            for (name, _) in builtin.params {
                assert!(
                    builtin.bql.contains(&format!(":{}", name)),
                    "{} declares :{} but does not use it",
                    builtin.name,
                    name
                );
            }
        }
        let mut names = BUILTINS.iter().map(|it| it.name).collect::<Vec<_>>();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILTINS.len(), "built-in names are unique");
    }
}
