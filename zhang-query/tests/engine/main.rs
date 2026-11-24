//! The behaviour tests of the query engine, one module per area: BQL semantics on inline ledgers
//! and on `integration-tests/<name>` by name, the data tables, the columns zhang adds, the
//! statements, the typed API and the limits. The beanquery comparisons are the `oracle` binary.
//!
//! Run one module with `cargo nextest run -p zhang-query -E 'binary(engine) & test(language::)'`.
//!
//! The kit loads a ledger as `zhang serve` does (`zhang_testkit::ledger`, `zhang_testkit::fixtures`), with the bundled
//! script `zhang:budget`, which computes the budget tables: a ledger with the script has a catalog of its own, which the
//! modules that read the budget tables compile their queries against ([`compile`]).

mod booked_rows;
mod data_tables;
mod evaluator;
mod functions;
mod having_pivot;
mod inline_params;
mod language;
mod ledger_columns;
mod limits;
mod lots;
mod now;
mod period;
mod posting_flags;
mod posting_metadata;
mod server_features;
mod server_features_api;
mod statements;
mod tables;
mod typed_api;
mod unnest;
mod zhang_tables;

use std::sync::{Arc, OnceLock};

use zhang_query::{Catalog, ParamTypes, Query, QueryError};

/// The catalog of a ledger the kit loads, with the tables of the bundled script `zhang:budget`: the same for every such
/// ledger, so a query compiled against it runs on each of them.
pub(crate) fn catalog() -> &'static Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let catalog = Catalog::of_ledger(&zhang_testkit::ledger::load_text("1970-01-01 open Assets:Bank\n"));
        assert_ne!(catalog.digest(), Catalog::standard().digest(), "the kit loads with the budget script");
        catalog
    })
}

/// `query` compiled against [`catalog`], as [`Query::compile`] compiles it against the standard catalog.
pub(crate) fn compile(query: &str) -> Result<Query, QueryError> {
    compile_with_params(query, &ParamTypes::new())
}

/// `query` with parameters of `params` compiled against [`catalog`], as [`Query::compile_with_params`] compiles it.
pub(crate) fn compile_with_params(query: &str, params: &ParamTypes) -> Result<Query, QueryError> {
    Query::compile_in(catalog(), query, params)
}
