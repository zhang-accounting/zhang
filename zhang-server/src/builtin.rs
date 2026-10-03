//! Built-in queries: the named, documented BQL behind the read endpoints (#479).
//!
//! Interim version for the budgets and commodities slice, with the interface of Track I's module:
//! [`BuiltinQuery`], the [`BUILTINS`] registry, compiled once, and [`run`], which runs a built-in
//! the way `/api/query` runs a query. Track I's module replaces it.

use std::collections::HashMap;
use std::sync::LazyLock;

use zhang_query::{DataType, ParamTypes, Params, Query, QueryResult};

use crate::routes::{budget, commodity};
use crate::state::SharedLedger;
use crate::ServerResult;

/// A named, documented BQL query that an endpoint runs. User input is only ever bound to its
/// parameters, never formatted into the text.
pub struct BuiltinQuery {
    pub name: &'static str,
    pub description: &'static str,
    pub bql: &'static str,
    /// the parameters of `bql`, by name, with their types
    pub params: &'static [(&'static str, DataType)],
}

impl BuiltinQuery {
    fn param_types(&self) -> ParamTypes {
        self.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, *ty))
    }
}

/// Every built-in query.
pub static BUILTINS: &[&BuiltinQuery] = &[
    &budget::BUDGETS_BY_MONTH,
    &budget::BUDGET,
    &budget::BUDGET_BY_MONTH,
    &budget::BUDGET_EVENTS_BY_MONTH,
    &budget::BUDGET_POSTINGS_BY_MONTH,
    &commodity::COMMODITY_TOTALS,
    &commodity::COMMODITY_TOTAL,
    &commodity::LATEST_PRICES,
    &commodity::LATEST_PRICE,
    &commodity::COMMODITY_LOTS,
    &commodity::COMMODITY_PRICES,
];

static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    BUILTINS
        .iter()
        .map(|builtin| {
            let query = Query::compile_with_params(builtin.bql, &builtin.param_types())
                .unwrap_or_else(|err| panic!("built-in query {} does not compile: {}", builtin.name, err));
            (builtin.name, query)
        })
        .collect()
});

/// Run a built-in query with `params` bound, under the ledger's read lock, the time limit and
/// the result size limit of `/api/query`, off the async workers.
pub async fn run(ledger: &SharedLedger, builtin: &'static BuiltinQuery, params: Params) -> ServerResult<QueryResult> {
    let ledger = ledger.0.clone().read_owned().await;
    let options = crate::routes::query::execute_options(crate::routes::query::max_result_values());
    let result = tokio::task::spawn_blocking(move || COMPILED[builtin.name].execute_with_options(&ledger, &params, &options)).await??;
    Ok(result)
}

#[cfg(test)]
mod test {
    use super::{BUILTINS, COMPILED};

    #[test]
    fn every_builtin_compiles_and_declares_exactly_its_parameters() {
        for builtin in BUILTINS {
            let query = &COMPILED[builtin.name];
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
