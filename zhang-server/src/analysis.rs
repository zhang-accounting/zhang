//! The host of the analyzers' analyses ([`zhang_core::analysis`]) for the ledgers the server loads: the query engine
//! builds a ledger's catalog and checks what an analyzer publishes ([`zhang_query::analysis`]), and an analyzer reads the
//! ledger being loaded as a router reads the ledger served: BQL with the limits of `POST /api/query`, answered as that
//! endpoint answers, and conversions with the ledger's prices.
//!
//! The server installs it with the load's options ([`load_options`]), and every load and reload of the ledger served
//! uses them; `zhang serve` loads through [`crate::load_served_ledger`]. A test that loads a ledger to analyse uses them
//! too.

use std::any::Any;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use serde_json::Value;
use zhang_core::analysis::{AnalysisCatalog, AnalysisHost, AnalyzerTables, LoadOptions};
use zhang_core::ledger::Ledger;
use zhang_core::plugin::router::{Conversion, ConvertedAmount, QueryFailure, RawQueryParam, RouterHost};
use zhang_core::plugin_policy::PluginPolicy;
use zhang_query::params::bind_json;
use zhang_query::{Amount, Catalog, ParamTypes, Params, Query};
use zhang_shared::plugin_abi::{analysis as abi, HostError, HostErrorKind};

use crate::response::{QueryResultEntity, QuerySchemaEntity};
use crate::routes::query::{execute_options, max_result_values};

/// The most queries one analysis runs, so that a script looping on queries ends before its time limit does.
pub const MAX_QUERIES: usize = 10_000;

/// The host of the analyses of the ledgers the server loads.
pub struct ServerAnalysisHost;

impl AnalysisHost for ServerAnalysisHost {
    fn catalog(&self, analyzers: &[AnalyzerTables<'_>]) -> AnalysisCatalog {
        let built = zhang_query::analysis::ledger_catalog(analyzers);
        AnalysisCatalog {
            analyzers: built.analyzers().map(str::to_owned).collect(),
            skipped: built.skipped,
            catalog: built.catalog as Arc<dyn Any + Send + Sync>,
        }
    }

    fn queries(&self, ledger: Arc<Ledger>) -> Arc<dyn RouterHost> {
        Arc::new(AnalysisQueries {
            ledger,
            queries: AtomicUsize::new(0),
        })
    }

    fn publish(&self, ledger: &Ledger, provider: &str, output: &abi::Output) -> Result<Arc<dyn Any + Send + Sync>, String> {
        zhang_query::analysis::publish(ledger, provider, output)
    }
}

/// The host of the analyses, one for the process.
pub fn analysis_host() -> Arc<dyn AnalysisHost> {
    static HOST: LazyLock<Arc<dyn AnalysisHost>> = LazyLock::new(|| Arc::new(ServerAnalysisHost));
    HOST.clone()
}

/// The options the server loads a ledger with: `plugin_policy`, the operator's, and the host of the analyses.
pub fn load_options(plugin_policy: Arc<PluginPolicy>) -> LoadOptions {
    LoadOptions {
        analysis: Some(analysis_host()),
        ..LoadOptions::with_policy(plugin_policy)
    }
}

/// What an analyzer reads the ledger being loaded with, during its analysis.
struct AnalysisQueries {
    ledger: Arc<Ledger>,
    /// the queries the analysis ran so far
    queries: AtomicUsize,
}

fn failure(error: zhang_query::QueryError) -> QueryFailure {
    QueryFailure {
        message: error.message,
        line: error.line,
        column: error.column,
    }
}

fn unlocated(message: String) -> QueryFailure {
    QueryFailure {
        message,
        line: None,
        column: None,
    }
}

impl AnalysisQueries {
    /// `bql` with parameters of `types` compiled against the ledger's catalog, run with `params` and the limits of
    /// `POST /api/query`, and answered as that endpoint answers
    fn run(&self, bql: &str, types: &ParamTypes, params: &Params) -> Result<Value, QueryFailure> {
        let count = self.queries.fetch_add(1, Ordering::Relaxed) + 1;
        if count > MAX_QUERIES {
            return Err(unlocated(format!("the analysis ran more than {MAX_QUERIES} queries")));
        }
        let query = Query::compile_in(&Catalog::of_ledger(&self.ledger), bql, types).map_err(failure)?;
        let result = query
            .execute_with_options(&self.ledger, params, &execute_options(max_result_values()))
            .map_err(failure)?;
        serde_json::to_value(QueryResultEntity::from(result)).map_err(|e| unlocated(format!("the result cannot be encoded: {e}")))
    }
}

impl RouterHost for AnalysisQueries {
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        self.run(bql, &ParamTypes::default(), &Params::new())
    }

    fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure> {
        let (types, values) = bind_json(params.iter().map(|(name, param)| (name.as_str(), param.ty.as_str(), &param.value))).map_err(unlocated)?;
        self.run(bql, &types, &values)
    }

    fn schema(&self) -> Result<Value, QueryFailure> {
        serde_json::to_value(QuerySchemaEntity::from(zhang_query::schema_of(&self.ledger))).map_err(|e| unlocated(format!("the schema cannot be encoded: {e}")))
    }

    fn convert(&self, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
        convert(&self.ledger, conversions)
    }
}

/// `conversions` converted with the prices of `ledger`, as `zhang_convert` answers: see
/// [`zhang_query::prices::convert_at`].
pub fn convert(ledger: &Ledger, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
    conversions
        .iter()
        .enumerate()
        .map(|(at, conversion)| {
            let invalid = |what: String| HostError::new(HostErrorKind::InvalidInput, format!("conversion {}: {}", at + 1, what));
            let number = BigDecimal::from_str(conversion.amount.number.trim())
                .ok()
                .filter(|_| conversion.amount.number.len() <= 100)
                .ok_or_else(|| invalid(format!("{:?} is no decimal", conversion.amount.number)))?;
            let date = NaiveDate::parse_from_str(&conversion.date, "%Y-%m-%d").map_err(|_| invalid(format!("{:?} is no date YYYY-MM-DD", conversion.date)))?;
            let amount = Amount::new(number, conversion.amount.currency.as_str());
            Ok(
                zhang_query::prices::convert_at(ledger, &amount, &conversion.target, conversion.via.as_deref(), date).map(|converted| ConvertedAmount {
                    number: zhang_query::decimal::to_plain_string(&converted.number),
                    currency: converted.commodity,
                }),
            )
        })
        .collect()
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;
    use zhang_core::plugin::router::{Conversion, ConvertedAmount};

    use super::convert;

    fn conversion(number: &str, currency: &str, target: &str, date: &str, via: Option<&str>) -> Conversion {
        Conversion {
            amount: ConvertedAmount {
                number: number.to_owned(),
                currency: currency.to_owned(),
            },
            target: target.to_owned(),
            date: date.to_owned(),
            via: via.map(str::to_owned),
        }
    }

    #[test]
    fn amounts_convert_as_queries_convert_them() {
        let dir = tempfile::tempdir().unwrap().keep();
        std::fs::write(
            dir.join("main.zhang"),
            "2024-01-01 price USD 7.10 CNY
2024-02-01 price USD 7.20 CNY
2024-01-01 price GOLD 400 USD
",
        )
        .unwrap();
        let ledger = Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))).unwrap();
        let converted = convert(
            &ledger,
            &[
                conversion("10.00", "USD", "CNY", "2024-01-15", None),
                conversion("10.00", "USD", "CNY", "2024-02-01", None),
                conversion("71.0", "CNY", "USD", "2024-01-15", None),
                conversion("2", "GOLD", "CNY", "2024-02-01", Some("USD")),
                conversion("2", "GOLD", "CNY", "2024-02-01", None),
                conversion("10.00", "USD", "CNY", "2023-12-31", None),
                conversion("12.50", "CNY", "CNY", "2000-01-01", None),
            ],
        )
        .unwrap();
        let text = converted
            .iter()
            .map(|it| it.as_ref().map(|amount| format!("{} {}", amount.number, amount.currency)))
            .collect::<Vec<_>>();
        assert_eq!(
            text,
            [
                Some("71.0000 CNY".to_owned()),
                Some("72.0000 CNY".to_owned()),
                // an inverse rate keeps the digits of the division, as `convert` keeps them
                Some("10.00000000000000000000000000 USD".to_owned()),
                Some("5760.00 CNY".to_owned()),
                None,
                None,
                Some("12.50 CNY".to_owned()),
            ]
        );
        let error = convert(&ledger, &[conversion("ten", "USD", "CNY", "2024-01-15", None)]).unwrap_err();
        assert_eq!(error.message, "conversion 1: \"ten\" is no decimal");
        let error = convert(&ledger, &[conversion("1", "USD", "CNY", "2024-1-15x", None)]).unwrap_err();
        assert_eq!(error.message, "conversion 1: \"2024-1-15x\" is no date YYYY-MM-DD");
    }
}
