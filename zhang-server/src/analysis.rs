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
use zhang_core::analysis::{AnalysisCatalog, AnalysisHost, AnalyzerTables, LentLedger, LoadOptions};
use zhang_core::ledger::Ledger;
use zhang_core::plugin::router::{Conversion, ConvertedAmount, QueryFailure, RawQueryParam, RouterHost};
use zhang_core::plugin_policy::PluginPolicy;
use zhang_query::params::bind_json;
use zhang_query::{Amount, Catalog, ParamTypes, Params, Query};
use zhang_shared::plugin_abi::{analysis as abi, HostError, HostErrorKind};

use crate::response::{QueryResultEntity, QuerySchemaEntity};
use crate::routes::query::{execute_options, max_result_values};
use crate::validate::written_digits;

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

    fn queries(&self, ledger: LentLedger) -> Arc<dyn RouterHost> {
        Arc::new(AnalysisQueries {
            ledger,
            queries: AtomicUsize::new(0),
        })
    }

    fn publish(&self, ledger: &Ledger, provider: &str, output: &abi::Output) -> Result<Arc<dyn Any + Send + Sync>, String> {
        zhang_query::analysis::publish(ledger, provider, output)
    }

    fn publish_raw(&self, ledger: &Ledger, provider: &str, output: &abi::RawOutput<'_>) -> Option<Arc<dyn Any + Send + Sync>> {
        zhang_query::analysis::publish_raw(ledger, provider, output)
    }
}

/// The host of the analyses, one for the process.
pub fn analysis_host() -> Arc<dyn AnalysisHost> {
    static HOST: LazyLock<Arc<dyn AnalysisHost>> = LazyLock::new(|| Arc::new(ServerAnalysisHost));
    HOST.clone()
}

/// The options the server loads a ledger with: `plugin_policy`, the operator's, and the host of the analyses. Budgets are
/// zhang's native ones.
///
/// A build with the feature `budget-switch` (tests and benches only: RFC #698's test switch) reads which implementation
/// computes budgets from [`BudgetImpl::ENV`] instead, and panics on a value it does not know.
pub fn load_options(plugin_policy: Arc<PluginPolicy>) -> LoadOptions {
    #[cfg(feature = "budget-switch")]
    let budgets = BudgetImpl::from_env().unwrap_or_else(|error| panic!("{error}"));
    #[cfg(not(feature = "budget-switch"))]
    let budgets = BudgetImpl::Native;
    load_options_for(plugin_policy, budgets)
}

/// Which implementation computes the budgets of a load: RFC #698's test switch, until the bundled script is the only one
/// (P3.3). A release build always loads with [`BudgetImpl::Native`]; tests and benches choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetImpl {
    /// zhang's native budget code: the in-process provider of the budget tables, and the fold's budget checks
    Native,
    /// the bundled Lua script `zhang:budget` (`plugins/budget/budget.lua`), an analyzer registered as a default plugin:
    /// its snapshot serves the budget tables, and its diagnostics stand for the native checks, which do not run
    Plugin,
    /// both, compared: a harness that compares (`zhang_testkit::budget_parity`, the budget fuzzer) loads the ledger
    /// natively and with the script, and reports where they differ. A single load with it is the script's
    Both,
}

impl BudgetImpl {
    /// the environment variable a build with `budget-switch` reads the switch from: `native` (or unset), `plugin` or
    /// `both`
    pub const ENV: &'static str = "ZHANG_BUDGET_IMPL";

    /// the switch as [`BudgetImpl::ENV`] sets it: `native` when it is unset or empty, an error for a value it does not
    /// know
    pub fn from_env() -> Result<BudgetImpl, String> {
        match std::env::var(Self::ENV).as_deref() {
            Err(_) | Ok("") | Ok("native") => Ok(BudgetImpl::Native),
            Ok("plugin") => Ok(BudgetImpl::Plugin),
            Ok("both") => Ok(BudgetImpl::Both),
            Ok(other) => Err(format!("{}={other:?}: expected native, plugin or both", Self::ENV)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BudgetImpl::Native => "native",
            BudgetImpl::Plugin => "plugin",
            BudgetImpl::Both => "both",
        }
    }
}

/// [`load_options`] with the budgets computed by `budgets`: [`BudgetImpl::Plugin`] and [`BudgetImpl::Both`] register the
/// bundled script `zhang:budget` as a default plugin, which the native provider and checks then make way for.
pub fn load_options_for(plugin_policy: Arc<PluginPolicy>, budgets: BudgetImpl) -> LoadOptions {
    let mut options = LoadOptions {
        analysis: Some(analysis_host()),
        ..LoadOptions::with_policy(plugin_policy)
    };
    if budgets != BudgetImpl::Native {
        options.default_plugins.push(zhang_core::analysis::DefaultPlugin::budget());
    }
    options
}

/// What an analyzer reads the ledger being loaded with, during its analysis: the ledger is lent for the call, and reads
/// nothing once it returned.
struct AnalysisQueries {
    ledger: LentLedger,
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
        let result = self.read(|ledger| {
            let query = Query::compile_in(&Catalog::of_ledger(ledger), bql, types).map_err(failure)?;
            query
                .execute_with_options(ledger, params, &execute_options(max_result_values()))
                .map_err(failure)
        })??;
        serde_json::to_value(QueryResultEntity::from(result)).map_err(|e| unlocated(format!("the result cannot be encoded: {e}")))
    }

    /// `read` with the ledger, while the analysis runs
    fn read<T>(&self, read: impl FnOnce(&Ledger) -> T) -> Result<T, QueryFailure> {
        self.ledger
            .with(read)
            .ok_or_else(|| unlocated("the analysis has ended: its ledger is no longer lent".to_owned()))
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
        let schema = self.read(zhang_query::schema_of)?;
        serde_json::to_value(QuerySchemaEntity::from(schema)).map_err(|e| unlocated(format!("the schema cannot be encoded: {e}")))
    }

    fn convert(&self, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
        self.ledger.with(|ledger| convert(ledger, conversions)).unwrap_or_else(|| {
            Err(HostError::new(
                HostErrorKind::Unavailable,
                "the analysis has ended: its ledger is no longer lent",
            ))
        })
    }
}

/// `conversions` converted with the prices of `ledger`, as `zhang_convert` answers: see
/// [`zhang_query::prices::convert_at`].
///
/// A plugin's number is bounded before anything reads it, as a number a write writes is (`MAX_WRITTEN_DIGITS`): its
/// text first, so that a text of millions of digits is refused at once instead of parsed (a parse takes the square
/// of its length), then its digits written out in full, so that `1e999999999999999999` is refused instead of written
/// out with as many zeros. An answer is bounded too: a price with a far exponent could give one of a million digits.
/// A refused number names itself in a few characters.
pub fn convert(ledger: &Ledger, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
    let limit = abi::MAX_CONVERTED_DIGITS;
    conversions
        .iter()
        .enumerate()
        .map(|(at, conversion)| {
            let invalid = |what: String| HostError::new(HostErrorKind::InvalidInput, format!("conversion {}: {}", at + 1, what));
            let text = conversion.amount.number.trim();
            // a number of `limit` digits, its sign, point and exponent written with it, is far shorter
            if text.len() > limit as usize + 32 {
                return Err(invalid(format!("{} is no number of at most {limit} digits", shown(text))));
            }
            let number = BigDecimal::from_str(text).map_err(|_| invalid(format!("{} is no decimal", shown(text))))?;
            if written_digits(&number) > limit {
                return Err(invalid(format!("{} has more than {limit} digits written out in full", shown(text))));
            }
            let date =
                NaiveDate::parse_from_str(&conversion.date, "%Y-%m-%d").map_err(|_| invalid(format!("{} is no date YYYY-MM-DD", shown(&conversion.date))))?;
            let amount = Amount::new(number, conversion.amount.currency.as_str());
            let Some(converted) = zhang_query::prices::convert_at(ledger, &amount, &conversion.target, conversion.via.as_deref(), date) else {
                return Ok(None);
            };
            if written_digits(&converted.number) > 2 * limit {
                return Err(invalid(format!(
                    "{} {} in {} has more than {} digits written out in full: the ledger's prices give it so",
                    shown(text),
                    shown(&conversion.amount.currency),
                    shown(&conversion.target),
                    2 * limit
                )));
            }
            Ok(Some(ConvertedAmount {
                number: zhang_query::decimal::to_plain_string(&converted.number),
                currency: converted.commodity,
            }))
        })
        .collect()
}

/// `text`, as an error names it: quoted, and cut after 40 characters.
fn shown(text: &str) -> String {
    match text.char_indices().nth(40) {
        None => format!("{text:?}"),
        Some((end, _)) => format!("{:?}… ({} bytes)", &text[..end], text.len()),
    }
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

    /// a price of far more digits than a plugin may get back gives an error, not a number of thousands of digits
    #[test]
    fn a_conversion_is_bounded_by_the_digits_of_its_answer() {
        let dir = tempfile::tempdir().unwrap().keep();
        std::fs::write(dir.join("main.zhang"), format!("2024-01-01 price USD 0.{}1 CNY\n", "0".repeat(2100))).unwrap();
        let ledger = Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))).unwrap();
        let error = convert(&ledger, &[conversion("1.00", "USD", "CNY", "2024-01-15", None)]).unwrap_err();
        assert!(
            error
                .message
                .ends_with("has more than 2000 digits written out in full: the ledger's prices give it so"),
            "{}",
            error.message
        );
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
        // numbers a plugin could give to exhaust the host: refused before anything reads them, quickly
        let probes = [
            ("1e999999999999999999", "has more than 1000 digits written out in full"),
            ("1e100000000", "has more than 1000 digits written out in full"),
            ("1e-9223372036854775807", "has more than 1000 digits written out in full"),
            (&"1".repeat(2_000_000), "… (2000000 bytes) is no number of at most 1000 digits"),
        ];
        for (number, error) in probes {
            let started = std::time::Instant::now();
            let refused = convert(&ledger, &[conversion(number, "USD", "CNY", "2024-01-15", None)]).unwrap_err();
            assert!(refused.message.ends_with(error), "{}", refused.message);
            assert!(refused.message.len() < 200, "the number is named shortly");
            assert!(started.elapsed() < std::time::Duration::from_secs(1), "{number:.20}: refused at once");
        }
        let error = convert(&ledger, &[conversion("ten", "USD", "CNY", "2024-01-15", None)]).unwrap_err();
        assert_eq!(error.message, "conversion 1: \"ten\" is no decimal");
        let error = convert(&ledger, &[conversion("1", "USD", "CNY", "2024-1-15x", None)]).unwrap_err();
        assert_eq!(error.message, "conversion 1: \"2024-1-15x\" is no date YYYY-MM-DD");
    }
}
