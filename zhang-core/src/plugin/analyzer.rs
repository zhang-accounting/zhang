//! Analyzer plugins at load time: the loop of [`crate::analysis`] that runs them, and one analyzer's call.
//!
//! Each analyzer runs in a new instance of its plugin, with its own manifest's config, timeout and memory limit, like
//! a stage. It reads the ledger through the router functions (`zhang_query`, `zhang_query_params`,
//! `zhang_query_schema`, `zhang_ledger_info`) and `zhang_convert`, bound to the ledger being loaded; `zhang_now`
//! returns the time of the load and makes the ledger depend on the date. It reads no file and reaches no network,
//! whatever its directive grants: an analysis is a function of the ledger. What it reports through `zhang_emit_error`
//! is logged and dropped: its diagnostics are those of its output.

use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use zhang_shared::plugin_abi::analysis as abi;

use crate::analysis::{self, AnalysisFailure, AnalyzerTables, Diagnostic, Reported};
use crate::extensions::{AnalyzerSlot, Snapshot};
use crate::inputs::ExtraInput;
use crate::ledger::Ledger;
use crate::plugin::host::{PluginHost, MESSAGE_META};
use crate::plugin::log_safe;
use crate::plugin::router::{ledger_info, RouterHost};
use crate::plugin::store::RegisteredPlugin;
use crate::plugin_policy::{Size, MAX_MEMORY_ENV};

/// What an analyzer's call answered: its output, as bytes, and whether it read the time.
pub struct AnalyzerRun {
    pub output: Vec<u8>,
    pub read_clock: bool,
}

impl RegisteredPlugin {
    /// Analyse `ledger` in a new instance of the plugin: call its `analyze` export with `input`, the JSON of an
    /// [`abi::Input`], with the router functions reading the ledger through `host`. The answer, or why there is none.
    pub fn execute_as_analyzer(&self, ledger: &Ledger, input: &[u8], host: Arc<dyn RouterHost>) -> Result<AnalyzerRun, AnalysisFailure> {
        let plugin_host =
            PluginHost::analysing(self.name.clone(), self.span().clone(), ledger.clock.clone(), ledger.options.timezone).with_router(host, ledger_info(ledger));
        let timeout = self.analysis_timeout();
        let output = {
            let mut plugin = self
                .runtime
                .instantiate(self.manifest(&ledger.options.all()), plugin_host.clone())
                .map_err(|e| AnalysisFailure::new("trap", format!("it cannot be loaded: {}", log_safe(&format!("{e:#}")))))?;
            if !plugin.function_exists(abi::ANALYZE) {
                return Err(AnalysisFailure::new(
                    "manifest",
                    format!("it declares the type Analyzer and exports no `{}`", abi::ANALYZE),
                ));
            }
            plugin.call::<&[u8], &[u8]>(abi::ANALYZE, input).map(<[u8]>::to_vec)
        };
        // an analysis has no error list of its own: what the plugin reports is in its output
        for error in plugin_host.take_errors() {
            let message = error.metas.get(MESSAGE_META).map(String::as_str).unwrap_or_default();
            warn!(
                "analyzer {} reported a problem through zhang_emit_error, which an analysis drops (its diagnostics are those of its output): {}",
                log_safe(&self.name),
                log_safe(message)
            );
        }
        let read_clock = plugin_host.take_clock_read();
        match output {
            Ok(output) => Ok(AnalyzerRun { output, read_clock }),
            Err(error) => Err(match error.to_string().as_str() {
                // extism reports a call it stopped at the manifest's timeout as exactly "timeout"
                "timeout" => AnalysisFailure::new(
                    "timeout",
                    format!("its analysis ran longer than {timeout:?}; a `timeout` meta on its `plugin` directive raises the limit, up to the server's"),
                ),
                // and one whose memory would grow past the manifest's limit as exactly "oom"
                "oom" => AnalysisFailure::new(
                    "memory",
                    format!(
                        "its analysis needed more than the {} the server allows a plugin ({MAX_MEMORY_ENV})",
                        Size(self.policy().max_memory)
                    ),
                ),
                _ => AnalysisFailure::new("trap", log_safe(&format!("{error:#}")).into_owned()),
            }),
        }
    }

    /// how long one analysis of the plugin may run: its `timeout`, within the policy's `max_timeout`
    pub fn analysis_timeout(&self) -> Duration {
        self.capabilities().timeout.min(self.policy().max_timeout)
    }
}

/// Run the analyzers of `ledger` (see [`crate::analysis`]): the ledger with its catalog and the snapshots they
/// published, and what they reported, to merge into its errors.
pub(crate) fn analyze(mut ledger: Ledger) -> (Ledger, Reported) {
    let analyzers = ledger.plugins.analyzers().cloned().collect::<Vec<_>>();
    let mut reported = Reported::default();
    if analyzers.is_empty() {
        return (ledger, reported);
    }
    let fail = |ledger: &Ledger, reported: &mut Reported, plugin: &RegisteredPlugin, failure: AnalysisFailure| {
        warn!(
            "analyzer {} did not analyse the ledger ({}): {}",
            log_safe(&plugin.name),
            failure.reason,
            log_safe(&failure.message)
        );
        if let Some(span) = analysis::plugin_span(ledger, plugin.span(), plugin.analyzer_manifest()) {
            reported.on_plugins.push(analysis::failure_error(&plugin.name, &failure, &span));
        }
    };
    let Some(host) = ledger.load_options.analysis.clone() else {
        for plugin in &analyzers {
            let failure = AnalysisFailure::new("unavailable", "this zhang analyses no ledger: it runs without an analysis host");
            fail(&ledger, &mut reported, plugin, failure);
        }
        return (ledger, reported);
    };
    // the analyzers whose manifest holds declare the catalog's tables; the others are reported, in the order of all
    let declared = analyzers
        .iter()
        .filter_map(|plugin| plugin.analyzer_manifest().map(|manifest| (plugin, manifest)))
        .collect::<Vec<_>>();
    let tables = declared
        .iter()
        .map(|(plugin, manifest)| AnalyzerTables {
            provider: &plugin.name,
            manifest,
            legacy: plugin.is_budget_provider(),
        })
        .collect::<Vec<_>>();
    let catalog = host.catalog(&tables);
    let skipped = |plugin: &RegisteredPlugin| {
        declared
            .iter()
            .position(|(declared, _)| std::ptr::eq(*declared, plugin))
            .and_then(|at| catalog.skipped.iter().find(|(index, _)| *index == at))
            .map(|(_, message)| message.clone())
    };
    let mut runs = vec![];
    for plugin in &analyzers {
        match (plugin.analyzer(), skipped(plugin)) {
            (Some(Err(message)), _) => fail(&ledger, &mut reported, plugin, AnalysisFailure::new("manifest", message.clone())),
            (_, Some(message)) => fail(&ledger, &mut reported, plugin, AnalysisFailure::new("manifest", message)),
            _ if catalog.analyzers.contains(&plugin.name) => runs.push(plugin.clone()),
            _ => {}
        }
    }
    ledger.extensions.catalog = Some(catalog.catalog);
    ledger.extensions.analyzers = catalog.analyzers.iter().map(AnalyzerSlot::new).collect();
    ledger.extensions.analysing = true;

    // the analyzers share the ledger with the host functions of their calls; the ledger is whole again once they end
    let shared = Arc::new(ledger);
    let mut outcomes = vec![];
    for plugin in &runs {
        let Outcome { snapshot, report, read_clock } = run(&shared, &*host, plugin);
        let slot = shared.extensions.analyzer(&plugin.name).expect("a slot per analyzer of the catalog");
        if slot.snapshot.set(snapshot).is_err() {
            unreachable!("the catalog runs one analyzer per provider");
        }
        outcomes.push((plugin, report, read_clock));
    }
    let mut ledger = Arc::into_inner(shared).expect("an analysis keeps no reference to the ledger once it ended");
    ledger.extensions.analysing = false;
    for (plugin, report, read_clock) in outcomes {
        if read_clock {
            ledger.extra_inputs.insert(ExtraInput::Clock);
        }
        match report {
            Ok(diagnostics) => {
                let span = analysis::plugin_span(&ledger, plugin.span(), plugin.analyzer_manifest());
                reported.add(diagnostics, span.as_ref(), &ledger.directives);
            }
            Err(failure) => fail(&ledger, &mut reported, plugin, failure),
        }
    }
    (ledger, reported)
}

/// What one analysis ended with: what it publishes, and its diagnostics or why it failed.
struct Outcome {
    snapshot: Snapshot,
    report: Result<Vec<Diagnostic>, AnalysisFailure>,
    read_clock: bool,
}

impl Outcome {
    fn failed(failure: AnalysisFailure, read_clock: bool) -> Outcome {
        Outcome {
            snapshot: Snapshot::failed(failure.reason),
            report: Err(failure),
            read_clock,
        }
    }
}

/// Run the analyzer `plugin` on `ledger`, and check what it answers.
fn run(ledger: &Arc<Ledger>, host: &dyn analysis::AnalysisHost, plugin: &RegisteredPlugin) -> Outcome {
    let manifest = plugin.analyzer_manifest().expect("an analyzer of the catalog has a manifest");
    let input = analysis::input(ledger, manifest);
    if !analysis::reads_something(ledger, manifest, &input) {
        info!("analyzer {} has nothing to read in the ledger: its tables are empty", log_safe(&plugin.name));
        return Outcome {
            snapshot: Snapshot::skipped(),
            report: Ok(vec![]),
            read_clock: false,
        };
    }
    let started = std::time::Instant::now();
    let input = match serde_json::to_vec(&input) {
        Ok(input) => input,
        Err(error) => return Outcome::failed(AnalysisFailure::new("trap", format!("its input cannot be written: {error}")), false),
    };
    let outcome = match plugin.execute_as_analyzer(ledger, &input, host.queries(ledger.clone())) {
        Err(failure) => Outcome::failed(failure, false),
        Ok(run) => match check(ledger, host, plugin, manifest, &run.output) {
            Ok((data, diagnostics)) => Outcome {
                snapshot: Snapshot::ok(data),
                report: Ok(diagnostics),
                read_clock: run.read_clock,
            },
            Err(failure) => Outcome::failed(failure, run.read_clock),
        },
    };
    info!(
        "analyzer {} analysed the ledger in {:?}: {}",
        log_safe(&plugin.name),
        started.elapsed(),
        match &outcome.report {
            Ok(diagnostics) => format!("{} diagnostics", diagnostics.len()),
            Err(failure) => format!("failed ({})", failure.reason),
        }
    );
    outcome
}

/// The data and the diagnostics of `output`, what `plugin` answered, checked whole.
fn check(
    ledger: &Ledger, host: &dyn analysis::AnalysisHost, plugin: &RegisteredPlugin, manifest: &abi::Manifest, output: &[u8],
) -> Result<(Arc<dyn std::any::Any + Send + Sync>, Vec<Diagnostic>), AnalysisFailure> {
    let invalid = |message: String| AnalysisFailure::new("output", message);
    if output.len() > analysis::MAX_OUTPUT_BYTES {
        return Err(invalid(format!(
            "its output is {} bytes of JSON, more than the {} an analysis may answer",
            output.len(),
            analysis::MAX_OUTPUT_BYTES
        )));
    }
    let output: abi::Output = serde_json::from_slice(output).map_err(|e| invalid(format!("its output is not one: {}", log_safe(&e.to_string()))))?;
    let diagnostics = analysis::diagnostics(ledger, &plugin.name, manifest, plugin.is_budget_provider(), &output.diagnostics).map_err(invalid)?;
    let data = host.publish(ledger, &plugin.name, &output).map_err(invalid)?;
    Ok((data, diagnostics))
}
