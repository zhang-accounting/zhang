//! Analyzer plugins at load time: the loop of [`crate::analysis`] that runs them, and one analyzer's call.
//!
//! Each analyzer runs in a new instance of its plugin, with its own manifest's config, timeout and memory limit, like
//! a stage. It reads the ledger through the router functions (`zhang_query`, `zhang_query_params`,
//! `zhang_query_schema`, `zhang_ledger_info`) and `zhang_convert`, bound to the ledger being loaded, which is lent to
//! them for the call only ([`analysis::LentLedger`]); `zhang_now` returns the time of the load and makes the ledger
//! depend on the date. It reads no file and reaches no network, whatever its directive grants: an analysis is a
//! function of the ledger. What it reports through `zhang_emit_error` is logged and dropped: its diagnostics are those
//! of its output.
//!
//! The analyses of one load run at most the operator's `ZHANG_PLUGIN_MAX_TIMEOUT` together: each runs at most its own
//! timeout and at most the time the analyses before it left, and one whose turn comes once that time is spent does not
//! run. Both fail with the reason `timeout`.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{info, warn};
use zhang_shared::plugin_abi::analysis as abi;

use crate::analysis::{self, AnalysisFailure, AnalyzerTables, Diagnostic, Lend, Reported};
use crate::extensions::{AnalyzerSlot, Snapshot};
use crate::inputs::ExtraInput;
use crate::ledger::Ledger;
use crate::plugin::host::{PluginHost, MESSAGE_META};
use crate::plugin::log_safe;
use crate::plugin::lua::OUT_OF_LUA_MEMORY;
use crate::plugin::router::{ledger_info, RouterHost};
use crate::plugin::store::RegisteredPlugin;
use crate::plugin_policy::{Size, MAX_MEMORY_ENV};

/// What an analyzer's call answered: its output as bytes, or why there is none, and whether it read the time (a call
/// that failed may have read it too).
pub struct AnalyzerRun {
    pub output: Result<Vec<u8>, AnalysisFailure>,
    pub read_clock: bool,
}

impl RegisteredPlugin {
    /// Analyse `ledger` in a new instance of the plugin: call its `analyze` export with `input`, the JSON of an
    /// [`abi::Input`], with the router functions reading the ledger through `host`, for at most its own timeout and at
    /// most `time_left`, what the analyses of the load have left.
    pub fn execute_as_analyzer(&self, ledger: &Ledger, input: &[u8], host: Arc<dyn RouterHost>, time_left: Duration) -> AnalyzerRun {
        let plugin_host =
            PluginHost::analysing(self.name.clone(), self.span().clone(), ledger.clock.clone(), ledger.options.timezone).with_router(host, ledger_info(ledger));
        let timeout = self.analysis_timeout();
        let timed_out = match time_left < timeout {
            true => format!(
                "its analysis ran longer than the {time_left:?} the analyses before it left of the {:?} the analyses of a load may take together (ZHANG_PLUGIN_MAX_TIMEOUT)",
                self.policy().max_timeout
            ),
            false => format!("its analysis ran longer than {timeout:?}; a `timeout` meta on its `plugin` directive raises the limit, up to the server's"),
        };
        let output = (|| {
            let mut plugin = self
                .runtime
                .instantiate(self.manifest(&ledger.options.all()), plugin_host.clone())
                .map_err(|e| AnalysisFailure::new("trap", format!("it cannot be loaded: {}", log_safe(&format!("{e:#}")))))?;
            // the instance has the plugin's own timeout: it gets what is left, if that is less
            plugin.limit_timeout(time_left);
            if !plugin.function_exists(abi::ANALYZE) {
                return Err(AnalysisFailure::new(
                    "manifest",
                    format!("it declares the type Analyzer and exports no `{}`", abi::ANALYZE),
                ));
            }
            // the interpreter of a script says with its code that the script ran out of its Lua memory; a wasm module's
            // code is its own, and one without an error is a failure all the same
            let called = match self.script() {
                Some(_) => plugin
                    .call_get_error_code::<&[u8], &[u8]>(abi::ANALYZE, input)
                    .map_err(|(error, code)| (error, Some(code))),
                None => plugin.call::<&[u8], &[u8]>(abi::ANALYZE, input).map_err(|error| (error, None)),
            };
            called.map(<[u8]>::to_vec).map_err(|(error, code)| match error.to_string().as_str() {
                // extism reports a call it stopped at the manifest's timeout as exactly "timeout"
                "timeout" => AnalysisFailure::new("timeout", timed_out.clone()),
                // and one whose memory would grow past the manifest's limit as exactly "oom"
                "oom" => AnalysisFailure::new(
                    "memory",
                    format!(
                        "its analysis needed more than the {} the server allows a plugin ({MAX_MEMORY_ENV})",
                        Size(self.policy().max_memory)
                    ),
                ),
                // a Lua script's error comes with its stack trace: the ledger's error says what failed, where the
                // script raised it, and the trace goes to the log
                _ => {
                    let text = format!("{error:#}");
                    let cause = text.split_once("\nstack traceback:").map_or(text.as_str(), |(cause, _)| cause.trim_end());
                    if cause.len() < text.len() {
                        warn!("analyzer {} failed: {}", log_safe(&self.name), log_safe(&text));
                    }
                    // a script that ran out of its Lua memory needed more memory than the server allows, too
                    let reason = if code == Some(OUT_OF_LUA_MEMORY) { "memory" } else { "trap" };
                    AnalysisFailure::new(reason, log_safe(cause).into_owned())
                }
            })
        })();
        // an analysis has no error list of its own: what the plugin reports is in its output
        for error in plugin_host.take_errors() {
            let message = error.metas.get(MESSAGE_META).map(String::as_str).unwrap_or_default();
            warn!(
                "analyzer {} reported a problem through zhang_emit_error, which an analysis drops (its diagnostics are those of its output): {}",
                log_safe(&self.name),
                log_safe(message)
            );
        }
        AnalyzerRun {
            output,
            read_clock: plugin_host.take_clock_read(),
        }
    }

    /// how long one analysis of the plugin may run: its `timeout`, within the policy's `max_timeout`
    pub fn analysis_timeout(&self) -> Duration {
        self.capabilities().timeout.min(self.policy().max_timeout)
    }
}

/// Run the analyzers of `ledger` (see [`crate::analysis`]): its catalog and the snapshots they published go into it,
/// and what they reported comes back, to merge into its errors.
pub(crate) fn analyze(ledger: &mut Ledger) -> Reported {
    let analyzers = ledger.plugins.analyzers().cloned().collect::<Vec<_>>();
    let mut reported = Reported::default();
    if analyzers.is_empty() {
        return reported;
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
            fail(ledger, &mut reported, plugin, failure);
        }
        return reported;
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
            (Some(Err(message)), _) => fail(ledger, &mut reported, plugin, AnalysisFailure::new("manifest", message.clone())),
            (_, Some(message)) => fail(ledger, &mut reported, plugin, AnalysisFailure::new("manifest", message)),
            _ if catalog.analyzers.contains(&plugin.name) => runs.push(plugin.clone()),
            _ => {}
        }
    }
    ledger.extensions.catalog = Some(catalog.catalog);
    ledger.extensions.analyzers = catalog.analyzers.iter().map(AnalyzerSlot::new).collect();
    ledger.extensions.analysing = true;

    // every analyzer reads the ledger as it is now, through a loan its host functions cannot keep
    let budget = ledger.plugin_policy.max_timeout;
    let started = Instant::now();
    let mut outcomes = vec![];
    {
        let shared: &Ledger = ledger;
        for plugin in &runs {
            let Outcome { snapshot, report, read_clock } = match budget.checked_sub(started.elapsed()).filter(|left| !left.is_zero()) {
                Some(time_left) => run(shared, &*host, plugin, time_left),
                None => Outcome::failed(
                    AnalysisFailure::new(
                        "timeout",
                        format!("it did not run: the analyses of this load ran past {budget:?} together (ZHANG_PLUGIN_MAX_TIMEOUT)"),
                    ),
                    false,
                ),
            };
            let slot = shared.extensions.analyzer(&plugin.name).expect("a slot per analyzer of the catalog");
            if slot.snapshot.set(snapshot).is_err() {
                unreachable!("the catalog runs one analyzer per provider");
            }
            outcomes.push((plugin, report, read_clock));
        }
    }
    ledger.extensions.analysing = false;
    for (plugin, report, read_clock) in outcomes {
        if read_clock {
            ledger.extra_inputs.insert(ExtraInput::Clock);
        }
        match report {
            Ok(diagnostics) => {
                let span = analysis::plugin_span(ledger, plugin.span(), plugin.analyzer_manifest());
                reported.add(diagnostics, span.as_ref(), &ledger.directives);
            }
            Err(failure) => fail(ledger, &mut reported, plugin, failure),
        }
    }
    reported
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

/// Run the analyzer `plugin` on `ledger`, for at most `time_left`, and check what it answers.
fn run(ledger: &Ledger, host: &dyn analysis::AnalysisHost, plugin: &RegisteredPlugin, time_left: Duration) -> Outcome {
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
    let started = Instant::now();
    // the records go once written: the call holds the text only
    let written = serde_json::to_vec(&input);
    drop(input);
    let input = match written {
        Ok(input) => input,
        Err(error) => return Outcome::failed(AnalysisFailure::new("trap", format!("its input cannot be written: {error}")), false),
    };
    let run = {
        // lent for the call only: once it returned, the host functions it may have kept read nothing
        let lend = Lend::new(ledger);
        plugin.execute_as_analyzer(ledger, &input, host.queries(lend.handle()), time_left)
    };
    let outcome = match run.output.and_then(|output| check(ledger, host, plugin, manifest, &output)) {
        Ok((data, diagnostics)) => Outcome {
            snapshot: Snapshot::ok(data),
            report: Ok(diagnostics),
            read_clock: run.read_clock,
        },
        Err(failure) => Outcome::failed(failure, run.read_clock),
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

/// The data and the diagnostics of `output`, what `plugin` answered, checked whole: its size first, without reading
/// it ([`abi::check_size`]), then the shapes. An output is read the quick way first, straight into cells
/// ([`AnalysisHost::publish_raw`](analysis::AnalysisHost::publish_raw)); one that it does not take is read again the
/// long way, which says what does not hold.
fn check(
    ledger: &Ledger, host: &dyn analysis::AnalysisHost, plugin: &RegisteredPlugin, manifest: &abi::Manifest, output: &[u8],
) -> Result<(Arc<dyn Any + Send + Sync>, Vec<Diagnostic>), AnalysisFailure> {
    let invalid = |message: String| AnalysisFailure::new("output", message);
    abi::check_size(output, abi::MAX_OUTPUT_BYTES, abi::MAX_OUTPUT_NODES, abi::MAX_OUTPUT_OBJECTS)
        .map_err(|why| invalid(format!("its output is too large to read: {}", log_safe(&why))))?;
    if let Some(checked) = quick_check(ledger, host, plugin, manifest, output) {
        return Ok(checked);
    }
    let output: abi::Output = serde_json::from_slice(output).map_err(|e| invalid(format!("its output is not one: {}", log_safe(&e.to_string()))))?;
    let diagnostics = analysis::diagnostics(ledger, &plugin.name, manifest, plugin.is_budget_provider(), &output.diagnostics).map_err(invalid)?;
    let data = host.publish(ledger, &plugin.name, &output).map_err(invalid)?;
    Ok((data, diagnostics))
}

/// [`check`] the quick way, after the size: the diagnostics and the data of `output`, or `None` when the quick reading
/// does not take it or something does not hold.
fn quick_check(
    ledger: &Ledger, host: &dyn analysis::AnalysisHost, plugin: &RegisteredPlugin, manifest: &abi::Manifest, output: &[u8],
) -> Option<(Arc<dyn Any + Send + Sync>, Vec<Diagnostic>)> {
    let output: abi::RawOutput<'_> = serde_json::from_slice(output).ok()?;
    let diagnostics = analysis::diagnostics(ledger, &plugin.name, manifest, plugin.is_budget_provider(), &output.diagnostics).ok()?;
    let data = host.publish_raw(ledger, &plugin.name, &output)?;
    Some((data, diagnostics))
}
