//! WASM plugins through the real load path, using the hand-written WAT fixtures in `tests/plugins/`.
//!
//! extism compiles WAT text itself, so the fixtures need no wasm toolchain. A test copies the fixtures
//! it uses into its own ledger directory and declares them by absolute path: a local ledger resolves
//! a plugin module against the working directory, and a path per test gives every test its own
//! module cache file.
#![cfg(feature = "plugin_runtime")]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use serde_json::json;
use tempfile::TempDir;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Transaction};
use zhang_core::clock::Clock;
use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::outcome::Detail;
use zhang_core::plugin::capabilities::{plugin_seed, PluginStage, WriteKind};
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::router::{QueryFailure, RawQueryParam, RouterError, RouterHost, WriteBatch, WriteCommitted, WritePreview};
use zhang_core::plugin::PluginType;
use zhang_core::ZhangResult;
use zhang_query::params::{bind_json, to_bql};
use zhang_query::{ParamRef, Query, QueryError};
use zhang_shared::plugin_abi::HostError;

const LEDGER: &str = indoc! {r#"
    1970-01-01 commodity CNY
    1970-01-01 open Assets:Cash
    1970-01-01 open Expenses:Food
    2024-01-02 * "lunch"
      Assets:Cash -10 CNY
      Expenses:Food 10 CNY
"#};

/// a fresh ledger directory holding copies of the given fixtures
fn ledger_dir(fixtures: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for fixture in fixtures {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugins").join(fixture);
        std::fs::copy(&source, dir.path().join(fixture)).unwrap_or_else(|e| panic!("cannot copy {}: {e}", source.display()));
    }
    dir
}

/// the `plugin` directive declaring a fixture copied into `dir`
fn plugin(dir: &TempDir, fixture: &str) -> String {
    format!("plugin \"{}\"\n", dir.path().join(fixture).display())
}

fn try_load(dir: &TempDir, content: &str) -> ZhangResult<Ledger> {
    try_load_from(dir, content, Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})))
}

fn try_load_from(dir: &TempDir, content: &str, source: Arc<dyn DataSource>) -> ZhangResult<Ledger> {
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source)
}

fn load(dir: &TempDir, content: &str) -> Ledger {
    try_load(dir, content).unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

/// load `content` as the main file of `dir`, reading the current time from `clock`
fn load_with_clock(dir: &TempDir, content: &str, clock: Clock) -> Ledger {
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source.load(root.to_string_lossy().to_string(), "main.zhang".to_owned()).unwrap();
    Ledger::process(LedgerProcessContext {
        directives: loaded.directives,
        entry: (root, "main.zhang".to_owned()),
        dialect: zhang_core::data_type::Dialect::Zhang,
        visited_files: loaded.visited_files,
        data_source: source,
        clock,
    })
    .unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

/// the contents of the comment directives in the processed stream, in order
fn comments(ledger: &Ledger) -> Vec<String> {
    ledger
        .metas
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Comment(comment) => Some(comment.content.clone()),
            _ => None,
        })
        .collect()
}

/// the registered plugins in declaration order, with the types they run as
fn registered(ledger: &Ledger) -> Vec<(String, Vec<PluginType>)> {
    ledger
        .plugins
        .ordered
        .iter()
        .map(|(plugin, types)| (plugin.name.clone(), types.clone()))
        .collect()
}

/// the padding transactions (flag `P`) the load accepted, in the order zhang processed them, with their dates
fn paddings(ledger: &Ledger) -> Vec<(chrono::NaiveDate, &Transaction)> {
    let transactions = ledger.transactions().into_iter().map(|(_, txn)| txn);
    let paddings = transactions.filter(|txn| txn.flag == Some(zhang_ast::Flag::BalancePad));
    paddings
        .map(|txn| (txn.date.to_timezone_datetime(&ledger.options.timezone).date_naive(), txn))
        .collect()
}

/// the postings of the booked transaction `txn` as written: the account, and the units booking gave it
fn written(txn: &Transaction) -> Vec<(String, Amount)> {
    let groups = zhang_ast::written_groups(&txn.postings);
    groups
        .into_iter()
        .map(|group| (group.legs[0].account.name().to_owned(), zhang_ast::group_units(group.legs)))
        .collect()
}

/// what the ledger holds: accounts, the postings of every transaction, and the reported errors
fn store_summary(ledger: &Ledger) -> (Vec<String>, Vec<String>, Vec<String>) {
    let txns = ledger.transactions();
    let postings = txns
        .iter()
        .flat_map(|(_, txn)| written(txn))
        .map(|(account, units)| format!("{account} {units}"))
        .sorted()
        .collect();
    let opens = ledger.directives.iter().filter_map(|it| match &it.data {
        Directive::Open(open) => Some(open.account.name().to_owned()),
        _ => None,
    });
    let accounts = opens.unique().sorted().collect();
    let errors = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
    (accounts, postings, errors)
}

/// the errors in the store with their spans and metas
fn errors(ledger: &Ledger) -> Vec<(ErrorKind, SpanInfo, HashMap<String, String>)> {
    ledger
        .errors
        .iter()
        .map(|it| (it.error_type.clone(), it.span.clone().expect("a stage error has a span"), it.metas.clone()))
        .collect()
}

fn metas(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
}

/// assert `span` is the span of `directive` in `content`, the ledger's main file
fn assert_directive_span(span: &SpanInfo, content: &str, directive: &str) {
    assert_eq!(span.start, content.find(directive).expect("the directive is in the ledger"));
    assert_eq!(span.content.trim_end(), directive.trim_end());
    assert!(span.filename.as_ref().is_some_and(|it| it.ends_with("main.zhang")), "{:?}", span.filename);
}

#[test]
fn echo_processor_stream_reaches_the_store() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "echo.wat")));

    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);
    // without a `stage` meta the plugin runs after booking
    let stages = ledger.plugins.build_stages(PluginStage::Booked);
    assert_eq!(stages.iter().map(|it| it.name()).collect_vec(), vec!["echo"]);
    assert!(ledger.plugins.build_stages(PluginStage::Raw).is_empty());

    let plain = load(&ledger_dir(&[]), LEDGER);
    assert_eq!(store_summary(&ledger), store_summary(&plain));
    assert_eq!(store_summary(&ledger).1.len(), 2, "the lunch transaction reaches the store");
}

#[test]
fn plugin_declaring_an_unknown_type_still_loads() {
    let dir = ledger_dir(&["unknown_type.wat"]);
    let ledger = load(
        &dir,
        &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "unknown_type.wat")),
    );

    // the unknown type is ignored, the known one still runs: this processor drops the whole stream
    assert_eq!(registered(&ledger), vec![("unknown-type".to_owned(), vec![PluginType::Processor])]);
    assert_eq!(store_summary(&ledger), (vec![], vec![], vec![]));
}

#[test]
fn features_plugins_enables_plugins_too() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &format!("option \"features.plugins\" \"true\"\n{}{LEDGER}", plugin(&dir, "echo.wat")));
    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);

    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &format!("{}{LEDGER}", plugin(&dir, "echo.wat")));
    assert_eq!(registered(&ledger), vec![], "without the feature option, plugins stay off");
}

/// a ledger with plugins on, `plugin_lines` and the lunch transaction
fn with_plugins(plugin_lines: &str) -> String {
    format!("option \"features.plugin\" \"true\"\n{plugin_lines}{LEDGER}")
}

#[test]
fn looping_plugin_fails_the_load_at_its_timeout() {
    let dir = ledger_dir(&["loop.wat"]);
    let started = Instant::now();

    let result = try_load(&dir, &with_plugins(&format!("{}  timeout: \"1\"\n", plugin(&dir, "loop.wat"))));

    let elapsed = started.elapsed();
    let Err(error) = result else {
        panic!("a plugin that never returns must fail the load")
    };
    let message = error.to_string();
    assert!(message.contains("plugin loop timed out"), "{message}");
    assert!(message.contains("`processor` call ran longer than 1s"), "{message}");
    assert!(elapsed < Duration::from_secs(20), "the load took {elapsed:?}, not about a second");
}

#[test]
fn invalid_timeout_is_reported_and_the_plugin_runs_with_the_default() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &with_plugins(&format!("{}  timeout: \"soon\"\n", plugin(&dir, "echo.wat"))));

    let errors = ledger
        .errors
        .iter()
        .map(|it| (it.error_type.clone(), it.span.as_ref().map(|span| span.content.clone()), it.metas.clone()))
        .collect_vec();
    let module = dir.path().join("echo.wat").display().to_string();
    assert_eq!(
        errors,
        vec![(
            ErrorKind::ParseInvalidMeta,
            Some(format!("{}  timeout: \"soon\"", plugin(&dir, "echo.wat"))),
            HashMap::from([("plugin".to_owned(), module), ("timeout".to_owned(), "soon".to_owned())])
        )]
    );
    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the echo processor still runs");
}

#[test]
fn raw_stage_plugin_runs_before_booking() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &with_plugins(&format!("{}  stage: \"raw\"\n", plugin(&dir, "echo.wat"))));

    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);
    let raw = ledger.plugins.build_stages(PluginStage::Raw);
    assert_eq!(raw.iter().map(|it| it.name()).collect_vec(), vec!["echo"]);
    assert!(ledger.plugins.build_stages(PluginStage::Booked).is_empty());
    assert_eq!(store_summary(&ledger).1.len(), 2, "the echo processor still runs");
    assert_eq!(store_summary(&ledger).2, Vec::<String>::new());
}

#[test]
fn invalid_stage_is_reported_and_the_plugin_runs_booked() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &with_plugins(&format!("{}  stage: \"early\"\n", plugin(&dir, "echo.wat"))));

    let errors = ledger
        .errors
        .iter()
        .map(|it| (it.error_type.clone(), it.span.as_ref().map(|span| span.content.clone()), it.metas.clone()))
        .collect_vec();
    let module = dir.path().join("echo.wat").display().to_string();
    assert_eq!(
        errors,
        vec![(
            ErrorKind::ParseInvalidMeta,
            Some(format!("{}  stage: \"early\"", plugin(&dir, "echo.wat"))),
            HashMap::from([("plugin".to_owned(), module), ("stage".to_owned(), "early".to_owned())])
        )]
    );
    assert_eq!(ledger.plugins.build_stages(PluginStage::Booked).len(), 1);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the echo processor still runs");
}

#[test]
fn failed_reload_keeps_the_previous_ledger() {
    let dir = ledger_dir(&["echo.wat", "loop.wat"]);
    let mut ledger = load(&dir, &with_plugins(&plugin(&dir, "echo.wat")));
    let before = store_summary(&ledger);

    std::fs::write(
        dir.path().join("main.zhang"),
        with_plugins(&format!("{}  timeout: \"1\"\n", plugin(&dir, "loop.wat"))),
    )
    .unwrap();
    let Err(error) = ledger.reload() else { panic!("the reload must fail") };

    assert!(error.to_string().contains("plugin loop timed out"), "{error}");
    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);
    assert_eq!(store_summary(&ledger), before);
}

#[test]
fn processor_reads_its_arguments_and_every_meta_value_from_the_zhang_plugin_config() {
    let dir = ledger_dir(&["config_echo.wat"]);
    let module = dir.path().join("config_echo.wat").display().to_string();
    let ledger = load(
        &dir,
        &formatdoc! {r#"
            option "features.plugin" "true"
            plugin "{module}" "USD" "strict"
              tag: "first"
              allowed_hosts: "api.example.com"
              tag: "second"
              zhang.plugin: "from meta"
            {LEDGER}"#},
    );

    // the plugin replaces the stream with one comment holding its `zhang.plugin` config
    let comments = ledger
        .metas
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Comment(comment) => Some(comment.content.clone()),
            _ => None,
        })
        .collect_vec();
    assert_eq!(comments.len(), 1, "the plugin emits one comment: {comments:?}");
    let config: serde_json::Value = serde_json::from_str(&comments[0]).unwrap_or_else(|e| panic!("zhang.plugin should be JSON: {e}: {}", comments[0]));
    assert_eq!(
        config,
        json!({
            "module": module,
            "args": ["USD", "strict"],
            "meta": {
                "allowed_hosts": ["api.example.com"],
                "tag": ["first", "second"],
                "zhang.plugin": ["from meta"],
            },
        })
    );
}

#[test]
fn plugin_reports_an_error_on_its_directive_and_the_load_continues() {
    let dir = ledger_dir(&["emit_error.wat"]);
    let directive = plugin(&dir, "emit_error.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}");
    let ledger = load(&dir, &content);

    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::PluginError);
    // the plugin's own meta is kept; `plugin` names the plugin that reported it, whatever the plugin sent
    assert_eq!(
        error_metas,
        &metas(&[("plugin", "emit-error"), ("message", "payee is missing"), ("rule", "payee-required")])
    );
    assert_directive_span(span, &content, &directive);

    // the processor returned the stream unchanged, and the rest of the load ran
    let (accounts, postings, _) = store_summary(&ledger);
    let (plain_accounts, plain_postings, _) = store_summary(&load(&ledger_dir(&[]), LEDGER));
    assert_eq!((accounts, postings), (plain_accounts, plain_postings));
}

#[test]
fn invalid_error_payload_is_reported_as_a_plugin_error() {
    let dir = ledger_dir(&["invalid_error.wat"]);
    let directive = plugin(&dir, "invalid_error.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}");
    let ledger = load(&dir, &content);

    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::PluginError);
    assert_eq!(error_metas.keys().sorted().collect_vec(), vec!["message", "plugin"]);
    assert_eq!(error_metas["plugin"], "invalid-error");
    assert!(
        error_metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
        "{}",
        error_metas["message"]
    );
    assert_directive_span(span, &content, &directive);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the lunch transaction still reaches the store");
}

#[test]
fn forged_error_payload_is_rejected_without_crashing() {
    // the plugin forges its block header so the kernel reports a ~2 GiB length; an unfixed host
    // slices that much memory and dies with SIGBUS. The fix must keep the load alive and report an
    // invalid payload.
    let dir = ledger_dir(&["emit_error_forge.wat"]);
    let directive = plugin(&dir, "emit_error_forge.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}");
    let ledger = load(&dir, &content);

    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::PluginError);
    assert_eq!(error_metas["plugin"], "emit-error-forge");
    assert!(
        error_metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
        "{}",
        error_metas["message"]
    );
    assert_directive_span(span, &content, &directive);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the load continued past the forged call");
}

#[test]
fn oversized_error_payload_is_rejected_without_crashing() {
    // a real, well-formed block just over the 1 MiB limit: rejected on size, not read, no crash
    let dir = ledger_dir(&["emit_error_toobig.wat"]);
    let directive = plugin(&dir, "emit_error_toobig.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}");
    let ledger = load(&dir, &content);

    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, _span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::PluginError);
    assert_eq!(error_metas["plugin"], "emit-error-toobig");
    assert!(
        error_metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
        "{}",
        error_metas["message"]
    );
    assert_eq!(store_summary(&ledger).1.len(), 2, "the load continued past the oversized call");
}

#[test]
fn root_length_cannot_be_forged_to_bypass_the_emit_error_bound() {
    // the plugin also tries to overwrite the kernel's own MemoryRoot::length (the host's ground
    // truth) to ~2 GiB, then forges a within-cap block that runs past real memory. If the root were
    // writable the host would slice past the mapped memory and crash; the kernel protects the root,
    // so this stays a rejected invalid payload and the load continues.
    let dir = ledger_dir(&["emit_error_root_forge.wat"]);
    let directive = plugin(&dir, "emit_error_root_forge.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}");
    let ledger = load(&dir, &content);

    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, _span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::PluginError);
    assert_eq!(error_metas["plugin"], "emit-error-root-forge");
    assert!(
        error_metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
        "{}",
        error_metas["message"]
    );
    assert_eq!(store_summary(&ledger).1.len(), 2, "the load continued past the root-forge attack");
}

#[test]
fn plugin_without_the_error_import_is_unaffected() {
    // echo imports no host function: it loads next to a plugin that does, and reports nothing
    let dir = ledger_dir(&["echo.wat", "emit_error.wat"]);
    let echo = plugin(&dir, "echo.wat");
    let emit_error = plugin(&dir, "emit_error.wat");
    let content = format!("option \"features.plugin\" \"true\"\n{echo}{emit_error}{LEDGER}");
    let ledger = load(&dir, &content);

    assert_eq!(
        registered(&ledger),
        vec![
            ("echo".to_owned(), vec![PluginType::Processor]),
            ("emit-error".to_owned(), vec![PluginType::Processor])
        ]
    );
    let reported = errors(&ledger);
    assert_eq!(reported.iter().map(|(_, _, metas)| metas["plugin"].as_str()).collect_vec(), vec!["emit-error"]);
    assert_directive_span(&reported[0].1, &content, &emit_error);

    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "echo.wat")));
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn plugin_declaring_router_and_processor_runs_as_both() {
    let dir = ledger_dir(&["router_processor.wat"]);
    let ledger = load(
        &dir,
        &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "router_processor.wat")),
    );

    assert_eq!(
        registered(&ledger),
        vec![("router-processor".to_owned(), vec![PluginType::Router, PluginType::Processor])]
    );
    assert!(ledger.plugins.router("router-processor").is_some(), "it serves its route");
    // the processor still runs: it drops the whole stream
    assert_eq!(store_summary(&ledger), (vec![], vec![], vec![]));
}

/// A router host answering every query with `{"echo": <bql>}`, or with a failure for `FAIL`. With
/// parameters it binds them as the server does, with the engine's `bind_json`, compiles the query
/// against their types, and echoes the bound values written as BQL: `{"echo": <bql>, "params":
/// {"from": "2024-01-01"}}`. The schema is a fixed one.
struct FakeHost;

impl RouterHost for FakeHost {
    fn query(&self, bql: &str) -> Result<serde_json::Value, QueryFailure> {
        if bql == "FAIL" {
            return Err(QueryFailure {
                message: "no".to_owned(),
                line: Some(1),
                column: Some(2),
            });
        }
        Ok(json!({ "echo": bql }))
    }

    fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<serde_json::Value, QueryFailure> {
        let (types, values) =
            bind_json(params.iter().map(|(name, param)| (name.as_str(), param.ty.as_str(), &param.value))).map_err(|message| QueryFailure {
                message,
                line: None,
                column: None,
            })?;
        Query::compile_with_params(bql, &types).map_err(|error: QueryError| QueryFailure {
            message: error.message,
            line: error.line,
            column: error.column,
        })?;
        let bound: BTreeMap<&str, Option<String>> = params
            .keys()
            .map(|name| (name.as_str(), values.get(&ParamRef::Named(name.clone())).and_then(to_bql)))
            .collect();
        Ok(json!({ "echo": bql, "params": bound }))
    }

    fn schema(&self) -> Result<serde_json::Value, QueryFailure> {
        Ok(json!({"tables": [{"name": "postings"}], "functions": [{"name": "sum"}]}))
    }

    /// echoes the plugin and the batch: the token is the batch's directives as JSON
    fn write_preview(&self, plugin: &str, batch: &WriteBatch) -> Result<WritePreview, HostError> {
        Ok(WritePreview {
            ok: true,
            file: format!("{plugin}.zhang"),
            generation: 1,
            entries: vec![],
            token: Some(serde_json::Value::Array(batch.directives.clone()).to_string()),
        })
    }

    /// echoes the token as the file
    fn write_commit(&self, plugin: &str, batch: &WriteBatch, token: &str) -> Result<WriteCommitted, HostError> {
        Ok(WriteCommitted {
            file: format!("{plugin}:{token}"),
            bytes: 0,
            entries: batch.directives.len(),
            generation: 2,
        })
    }
}

/// a ledger declaring the given router fixtures, with plugins on
fn router_ledger(fixtures: &[&str]) -> (TempDir, Ledger) {
    let dir = ledger_dir(fixtures);
    let plugins = fixtures.iter().map(|fixture| plugin(&dir, fixture)).collect::<String>();
    let ledger = load(
        &dir,
        &format!("option \"features.plugin\" \"true\"\noption \"title\" \"Home\"\n{plugins}{LEDGER}"),
    );
    (dir, ledger)
}

fn call(ledger: &Ledger, name: &str, request: &PluginRequest) -> Result<http::Response<Vec<u8>>, RouterError> {
    let plugin = ledger.plugins.router(name).unwrap_or_else(|| panic!("{name} should be a router"));
    plugin.execute_as_router(request, ledger, Arc::new(FakeHost))
}

#[test]
fn router_answers_with_its_response() {
    let (_dir, ledger) = router_ledger(&["router.wat"]);
    let request = PluginRequest::new(
        "POST",
        "/sub/path",
        vec![("k".to_owned(), "v1".to_owned()), ("k".to_owned(), "v2".to_owned())],
        vec![("X-Custom".to_owned(), "yes".to_owned())],
        b"{\"amount\": \"10 \\\"CNY\\\"\"}".to_vec(),
    );

    let response = call(&ledger, "router-echo", &request).unwrap();

    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()["x-echo"], "router");
    assert_eq!(response.headers()["content-type"], "application/json");
    let echoed: PluginRequest = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(echoed, request);
}

#[test]
fn router_reads_the_ledger_through_host_functions_only_while_routing() {
    // loading proves the processor got an `Err` from both host functions: it traps otherwise
    let (_dir, ledger) = router_ledger(&["router_query.wat"]);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the processor passed the stream through");

    let response = call(&ledger, "router-query", &PluginRequest::new("GET", "/", vec![], vec![], vec![])).unwrap();

    assert_eq!(response.status(), 200, "the default status");
    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(
        body,
        json!([
            {"Ok": {"echo": "SELECT account, sum(position) AS balance GROUP BY account ORDER BY account"}},
            {"Ok": {"title": "Home", "operating_currency": "CNY", "timezone": ledger.options.timezone.name()}},
        ])
    );
    // what it reported with `zhang_emit_error` during the request is only logged
    assert_eq!(errors(&ledger), vec![]);
}

/// a meeting point for `parties` threads that gives up after `timeout`, so a request that never
/// arrives fails the test instead of hanging it
struct Rendezvous {
    parties: usize,
    arrived: std::sync::Mutex<usize>,
    complete: std::sync::Condvar,
}

impl Rendezvous {
    fn new(parties: usize) -> Self {
        Rendezvous {
            parties,
            arrived: std::sync::Mutex::new(0),
            complete: std::sync::Condvar::new(),
        }
    }

    /// wait until every party arrived; `false` when `timeout` passed first
    fn wait(&self, timeout: Duration) -> bool {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        if *arrived >= self.parties {
            self.complete.notify_all();
            return true;
        }
        let (_arrived, result) = self.complete.wait_timeout_while(arrived, timeout, |arrived| *arrived < self.parties).unwrap();
        !result.timed_out()
    }
}

#[test]
fn concurrent_router_calls_use_their_own_query_hosts() {
    /// a host answering every query with the marker of its own request, once every request is inside
    /// the host at the same time
    struct PerRequestHost {
        marker: usize,
        rendezvous: Arc<Rendezvous>,
    }
    impl RouterHost for PerRequestHost {
        fn query(&self, _bql: &str) -> Result<serde_json::Value, QueryFailure> {
            // the calls overlap inside the host: the host functions the instances share must neither
            // hold a lock while a query runs nor answer with another request's host
            if !self.rendezvous.wait(Duration::from_secs(10)) {
                return Err(QueryFailure {
                    message: "the requests did not overlap inside the host within 10 s".to_owned(),
                    line: None,
                    column: None,
                });
            }
            Ok(json!({"marker": self.marker}))
        }

        fn query_with_params(&self, _bql: &str, _params: &BTreeMap<String, RawQueryParam>) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }

        fn schema(&self) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }
    }
    let (_dir, ledger) = router_ledger(&["router_query.wat"]);
    let rendezvous = Arc::new(Rendezvous::new(4));
    std::thread::scope(|scope| {
        for marker in 0..4 {
            let ledger = &ledger;
            let rendezvous = rendezvous.clone();
            scope.spawn(move || {
                let router = ledger.plugins.router("router-query").unwrap();
                let host = Arc::new(PerRequestHost { marker, rendezvous });
                let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);
                let response = router.execute_as_router(&request, ledger, host).unwrap();
                let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
                assert_eq!(body[0], json!({"Ok": {"marker": marker}}));
            });
        }
    });
    assert_eq!(errors(&ledger), vec![], "router errors cannot leak into the ledger");
}

#[test]
fn a_request_timing_out_does_not_interrupt_a_concurrent_one() {
    /// a host whose every query takes this long
    struct SlowHost(Duration);
    impl RouterHost for SlowHost {
        fn query(&self, _bql: &str) -> Result<serde_json::Value, QueryFailure> {
            std::thread::sleep(self.0);
            Ok(json!({"slept_ms": self.0.as_millis() as u64}))
        }

        fn query_with_params(&self, _bql: &str, _params: &BTreeMap<String, RawQueryParam>) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }

        fn schema(&self) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }
    }
    let dir = ledger_dir(&["router_query.wat"]);
    let ledger = load(
        &dir,
        &format!(
            "option \"features.plugin\" \"true\"\n{}  timeout: \"1s\"\n{LEDGER}",
            plugin(&dir, "router_query.wat")
        ),
    );
    let router = ledger.plugins.router("router-query").unwrap();
    let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);
    // the slow request is stuck in its host for 2.2 s, so its instance is stopped at 1 s; the quick
    // one starts at 0.6 s and runs 0.6 s, so the slow one's deadline passes while it runs, well
    // inside its own 1 s. extism stops a call by bumping its engine's epoch: with one compiled
    // plugin (one engine) per registration the quick request would be stopped too
    let (slow, quick, quick_took) = std::thread::scope(|scope| {
        let slow = scope.spawn(|| router.execute_as_router(&request, &ledger, Arc::new(SlowHost(Duration::from_millis(2200)))));
        std::thread::sleep(Duration::from_millis(600));
        let started = Instant::now();
        let quick = router.execute_as_router(&request, &ledger, Arc::new(SlowHost(Duration::from_millis(600))));
        (slow.join().unwrap(), quick, started.elapsed())
    });
    assert_eq!(slow.unwrap_err(), RouterError::Timeout);
    let quick = quick.unwrap_or_else(|e| panic!("the quick request must not be stopped by the slow one's timeout: {e} (after {quick_took:?})"));
    assert_eq!(quick.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(quick.body()).unwrap();
    assert_eq!(body[0], json!({"Ok": {"slept_ms": 600}}));
}

#[test]
fn a_panicking_host_call_does_not_hand_its_engine_to_the_next_request() {
    /// a host whose query panics, as a bug in the query engine would, or else takes this long
    struct Host(Option<Duration>);
    impl RouterHost for Host {
        fn query(&self, _bql: &str) -> Result<serde_json::Value, QueryFailure> {
            let Some(duration) = self.0 else { panic!("a bug in the host") };
            std::thread::sleep(duration);
            Ok(json!({"slept_ms": duration.as_millis() as u64}))
        }

        fn query_with_params(&self, _bql: &str, _params: &BTreeMap<String, RawQueryParam>) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }

        fn schema(&self) -> Result<serde_json::Value, QueryFailure> {
            unreachable!("the fixture only calls zhang_query")
        }
    }
    let dir = ledger_dir(&["router_query.wat"]);
    let ledger = load(
        &dir,
        &format!(
            "option \"features.plugin\" \"true\"\n{}  timeout: \"1s\"\n{LEDGER}",
            plugin(&dir, "router_query.wat")
        ),
    );
    let router = ledger.plugins.router("router-query").unwrap();
    let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);
    // one normal request, so the panicking one leases a compiled plugin the pool would get back
    assert_eq!(
        router
            .execute_as_router(&request, &ledger, Arc::new(Host(Some(Duration::ZERO))))
            .unwrap()
            .status(),
        200
    );
    // the panic unwinds through extism before it stops the call's timer (the server's
    // `spawn_blocking` catches it in production), so the engine's epoch is still bumped at 1 s
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        router.execute_as_router(&request, &ledger, Arc::new(Host(None)))
    }));
    assert!(panicked.is_err(), "the host panic unwinds through the call");
    std::thread::sleep(Duration::from_millis(400));
    // the next request needs 0.8 s of its own 1 s and runs across the panicked call's deadline
    let next = router.execute_as_router(&request, &ledger, Arc::new(Host(Some(Duration::from_millis(800)))));
    let next = next.unwrap_or_else(|e| panic!("the next request must not be stopped by the panicked call's timer: {e}"));
    assert_eq!(next.status(), 200);
}

#[test]
fn reloading_a_changed_module_replaces_its_compiled_code() {
    let (dir, mut ledger) = router_ledger(&["router.wat"]);
    let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);
    assert_eq!(call(&ledger, "router-echo", &request).unwrap().headers()["x-echo"], "router");
    let old_plugin = ledger.plugins.router("router-echo").unwrap().clone();

    let module = dir.path().join("router.wat");
    let original = std::fs::read_to_string(&module).unwrap();
    // the replacement has the same length, which keeps the fixture's data offsets valid
    let changed = original.replace(r#"\"x-echo\":\"router\""#, r#"\"x-echo\":\"newone\""#);
    assert_ne!(original, changed);
    std::fs::write(module, changed).unwrap();
    ledger.reload().unwrap();

    assert_eq!(call(&ledger, "router-echo", &request).unwrap().headers()["x-echo"], "newone");
    // a registration taken before the reload owns its own module and compiled code, and still runs
    let old_response = old_plugin.execute_as_router(&request, &ledger, Arc::new(FakeHost)).unwrap();
    assert_eq!(old_response.headers()["x-echo"], "router");
}

/// what `router-query-params` answers for the request path `/{case}`: the raw host function result
fn query_params_case(ledger: &Ledger, case: char) -> serde_json::Value {
    let request = PluginRequest::new("GET", format!("/{case}"), vec![], vec![], vec![]);
    // the fixture reads the case from byte 25 of the request JSON
    assert_eq!(serde_json::to_vec(&request).unwrap()[25], case as u8);
    let response = call(ledger, "router-query-params", &request).unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(response.body()).unwrap()
}

#[test]
fn router_binds_query_parameters_through_the_host() {
    // loading proves the processor got an `Err` from both host functions: it traps otherwise
    let (_dir, ledger) = router_ledger(&["router_query_params.wat"]);
    assert_eq!(store_summary(&ledger).1.len(), 2, "the processor passed the stream through");

    // a date, a str and a decimal given as a JSON number bind with their declared types
    assert_eq!(
        query_params_case(&ledger, 'a'),
        json!({"Ok": {
            "echo": "SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min",
            "params": {"from": "2024-01-01", "who": "'lunch'", "min": "5.5"},
        }})
    );
    // null is NULL, and the query still compiles against the declared type
    assert_eq!(
        query_params_case(&ledger, 'b'),
        json!({"Ok": {"echo": "SELECT date WHERE narration = :who", "params": {"who": "NULL"}}})
    );
}

#[test]
fn router_query_parameter_problems_are_query_errors() {
    let (_dir, ledger) = router_ledger(&["router_query_params.wat"]);

    assert_eq!(
        query_params_case(&ledger, 'c'),
        json!({"Err": {
            "kind": "query",
            "message": "parameter :who has the type \"money\", which is not one of bool, int, decimal, str, date, set",
            "line": null,
            "column": null,
        }})
    );
    assert_eq!(
        query_params_case(&ledger, 'd'),
        json!({"Err": {"kind": "query", "message": "parameter :from must be a date string YYYY-MM-DD or null", "line": null, "column": null}})
    );
    // a parameter the query uses but the input does not bind is reported where the query uses it
    assert_eq!(
        query_params_case(&ledger, 'e'),
        json!({"Err": {"kind": "query", "message": "parameter :from is not bound", "line": 1, "column": 27}})
    );
}

#[test]
fn router_query_params_input_that_cannot_be_read_is_invalid_input() {
    let (_dir, ledger) = router_ledger(&["router_query_params.wat"]);

    let not_json = query_params_case(&ledger, 'f');
    assert_eq!(not_json["Err"]["kind"], json!("invalid_input"), "{not_json}");
    assert!(
        not_json["Err"]["message"].as_str().unwrap().starts_with("the input is not a JSON object"),
        "{not_json}"
    );
    // the whole input is capped like a query's text, and a block over the cap is never read
    let oversize = query_params_case(&ledger, 'g');
    assert_eq!(oversize["Err"]["kind"], json!("invalid_input"), "{oversize}");
    assert_eq!(
        oversize["Err"]["message"],
        json!("the input is 1048577 bytes, over the 1048576 byte limit"),
        "{oversize}"
    );
}

#[test]
fn router_reads_the_query_schema_through_the_host() {
    let (_dir, ledger) = router_ledger(&["router_query_params.wat"]);

    assert_eq!(
        query_params_case(&ledger, 's'),
        json!({"Ok": {"tables": [{"name": "postings"}], "functions": [{"name": "sum"}]}})
    );
}

#[test]
fn forged_query_input_returns_invalid_input() {
    // the router forges its block header so the kernel reports a ~2 GiB length; an unfixed host
    // slices that much memory in `zhang_query` and dies with SIGBUS. The fix must answer the plugin
    // with an `invalid_input` value and never crash.
    let (_dir, ledger) = router_ledger(&["router_query_forge.wat"]);

    let response = call(&ledger, "router-query-forge", &PluginRequest::new("GET", "/", vec![], vec![], vec![])).unwrap();

    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["Err"]["kind"], json!("invalid_input"), "{body}");
}

#[test]
fn forged_query_overread_is_rejected_not_echoed() {
    // the router forges its block header to ~64 KiB, just past the real data region. This does not
    // crash an unfixed host but makes `zhang_query` read ~64 KiB of the plugin's own kernel memory,
    // which the fake host echoes straight back. The fix must reject it as `invalid_input` instead.
    let (_dir, ledger) = router_ledger(&["router_query_overread.wat"]);

    let response = call(&ledger, "router-query-overread", &PluginRequest::new("GET", "/", vec![], vec![], vec![])).unwrap();

    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["Err"]["kind"], json!("invalid_input"), "{body}");
    assert!(
        response.body().len() < 1024,
        "the leaked memory must not be echoed back: {} bytes",
        response.body().len()
    );
}

#[test]
fn root_length_cannot_be_forged_to_bypass_the_query_bound() {
    // the router also tries to overwrite the kernel's own MemoryRoot::length (the host's ground
    // truth) to ~2 GiB, then forges a within-cap block that runs past real memory. If the root were
    // writable the host would slice past the mapped memory and crash in `zhang_query`; the kernel
    // protects the root, so the call is answered with `invalid_input` and never crashes.
    let (_dir, ledger) = router_ledger(&["router_query_root_forge.wat"]);

    let response = call(&ledger, "router-query-root-forge", &PluginRequest::new("GET", "/", vec![], vec![], vec![])).unwrap();

    let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["Err"]["kind"], json!("invalid_input"), "{body}");
}

#[test]
fn looping_router_stops_at_its_timeout() {
    let dir = ledger_dir(&["router_loop.wat"]);
    let ledger = load(&dir, &with_plugins(&format!("{}  timeout: \"500ms\"\n", plugin(&dir, "router_loop.wat"))));
    let started = Instant::now();

    let result = call(&ledger, "router-loop", &PluginRequest::new("GET", "/", vec![], vec![], vec![]));

    assert_eq!(result.unwrap_err(), RouterError::Timeout);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(10), "the call took {elapsed:?}, not about half a second");
}

#[test]
fn router_failures_are_errors_not_panics() {
    let (_dir, ledger) = router_ledger(&["router_no_export.wat", "router_malformed.wat", "router_trap.wat"]);
    let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);

    assert_eq!(call(&ledger, "router-no-export", &request).unwrap_err(), RouterError::NoRouterExport);
    assert!(matches!(
        call(&ledger, "router-malformed", &request).unwrap_err(),
        RouterError::BadResponse(message) if message.starts_with("expected ident")
    ));
    assert!(matches!(
        call(&ledger, "router-trap", &request).unwrap_err(),
        RouterError::Failed(message) if message.contains("unreachable")
    ));
}

#[test]
fn the_first_declared_router_serves_a_shared_name() {
    let dir = ledger_dir(&["router.wat"]);
    let module = dir.path().join("router.wat");
    let content = format!(
        "option \"features.plugin\" \"true\"\nplugin \"{module}\"\n  allowed_hosts: \"first.example\"\nplugin \"{module}\"\n  allowed_hosts: \"second.example\"\n{LEDGER}",
        module = module.display()
    );
    let ledger = load(&dir, &content);

    assert_eq!(ledger.plugins.routers.len(), 2);
    let router = ledger.plugins.router("router-echo").unwrap();
    assert_eq!(router.capabilities().allowed_hosts, vec!["first.example".to_owned()]);
    assert!(ledger.plugins.router("router").is_none());
}

/// the instant tests fix the clock at: 00:30 the next day in Asia/Shanghai
fn fixed_now() -> DateTime<Utc> {
    "2024-03-15T16:30:00Z".parse().unwrap()
}

/// what the `now_echo.wat` plugins of a ledger got from `zhang_now`, in declaration order
fn now_results(ledger: &Ledger) -> Vec<serde_json::Value> {
    comments(ledger)
        .iter()
        .map(|it| serde_json::from_str(it).unwrap_or_else(|e| panic!("zhang_now should return JSON: {e}: {it}")))
        .collect()
}

#[test]
fn processor_reads_the_load_time_in_the_ledger_timezone_from_zhang_now() {
    let dir = ledger_dir(&["now_echo.wat"]);
    let content = format!("option \"timezone\" \"Asia/Shanghai\"\n{}", with_plugins(&plugin(&dir, "now_echo.wat")));
    let mut ledger = load_with_clock(&dir, &content, Clock::Fixed(fixed_now()));

    let expected = json!({"Ok": {"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}});
    assert_eq!(now_results(&ledger), vec![expected.clone()]);
    assert!(ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);
    assert_eq!(ledger.clock_reading().map(|it| it.to_rfc3339()).as_deref(), Some("2024-03-16T00:30:00+08:00"));
    assert_eq!(
        store_summary(&ledger).1.len(),
        2,
        "the plugin appends to the stream, the lunch transaction still lands"
    );

    // a reload reads the same clock again
    ledger.reload().unwrap();
    assert_eq!(ledger.clock(), Clock::Fixed(fixed_now()));
    assert_eq!(now_results(&ledger), vec![expected]);
    assert!(ledger.extra_inputs.contains(&ExtraInput::Clock));
}

#[test]
fn every_plugin_of_a_load_reads_the_same_time() {
    // the system clock is read once per load, on the first call, so two plugins agree on the instant
    let dir = ledger_dir(&["now_echo.wat"]);
    let now_echo = plugin(&dir, "now_echo.wat");
    let ledger = load(&dir, &with_plugins(&format!("{now_echo}{now_echo}")));

    let results = now_results(&ledger);
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0], results[1]);
    let reading = ledger.clock_reading().expect("the load read the clock");
    assert_eq!(results[0]["Ok"]["now"], reading.to_rfc3339());
    assert_eq!(results[0]["Ok"]["today"], reading.format("%Y-%m-%d").to_string());
    assert_eq!(ledger.clock(), Clock::System);
}

#[test]
fn plugin_that_never_calls_zhang_now_does_not_read_the_clock() {
    let dir = ledger_dir(&["echo.wat"]);
    let ledger = load(&dir, &with_plugins(&plugin(&dir, "echo.wat")));

    assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);
    assert_eq!(ledger.clock_reading(), None, "nothing asked for the time");
}

#[test]
fn reading_the_time_while_registering_does_not_make_the_ledger_depend_on_the_date() {
    let dir = ledger_dir(&["now_at_registration.wat"]);
    let ledger = load_with_clock(&dir, &with_plugins(&plugin(&dir, "now_at_registration.wat")), Clock::Fixed(fixed_now()));

    assert_eq!(registered(&ledger), vec![("now-at-registration".to_owned(), vec![PluginType::Processor])]);
    assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);
    assert_eq!(
        ledger.clock_reading().map(|it| it.with_timezone(&Utc)),
        Some(fixed_now()),
        "registering got the time of the load"
    );
    assert_eq!(store_summary(&ledger).1.len(), 2);
}

/// the `zhang.seed` values the `seed_echo.wat` plugins of a ledger received, in declaration order
fn seeds(ledger: &Ledger) -> Vec<u64> {
    comments(ledger)
        .iter()
        .map(|it| it.parse().unwrap_or_else(|e| panic!("zhang.seed should be a decimal u64: {e}: {it}")))
        .collect()
}

#[test]
fn plugin_seed_is_stable_across_loads_and_unique_per_declaration() {
    let dir = ledger_dir(&["seed_echo.wat", "echo.wat"]);
    let module = dir.path().join("seed_echo.wat").display().to_string();
    let seed_echo = plugin(&dir, "seed_echo.wat");

    let first = seeds(&load(&dir, &with_plugins(&seed_echo)));
    assert_eq!(first, vec![plugin_seed(&module, 0, None)]);
    assert_eq!(seeds(&load(&dir, &with_plugins(&seed_echo))), first, "every load gets the same seed");

    // a second declaration of the module gets its own seed, and the first keeps its own
    let twice = seeds(&load(&dir, &with_plugins(&format!("{seed_echo}{seed_echo}"))));
    assert_eq!(twice, vec![plugin_seed(&module, 0, None), plugin_seed(&module, 1, None)]);
    assert_ne!(twice[0], twice[1]);

    // a `seed` meta changes it
    let seeded = seeds(&load(&dir, &with_plugins(&format!("{seed_echo}  seed: \"ids\"\n"))));
    assert_eq!(seeded, vec![plugin_seed(&module, 0, Some("ids"))]);
    assert_ne!(seeded, first);

    // a plugin of another module declared before it changes nothing
    let after_echo = seeds(&load(&dir, &with_plugins(&format!("{}{seed_echo}", plugin(&dir, "echo.wat")))));
    assert_eq!(after_echo, first);
}

#[test]
fn router_reads_the_clock_for_each_request_and_records_nothing() {
    let dir = ledger_dir(&["router_now.wat"]);
    let content = format!("option \"timezone\" \"Asia/Shanghai\"\n{}", with_plugins(&plugin(&dir, "router_now.wat")));
    let ledger = load_with_clock(&dir, &content, Clock::Fixed(fixed_now()));
    assert_eq!(ledger.clock_reading(), None, "loading a router plugin reads no clock");

    let request = PluginRequest::new("GET", "/", vec![], vec![], vec![]);
    let response = call(&ledger, "router-now", &request).unwrap();
    let now: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(
        now,
        json!({"Ok": {"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}})
    );
    // a request is not a load: the ledger does not start depending on the date
    assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);
    assert_eq!(ledger.clock_reading(), None);
}

/// a ledger directory with the `files.wat` fixture, `documents/receipt.txt`, `documents/2024/a.csv` and
/// `documents-private/secret.txt`
fn documents_dir() -> TempDir {
    let dir = ledger_dir(&["files.wat"]);
    for (file, content) in [
        ("documents/receipt.txt", "lunch 10 CNY\n"),
        ("documents/2024/a.csv", "a,b"),
        ("documents-private/secret.txt", "secret"),
    ] {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

/// the content of the one comment a plugin replaced the stream with
fn single_comment(ledger: &Ledger) -> String {
    let comments = ledger
        .metas
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Comment(comment) => Some(comment.content.clone()),
            _ => None,
        })
        .collect_vec();
    assert_eq!(comments.len(), 1, "the plugin emits one comment: {comments:?}");
    comments.into_iter().next().unwrap()
}

/// the `files.wat` plugin declared with the `meta` lines (each `key: "value"`), loaded from `source`
fn files_plugin_from(dir: &TempDir, meta: &[(&str, &str)], source: Arc<dyn DataSource>) -> (Ledger, serde_json::Value) {
    let meta = meta.iter().map(|(key, value)| format!("  {key}: \"{value}\"\n")).join("");
    let ledger = try_load_from(dir, &with_plugins(&format!("{}{meta}", plugin(dir, "files.wat"))), source)
        .unwrap_or_else(|e| panic!("a file call never fails the load: {e}"));
    let answer = single_comment(&ledger);
    let answer = serde_json::from_str(&answer).unwrap_or_else(|e| panic!("the host answers JSON: {e}: {answer}"));
    (ledger, answer)
}

/// the `files.wat` plugin declared with the `meta` lines, loaded from the local disk
fn files_plugin(dir: &TempDir, meta: &[(&str, &str)]) -> (Ledger, serde_json::Value) {
    files_plugin_from(dir, meta, Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})))
}

/// the extra inputs a load recorded besides plugin modules
fn read_inputs(ledger: &Ledger) -> Vec<ExtraInput> {
    ledger
        .extra_inputs
        .iter()
        .filter(|it| !matches!(it, ExtraInput::File(path) if path.extension().is_some_and(|it| it == "wat")))
        .cloned()
        .collect()
}

fn error_kind(answer: &serde_json::Value) -> &str {
    answer["Err"]["kind"].as_str().unwrap_or_else(|| panic!("an error answer: {answer}"))
}

#[test]
fn plugin_reads_a_granted_file_and_it_becomes_an_extra_input() {
    let dir = documents_dir();
    let meta = [("allowed_paths", "documents"), ("read", "./documents//receipt.txt")];
    let (mut ledger, answer) = files_plugin(&dir, &meta);

    assert_eq!(answer, json!({"Ok": {"content": "lunch 10 CNY\n", "encoding": "utf8"}}));
    assert_eq!(read_inputs(&ledger), vec![ExtraInput::File("documents/receipt.txt".into())]);
    assert_eq!(errors(&ledger), vec![]);
    // the receipt is no ledger file: the file editor does not list it
    assert!(ledger.visited_files.iter().all(|it| !it.ends_with("receipt.txt")));

    // a reload, as a change to the receipt triggers, reads the new content
    std::fs::write(dir.path().join("documents/receipt.txt"), "lunch 12 CNY\n").unwrap();
    ledger.reload().unwrap();
    assert_eq!(single_comment(&ledger), r#"{"Ok":{"content":"lunch 12 CNY\n","encoding":"utf8"}}"#);
}

#[test]
fn denied_read_is_a_value_not_a_trap() {
    let dir = documents_dir();
    let (ledger, answer) = files_plugin(&dir, &[("read", "documents/receipt.txt")]);

    assert_eq!(error_kind(&answer), "denied");
    assert!(
        answer["Err"]["message"].as_str().unwrap().contains("allowed_paths"),
        "the message says how to grant it: {answer}"
    );
    assert_eq!(read_inputs(&ledger), vec![], "a denied path is not an input");
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn listing_is_recorded_as_a_dir_input() {
    let dir = documents_dir();
    let (ledger, answer) = files_plugin(&dir, &[("allowed_paths", "documents"), ("list", "documents")]);

    assert_eq!(
        answer,
        json!({"Ok": {"entries": [{"name": "2024", "kind": "dir"}, {"name": "receipt.txt", "kind": "file"}]}})
    );
    assert_eq!(read_inputs(&ledger), vec![ExtraInput::Dir("documents".into())]);
}

#[test]
fn file_access_security_rules_hold_through_the_host_functions() {
    let dir = documents_dir();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("passwd"), "root").unwrap();
    let absolute = dir.path().join("documents/receipt.txt").display().to_string();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("receipt.txt", dir.path().join("documents/latest.txt")).unwrap();
        symlink("../documents-private/secret.txt", dir.path().join("documents/escape.txt")).unwrap();
        symlink(outside.path().join("passwd"), dir.path().join("documents/passwd")).unwrap();
    }
    std::fs::File::create(dir.path().join("documents/big.pdf"))
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    std::fs::write(dir.path().join("documents/.secret"), "token").unwrap();

    let mut cases = vec![
        ("../outside.txt", "denied"),
        ("documents/../documents-private/secret.txt", "denied"),
        (absolute.as_str(), "denied"),
        ("documents-private/secret.txt", "denied"),
        ("documents/big.pdf", "too_large"),
        ("documents/missing.pdf", "not_found"),
        ("documents/.secret", "denied"),
        ("documents/receipt.txt/", "invalid"),
    ];
    if cfg!(unix) {
        cases.extend([("documents/escape.txt", "denied"), ("documents/passwd", "denied")]);
    }
    for (path, kind) in cases {
        let (_, answer) = files_plugin(&dir, &[("allowed_paths", "documents"), ("read", path)]);
        assert_eq!(error_kind(&answer), kind, "reading {path}: {answer}");
    }

    if cfg!(unix) {
        let (_, answer) = files_plugin(&dir, &[("allowed_paths", "documents"), ("read", "documents/latest.txt")]);
        assert_eq!(
            answer,
            json!({"Ok": {"content": "lunch 10 CNY\n", "encoding": "utf8"}}),
            "a symlink inside the grant"
        );
    }
}

#[test]
fn whole_root_grant_keeps_hidden_files_hidden() {
    let dir = documents_dir();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/config"), "[remote] url = https://token@example.com").unwrap();
    std::fs::write(dir.path().join(".env"), "TOKEN=1").unwrap();

    for path in [".git/config", ".env"] {
        let (_, answer) = files_plugin(&dir, &[("allowed_paths", "."), ("read", path)]);
        assert_eq!(error_kind(&answer), "denied", "reading {path}: {answer}");
    }
    let (_, answer) = files_plugin(&dir, &[("allowed_paths", "."), ("list", ".")]);
    let names = answer["Ok"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing: {answer}"))
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect_vec();
    assert!(names.iter().all(|it| !it.starts_with('.')), "{names:?}");
    assert!(names.contains(&"documents".to_owned()), "{names:?}");

    let (_, answer) = files_plugin(&dir, &[("allowed_paths", ".git"), ("read", ".git/config")]);
    assert_eq!(answer["Ok"]["encoding"], "utf8", "a grant naming the hidden directory: {answer}");
}

/// a source reading the ledger directory the way a remote one (S3, WebDAV, GitHub) does: it has no local root,
/// fetches files with `get` by their path relative to the ledger root, and cannot list directories
struct RemoteLike {
    root: PathBuf,
    local: LocalFileSystemDataSource,
}

impl DataSource for RemoteLike {
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        let path = Path::new(&path);
        let path = if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        self.local.get(path.display().to_string())
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        self.local.load(entry, endpoint)
    }
}

#[test]
fn remote_source_reads_through_get_and_cannot_list() {
    let dir = documents_dir();
    let remote = || -> Arc<dyn DataSource> {
        Arc::new(RemoteLike {
            root: dir.path().to_path_buf(),
            local: LocalFileSystemDataSource::new(ZhangDataType {}),
        })
    };

    let (ledger, answer) = files_plugin_from(&dir, &[("allowed_paths", "documents"), ("read", "documents/receipt.txt")], remote());
    assert_eq!(answer, json!({"Ok": {"content": "lunch 10 CNY\n", "encoding": "utf8"}}));
    assert_eq!(read_inputs(&ledger), vec![ExtraInput::File("documents/receipt.txt".into())]);

    let (_, answer) = files_plugin_from(&dir, &[("allowed_paths", "documents"), ("list", "documents")], remote());
    assert_eq!(error_kind(&answer), "unsupported");

    for (path, kind) in [("documents-private/secret.txt", "denied"), ("documents/../main.zhang", "denied")] {
        let (_, answer) = files_plugin_from(&dir, &[("allowed_paths", "documents"), ("read", path)], remote());
        assert_eq!(error_kind(&answer), kind, "reading {path}: {answer}");
    }
}

#[test]
fn invalid_allowed_paths_value_is_reported_and_grants_nothing() {
    let dir = documents_dir();
    let absolute = dir.path().join("documents").display().to_string();
    let (ledger, answer) = files_plugin(&dir, &[("allowed_paths", &absolute), ("read", "documents/receipt.txt")]);

    assert_eq!(error_kind(&answer), "denied");
    let module = dir.path().join("files.wat").display().to_string();
    let errors = errors(&ledger);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let (kind, span, error_metas) = &errors[0];
    assert_eq!(kind, &ErrorKind::ParseInvalidMeta);
    assert_eq!(error_metas, &metas(&[("plugin", &module), ("allowed_paths", &absolute)]));
    assert!(
        span.content.starts_with(&plugin(&dir, "files.wat")),
        "on the plugin directive: {}",
        span.content
    );
}

#[test]
fn forged_path_block_is_invalid_and_the_load_continues() {
    // the plugin forges its block header so the kernel reports a ~2 GiB length and passes that block to
    // `zhang_read_file` as the path; a host trusting the length slices that much memory and dies with
    // SIGBUS. The fix answers `invalid` and the load goes on, even with every file granted.
    let dir = ledger_dir(&["read_file_forge.wat"]);
    let ledger = load(&dir, &with_plugins(&format!("{}  allowed_paths: \".\"\n", plugin(&dir, "read_file_forge.wat"))));

    let answer: serde_json::Value = serde_json::from_str(&single_comment(&ledger)).unwrap();
    assert_eq!(error_kind(&answer), "invalid", "{answer}");
    assert!(answer["Err"]["message"].as_str().unwrap().starts_with("the path is "), "{answer}");
    assert_eq!(read_inputs(&ledger), vec![]);
}

#[test]
fn router_gets_no_file_access_even_with_allowed_paths() {
    let dir = documents_dir();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugins/router_files.wat"),
        dir.path().join("router_files.wat"),
    )
    .unwrap();
    let directive = format!(
        "{}  allowed_paths: \"documents\"\n  read: \"documents/receipt.txt\"\n",
        plugin(&dir, "router_files.wat")
    );
    let ledger = load(&dir, &with_plugins(&directive));

    let response = call(&ledger, "router-files", &PluginRequest::new("GET", "/", vec![], vec![], vec![])).unwrap();

    let answer: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(error_kind(&answer), "denied", "{answer}");
    assert!(answer["Err"]["message"].as_str().unwrap().contains("processor or mapper"), "{answer}");
}

const PADDED: &str = indoc! {r#"
    1970-01-01 commodity CNY
    1970-01-01 open Assets:Cash
    1970-01-01 open Equity:Open
    1970-01-01 open Expenses:Food
    2024-01-01 pad Assets:Cash Equity:Open
    2024-01-02 * "lunch"
      Assets:Cash -10 CNY
      Expenses:Food 10 CNY
    2024-02-01 balance Assets:Cash 100 CNY
"#};

#[test]
fn a_plugin_of_the_oldest_contract_loads_a_ledger_with_a_pad() {
    // `old_contract.wat` fails on a `pad` directive, like a plugin built before `pad` existed
    let dir = ledger_dir(&["old_contract.wat"]);
    let ledger = load(
        &dir,
        &format!("option \"features.plugin\" \"true\"\n{}{PADDED}", plugin(&dir, "old_contract.wat")),
    );
    assert_eq!(
        registered(&ledger),
        vec![("old-contract".to_owned(), vec![PluginType::Processor, PluginType::Mapper])]
    );

    // the pad still works: it pads the 110 the cash lacks on the day of the pad, and the balance holds
    assert_eq!(store_summary(&ledger), store_summary(&load(&ledger_dir(&[]), PADDED)));
    assert!(errors(&ledger).is_empty(), "{:?}", errors(&ledger));
    let (date, booked) = paddings(&ledger)[0];
    assert_eq!(date.to_string(), "2024-01-01");
    assert_eq!(written(booked)[0].1.to_string(), "110 CNY");
    assert!(ledger.outcomes.iter().all(|it| !matches!(it.detail, Detail::Assertion { passed: false, .. })));
    assert!(
        ledger.directives.iter().any(|it| matches!(it.data, Directive::Pad(_))),
        "the pad is back in the stream"
    );

    // the plugin is called without the pad: handed one, it fails
    let pad = ledger.directives.iter().find(|it| matches!(it.data, Directive::Pad(_))).unwrap().clone();
    for stage in ledger.plugins.build_stages(PluginStage::Booked) {
        let options = zhang_core::options::InMemoryOptions::default();
        let mut ctx = zhang_core::pipeline::StageContext::new(&options);
        assert!(stage.process(vec![pad.clone()], &mut ctx).is_err(), "{} fails on a pad", stage.name());
    }
}

/// the shell of the plugins [`edit_plugin`] writes: a processor reading the stream it is shown into its own
/// memory at 65536, whose `@PROCESSOR@` writes the stream it returns. See echo.wat for the kernel ABI
const EDIT_WAT: &str = r#"
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (memory 2)
  (global $o (mut i64) (i64.const 0))
  (global $first (mut i32) (i32.const 1))
  (data (i32.const 0) "@NAME@")
  (data (i32.const 256) "\"0.1.0\"")
  (data (i32.const 512) "[\"Processor\"]")
  (data (i32.const 1024) "@NEEDLE@")
  (data (i32.const 16384) "@REPLACEMENT@")

  (func $output_bytes (param $ptr i32) (param $len i32)
    (local $block i64) (local $i i32)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (block $done (loop $copy
      (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
      (call $store_u8 (i64.add (local.get $block) (i64.extend_i32_u (local.get $i))) (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $copy)))
    (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))
  (func (export "name") (result i32) (call $output_bytes (i32.const 0) (i32.const @NAME_LEN@)) (i32.const 0))
  (func (export "version") (result i32) (call $output_bytes (i32.const 256) (i32.const 7)) (i32.const 0))
  (func (export "supported_type") (result i32) (call $output_bytes (i32.const 512) (i32.const 13)) (i32.const 0))

  ;; read the input into this module's memory at 65536; returns its end
  (func $read_input (result i32)
    (local $len i32) (local $i i32)
    (local.set $len (i32.wrap_i64 (call $input_length)))
    (drop (memory.grow (i32.add (i32.shr_u (local.get $len) (i32.const 16)) (i32.const 1))))
    (block $done (loop $copy
      (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
      (i32.store8 (i32.add (i32.const 65536) (local.get $i)) (call $load_u8 (i64.add (call $input_offset) (i64.extend_i32_u (local.get $i)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $copy)))
    (i32.add (i32.const 65536) (local.get $len)))

  ;; whether the needle is at `at`, before `end`
  (func $matches (param $at i32) (param $end i32) (result i32)
    (local $j i32)
    (if (i32.gt_u (i32.add (local.get $at) (i32.const @NEEDLE_LEN@)) (local.get $end)) (then (return (i32.const 0))))
    (block $differs (loop $compare
      (if (i32.eq (local.get $j) (i32.const @NEEDLE_LEN@)) (then (return (i32.const 1))))
      (br_if $differs (i32.ne (i32.load8_u (i32.add (local.get $at) (local.get $j))) (i32.load8_u (i32.add (i32.const 1024) (local.get $j)))))
      (local.set $j (i32.add (local.get $j) (i32.const 1)))
      (br $compare)))
    (i32.const 0))

  ;; write `byte` to the output
  (func $write (param $byte i32)
    (call $store_u8 (global.get $o) (local.get $byte))
    (global.set $o (i64.add (global.get $o) (i64.const 1))))

  @PROCESSOR@)
"#;

/// copies the stream, replacing every needle with the replacement
const REPLACE_WAT: &str = r#"
  (func (export "processor") (result i32)
    (local $end i32) (local $i i32) (local $j i32) (local $count i32) (local $out i64) (local $len i64)
    (local.set $end (call $read_input))
    (local.set $i (i32.const 65536))
    (block $done (loop $scan
      (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
      (if (call $matches (local.get $i) (local.get $end))
        (then
          (local.set $count (i32.add (local.get $count) (i32.const 1)))
          (local.set $i (i32.add (local.get $i) (i32.const @NEEDLE_LEN@))))
        (else (local.set $i (i32.add (local.get $i) (i32.const 1)))))
      (br $scan)))
    (local.set $len (i64.sub
      (i64.extend_i32_u (i32.add (i32.sub (local.get $end) (i32.const 65536)) (i32.mul (local.get $count) (i32.const @REPLACEMENT_LEN@))))
      (i64.extend_i32_u (i32.mul (local.get $count) (i32.const @NEEDLE_LEN@)))))
    (local.set $out (call $alloc (local.get $len)))
    (global.set $o (local.get $out))
    (local.set $i (i32.const 65536))
    (block $done (loop $copy
      (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
      (if (call $matches (local.get $i) (local.get $end))
        (then
          (local.set $j (i32.const 0))
          (block $written (loop $replace
            (br_if $written (i32.ge_u (local.get $j) (i32.const @REPLACEMENT_LEN@)))
            (call $write (i32.load8_u (i32.add (i32.const 16384) (local.get $j))))
            (local.set $j (i32.add (local.get $j) (i32.const 1)))
            (br $replace)))
          (local.set $i (i32.add (local.get $i) (i32.const @NEEDLE_LEN@))))
        (else
          (call $write (i32.load8_u (local.get $i)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))))
      (br $copy)))
    (call $output_set (local.get $out) (local.get $len))
    (i32.const 0))
"#;

/// copies the stream, a JSON array, leaving out every directive holding the needle
const DROP_WAT: &str = r#"
  ;; write the directive in [start, stop) to the output, after a comma unless it is the first, unless it holds the needle
  (func $emit (param $start i32) (param $stop i32)
    (local $i i32)
    (if (i32.ge_u (local.get $start) (local.get $stop)) (then (return)))
    (local.set $i (local.get $start))
    (block $absent (loop $scan
      (br_if $absent (i32.ge_u (local.get $i) (local.get $stop)))
      (if (call $matches (local.get $i) (local.get $stop)) (then (return)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $scan)))
    (if (i32.eqz (global.get $first)) (then (call $write (i32.const 44))))
    (global.set $first (i32.const 0))
    (local.set $i (local.get $start))
    (block $done (loop $copy
      (br_if $done (i32.ge_u (local.get $i) (local.get $stop)))
      (call $write (i32.load8_u (local.get $i)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $copy))))

  (func (export "processor") (result i32)
    (local $end i32) (local $i i32) (local $c i32) (local $depth i32) (local $string i32) (local $start i32) (local $out i64)
    (local.set $end (call $read_input))
    (local.set $out (call $alloc (i64.extend_i32_u (i32.sub (local.get $end) (i32.const 65536)))))
    (global.set $o (local.get $out))
    (global.set $first (i32.const 1))
    (call $write (i32.const 91))
    ;; after the `[`: directives separated by commas outside strings and nested values, up to the `]`
    (local.set $i (i32.const 65537))
    (local.set $start (local.get $i))
    (block $done (loop $scan
      (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
      (local.set $c (i32.load8_u (local.get $i)))
      (if (local.get $string)
        (then
          (if (i32.eq (local.get $c) (i32.const 92))
            (then (local.set $i (i32.add (local.get $i) (i32.const 1))))
            (else (if (i32.eq (local.get $c) (i32.const 34)) (then (local.set $string (i32.const 0)))))))
        (else
          (if (i32.eq (local.get $c) (i32.const 34)) (then (local.set $string (i32.const 1))))
          (if (i32.or (i32.eq (local.get $c) (i32.const 123)) (i32.eq (local.get $c) (i32.const 91)))
            (then (local.set $depth (i32.add (local.get $depth) (i32.const 1)))))
          (if (i32.or (i32.eq (local.get $c) (i32.const 125)) (i32.eq (local.get $c) (i32.const 93)))
            (then
              (if (i32.eqz (local.get $depth))
                (then (call $emit (local.get $start) (local.get $i)) (br $done)))
              (local.set $depth (i32.sub (local.get $depth) (i32.const 1)))))
          (if (i32.and (i32.eq (local.get $c) (i32.const 44)) (i32.eqz (local.get $depth)))
            (then
              (call $emit (local.get $start) (local.get $i))
              (local.set $start (i32.add (local.get $i) (i32.const 1)))))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $scan)))
    (call $write (i32.const 93))
    (call $output_set (local.get $out) (i64.sub (global.get $o) (local.get $out)))
    (i32.const 0))
"#;

/// what a plugin [`edit_plugin`] writes does to the JSON of the stream it is shown
enum Edit<'a> {
    /// replace every needle with the replacement
    Replace(&'a str, &'a str),
    /// leave out every directive holding the needle
    Drop(&'a str),
}

/// write a processor plugin named `name` editing the stream it is shown into `dir`; returns its `plugin` directive
fn edit_plugin(dir: &TempDir, name: &str, edit: Edit) -> String {
    // every byte escaped, as WAT data strings take them
    let data = |text: &str| text.bytes().map(|byte| format!("\\{byte:02x}")).collect::<String>();
    let (processor, needle, replacement) = match edit {
        Edit::Replace(needle, replacement) => (REPLACE_WAT, needle, replacement),
        Edit::Drop(needle) => (DROP_WAT, needle, ""),
    };
    let quoted = format!("\"{name}\"");
    let module = EDIT_WAT
        .replace("@PROCESSOR@", processor)
        .replace("@NAME@", &data(&quoted))
        .replace("@NAME_LEN@", &quoted.len().to_string())
        .replace("@NEEDLE@", &data(needle))
        .replace("@NEEDLE_LEN@", &needle.len().to_string())
        .replace("@REPLACEMENT@", &data(replacement))
        .replace("@REPLACEMENT_LEN@", &replacement.len().to_string());
    let file = format!("{name}.wat");
    std::fs::write(dir.path().join(&file), module).unwrap();
    plugin(dir, &file)
}

/// the JSON of an account, as a plugin is shown it
fn account_json(name: &str) -> String {
    serde_json::to_string(&<zhang_ast::Account as std::str::FromStr>::from_str(name).unwrap()).unwrap()
}

/// a pad serving a balance in two currencies, and a later balance of the account it does not serve
const SERVED: &str = indoc! {r#"
    1970-01-01 commodity CNY
    1970-01-01 commodity USD
    1970-01-01 open Assets:Bank
    1970-01-01 open Equity:Open
    1970-01-01 open Equity:Other
    2024-01-01 pad Assets:Bank Equity:Open
    2024-01-02 balance Assets:Bank 100 CNY
    2024-01-02 balance Assets:Bank 20 USD
    2024-01-06 balance Assets:Bank 100 CNY
"#};

/// load [`SERVED`] with the plugin edit; returns the paddings (date, padded account, units, account padded from),
/// the pads (account, account padded from) and the errors
fn load_served(name: &str, edit: Edit) -> (Vec<String>, Vec<String>, Vec<String>) {
    let dir = ledger_dir(&[]);
    let directive = edit_plugin(&dir, name, edit);
    let ledger = load(&dir, &format!("option \"features.plugin\" \"true\"\n{directive}{SERVED}"));
    assert_eq!(registered(&ledger), vec![(name.to_owned(), vec![PluginType::Processor])]);
    let paddings = paddings(&ledger)
        .into_iter()
        .map(|(date, txn)| (date, written(txn)))
        .map(|(date, postings)| format!("{} {} {} from {}", date, postings[0].0, postings[0].1, postings[1].0))
        .collect();
    let pads = ledger
        .directives
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Pad(pad) => Some(format!("{} from {}", pad.account.name(), pad.pad.name())),
            _ => None,
        })
        .collect();
    let errors = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
    (paddings, pads, errors)
}

#[test]
fn a_plugin_giving_a_served_balance_another_pad_account_pads_from_it() {
    let (paddings, pads, errors) = load_served(
        "repad",
        Edit::Replace(
            &format!("\"pad\":{}", account_json("Equity:Open")),
            &format!("\"pad\":{}", account_json("Equity:Other")),
        ),
    );
    assert_eq!(
        paddings,
        vec![
            "2024-01-01 Assets:Bank 100 CNY from Equity:Other",
            "2024-01-01 Assets:Bank 20 USD from Equity:Other"
        ]
    );
    assert_eq!(pads, vec!["Assets:Bank from Equity:Other"]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_plugin_renaming_the_pad_account_renames_it_in_the_pad() {
    let (paddings, pads, errors) = load_served("rename", Edit::Replace(&account_json("Equity:Open"), &account_json("Equity:Renamed")));
    assert_eq!(
        paddings,
        vec![
            "2024-01-01 Assets:Bank 100 CNY from Equity:Renamed",
            "2024-01-01 Assets:Bank 20 USD from Equity:Renamed"
        ]
    );
    assert_eq!(pads, vec!["Assets:Bank from Equity:Renamed"]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_plugin_stripping_the_pad_of_the_served_balances_leaves_their_pad_out() {
    let (paddings, pads, errors) = load_served("strip", Edit::Replace("{\"BalancePad\":", "{\"BalanceCheck\":"));
    // three plain balances of an account holding nothing; the pad left out pads nothing
    assert!(paddings.is_empty(), "{paddings:?}");
    assert!(pads.is_empty(), "{pads:?}");
    assert_eq!(
        errors,
        vec!["UnusedPad", "AccountBalanceCheckError", "AccountBalanceCheckError", "AccountBalanceCheckError"]
    );
}

#[test]
fn a_plugin_dropping_the_served_balances_leaves_their_pad_out() {
    let (paddings, pads, errors) = load_served("drop", Edit::Drop("{\"BalancePad\":"));
    // the pad does not move on to the later balance, which fails; left out, it pads nothing
    assert!(paddings.is_empty(), "{paddings:?}");
    assert!(pads.is_empty(), "{pads:?}");
    assert_eq!(errors, vec!["UnusedPad", "AccountBalanceCheckError"]);
}

#[test]
fn a_plugin_editing_nothing_it_is_shown_keeps_the_pad() {
    let (paddings, pads, errors) = load_served("noop", Edit::Replace("{\"Nothing\":", "{\"Never\":"));
    assert_eq!(
        paddings,
        vec![
            "2024-01-01 Assets:Bank 100 CNY from Equity:Open",
            "2024-01-01 Assets:Bank 20 USD from Equity:Open"
        ]
    );
    assert_eq!(pads, vec!["Assets:Bank from Equity:Open"]);
    assert!(errors.is_empty(), "{errors:?}");
}

// --- writes: `zhang_write_preview` and `zhang_write_commit` ------------------------------------------------------

/// a ledger declaring `router_write.wat` with the `writes` config `declared` (its `capabilities` export) and the
/// `allowed_writes` grants `granted`
fn write_ledger(declared: Option<&str>, granted: &[&str]) -> (TempDir, Ledger) {
    let dir = ledger_dir(&["router_write.wat"]);
    let mut directive = plugin(&dir, "router_write.wat");
    if let Some(declared) = declared {
        directive.push_str(&format!("  writes: \"{}\"\n", declared.replace('"', "\\\"")));
    }
    for grant in granted {
        directive.push_str(&format!("  allowed_writes: \"{grant}\"\n"));
    }
    let ledger = load(&dir, &format!("option \"features.plugin\" \"true\"\n{directive}{LEDGER}"));
    assert_eq!(store_summary(&ledger).1.len(), 2, "the processor passed the stream through");
    (dir, ledger)
}

/// what `router-write` answers for the request path `/{case}` with `body`: the raw host function result
fn write_case(ledger: &Ledger, case: &str, body: &str) -> serde_json::Value {
    let request = PluginRequest::new("POST", format!("/{case}"), vec![], vec![], body.as_bytes().to_vec());
    let response = call(ledger, "router-write", &request).unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(response.body()).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(response.body())))
}

const OPEN_BATCH: &str = r#"{"directives": [{"kind": "open", "date": "2024-01-02", "account": "Expenses:Coffee"}]}"#;

#[test]
fn router_writes_through_the_host_what_it_declares_and_is_granted() {
    let (_dir, ledger) = write_ledger(Some(r#"["transaction", "open"]"#), &["open", "transaction"]);
    assert_eq!(
        ledger.plugins.router("router-write").unwrap().declared_writes(),
        &[WriteKind::Transaction, WriteKind::Open]
    );

    let batch = r#"{"directives": [{"kind": "open", "date": "2024-01-02", "account": "Expenses:Coffee"}, {"kind": "transaction", "payee": "a \"quoted\" payee\nnext line", "postings": [{"account": "Assets:Cash", "unit": {"number": "4.50", "commodity": "USD"}}]}]}"#;
    let preview = write_case(&ledger, "preview", batch);
    let directives = json!([
        {"kind": "open", "date": "2024-01-02", "account": "Expenses:Coffee"},
        {"kind": "transaction", "payee": "a \"quoted\" payee\nnext line", "postings": [{"account": "Assets:Cash", "unit": {"number": "4.50", "commodity": "USD"}}]},
    ]);
    assert_eq!(
        preview,
        json!({"Ok": {"ok": true, "file": "router-write.zhang", "generation": 1, "entries": [], "token": directives.to_string()}})
    );
    let commit = write_case(&ledger, "commit", &json!({"directives": directives, "token": "t0k"}).to_string());
    assert_eq!(commit, json!({"Ok": {"file": "router-write:t0k", "bytes": 0, "entries": 2, "generation": 2}}));
}

#[test]
fn router_writes_are_unavailable_without_a_declaration_and_denied_without_a_grant() {
    // declared nothing: the `capabilities` export answers no writes
    let (_dir, ledger) = write_ledger(None, &["open", "transaction"]);
    assert_eq!(ledger.plugins.router("router-write").unwrap().declared_writes(), &[]);
    assert_eq!(
        write_case(&ledger, "preview", OPEN_BATCH),
        json!({"Err": {"kind": "unavailable", "message": "plugin did not declare writes: its `capabilities` export lists no `writes`"}})
    );
    // declared, but the directive grants nothing: denied before the input is looked at
    let (_dir, ledger) = write_ledger(Some(r#"["open"]"#), &[]);
    assert_eq!(
        write_case(&ledger, "commit", "not even json"),
        json!({"Err": {"kind": "denied", "message": "plugin is not granted the writes it declares (open): add `allowed_writes: \"open\"` to its plugin directive"}})
    );
    // granted a kind the batch does not use
    let (_dir, ledger) = write_ledger(Some(r#"["transaction", "open"]"#), &["open"]);
    let transaction = r#"{"directives": [{"kind": "transaction", "payee": "x"}]}"#;
    assert_eq!(
        write_case(&ledger, "preview", transaction),
        json!({"Err": {"kind": "denied", "message": "plugin is not granted transaction writes: add `allowed_writes: \"transaction\"` to its plugin directive"}})
    );
    assert_eq!(write_case(&ledger, "preview", OPEN_BATCH)["Ok"]["ok"], json!(true));
    // granted a kind the plugin did not declare
    let (_dir, ledger) = write_ledger(Some(r#"["open"]"#), &["open", "transaction"]);
    assert_eq!(
        write_case(&ledger, "preview", transaction),
        json!({"Err": {"kind": "denied", "message": "plugin did not declare transaction writes: its `capabilities` export lists open"}})
    );
    // a declared kind this zhang does not know is ignored
    let (_dir, ledger) = write_ledger(Some(r#"["balance", "open"]"#), &["open"]);
    assert_eq!(ledger.plugins.router("router-write").unwrap().declared_writes(), &[WriteKind::Open]);
    assert_eq!(write_case(&ledger, "preview", OPEN_BATCH)["Ok"]["ok"], json!(true));
}

#[test]
fn router_write_input_that_cannot_be_read_is_invalid_input() {
    let (_dir, ledger) = write_ledger(Some(r#"["open"]"#), &["open"]);

    let not_json = write_case(&ledger, "x", "");
    assert_eq!(not_json["Err"]["kind"], json!("invalid_input"), "{not_json}");
    assert!(
        not_json["Err"]["message"]
            .as_str()
            .unwrap()
            .starts_with("the input is not a JSON object {\"directives\": ["),
        "{not_json}"
    );
    // a block over the cap is never read
    assert_eq!(
        write_case(&ledger, "g", ""),
        json!({"Err": {"kind": "invalid_input", "message": "the input is 1048577 bytes, over the 1048576 byte limit"}})
    );
    // a forged block header, which would crash a host trusting it, and no block at all
    let forged = write_case(&ledger, "f", "");
    assert_eq!(forged["Err"]["kind"], json!("invalid_input"), "{forged}");
    assert_eq!(
        write_case(&ledger, "n", ""),
        json!({"Err": {"kind": "invalid_input", "message": "the input is not a memory block"}})
    );
}

#[test]
fn router_write_refuses_a_batch_over_its_bounds_before_the_ledger_sees_it() {
    let (_dir, ledger) = write_ledger(Some(r#"["transaction", "open"]"#), &["open", "transaction"]);
    let invalid = |case: &str, body: &str| {
        let answer = write_case(&ledger, case, body);
        assert_eq!(answer["Err"]["kind"], json!("invalid"), "{answer}");
        answer["Err"]["message"].as_str().unwrap().to_owned()
    };

    let open = json!({"kind": "open", "date": "2024-01-02", "account": "Expenses:Coffee"});
    assert_eq!(
        invalid("preview", &json!({"directives": vec![open.clone(); 51]}).to_string()),
        "the batch holds 51 directives, over the limit of 50"
    );
    assert_eq!(invalid("preview", r#"{"directives": []}"#), "the batch holds no directive");
    assert_eq!(
        invalid("preview", r#"{"directives": [{"kind": "balance"}]}"#),
        "directives[0] has the kind \"balance\", which is not one of transaction, open"
    );
    assert_eq!(
        invalid(
            "preview",
            r#"{"directives": [{"kind": "transaction", "postings": [{"account": "Assets:Cash", "unit": {"number": 4.5, "commodity": "USD"}}]}]}"#
        ),
        "directives[0].postings[0].unit.number is the number 4.5: write amounts as strings, such as \"4.5\", so nothing is rounded"
    );
    assert_eq!(
        invalid("commit", OPEN_BATCH),
        "the batch has no token: preview it first, and commit it with the preview's token"
    );
    // exactly the limit reaches the host
    assert_eq!(
        write_case(&ledger, "preview", &json!({"directives": vec![open; 50]}).to_string())["Ok"]["ok"],
        json!(true)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Lua script plugins: the `.lua` fixtures through the same load path, on the vendored interpreter
// ---------------------------------------------------------------------------------------------------------------

mod lua {
    use zhang_core::plugin::lua::MAX_SCRIPT_LEN;

    use super::*;

    fn get(path: &str) -> PluginRequest {
        PluginRequest::new("GET", path, vec![], vec![], vec![])
    }

    fn json_body(response: &http::Response<Vec<u8>>) -> serde_json::Value {
        serde_json::from_slice(response.body()).unwrap_or_else(|e| panic!("the body is JSON: {e}: {}", String::from_utf8_lossy(response.body())))
    }

    /// a ledger declaring the script `name`, written with `source` and the meta lines `meta`, and the outcome of its load
    fn script_ledger(name: &str, source: &[u8], meta: &str) -> (TempDir, ZhangResult<Ledger>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), source).unwrap();
        let content = with_plugins(&format!("{}{meta}", plugin(&dir, name)));
        let ledger = try_load(&dir, &content);
        (dir, ledger)
    }

    /// the message of a load that must fail
    fn load_error(name: &str, source: &[u8]) -> String {
        let (_dir, ledger) = script_ledger(name, source, "");
        ledger.err().unwrap_or_else(|| panic!("{name} must not load")).to_string()
    }

    #[test]
    fn a_lua_router_answers_with_its_response_and_its_own_identity() {
        let (dir, ledger) = router_ledger(&["router_echo.lua"]);
        assert_eq!(registered(&ledger), vec![("router-echo".to_owned(), vec![PluginType::Router])]);
        assert_eq!(ledger.plugins.router("router-echo").unwrap().version, "0.1.0");
        let script = ledger.plugins.router("router-echo").unwrap().script().unwrap();
        assert_eq!(script.name, dir.path().join("router_echo.lua").display().to_string());
        assert!(script.source.starts_with("-- A router plugin"), "the source as written");
        let request = PluginRequest::new(
            "POST",
            "/sub/path",
            vec![("k".to_owned(), "v1".to_owned()), ("k".to_owned(), "v2".to_owned())],
            vec![("X-Custom".to_owned(), "yes".to_owned())],
            b"{\"amount\": \"10 \\\"CNY\\\"\"}".to_vec(),
        );

        let response = call(&ledger, "router-echo", &request).unwrap();

        assert_eq!(response.status(), 201);
        assert_eq!(response.headers()["x-echo"], "lua");
        assert_eq!(response.headers()["content-type"], "application/json; charset=utf-8");
        let echoed: PluginRequest = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(echoed, request);
    }

    #[test]
    fn a_script_is_an_input_of_the_load() {
        // the module is declared by the path the disk resolves the root to, as a relative path would be
        let dir = ledger_dir(&["router_echo.lua"]);
        let root = dir.path().canonicalize().unwrap();
        let content = with_plugins(&format!("plugin \"{}\"\n", root.join("router_echo.lua").display()));
        std::fs::write(root.join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load_with_data_source(root, "main.zhang".to_owned(), source).unwrap();

        // editing the script reloads the ledger under `zhang serve`, as editing a wasm module does
        assert!(
            ledger.extra_inputs.contains(&ExtraInput::File("router_echo.lua".into())),
            "{:?}",
            ledger.extra_inputs
        );
    }

    #[test]
    fn a_lua_router_reads_the_ledger_through_the_host_and_a_failed_query_raises() {
        let (_dir, ledger) = router_ledger(&["router_query.lua"]);

        let response = call(&ledger, "router-query", &get("/")).unwrap();

        assert_eq!(response.status(), 200, "the default status");
        assert_eq!(
            json_body(&response),
            json!([
                {"echo": "SELECT account, sum(position) AS balance GROUP BY account ORDER BY account"},
                {"title": "Home", "operating_currency": "CNY", "timezone": ledger.options.timezone.name()},
                {"ok": false, "message": "query error at line 1, column 2: no"},
            ])
        );
        // what it reported with `zhang.emit_error` during the request is only logged
        assert_eq!(errors(&ledger), vec![]);
    }

    #[test]
    fn a_lua_router_binds_query_parameters_through_the_host() {
        let (_dir, ledger) = router_ledger(&["router_query_params.lua"]);
        let case = |case: char| json_body(&call(&ledger, "router-query-params", &get(&format!("/{case}"))).unwrap());

        // the constructors make typed parameters; a decimal is a string
        assert_eq!(
            case('a'),
            json!({
                "echo": "SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min",
                "params": {"from": "2024-01-01", "who": "'lunch'", "min": "5.5"},
            })
        );
        // nil is NULL, and the query still compiles against the declared type
        assert_eq!(case('b'), json!({"echo": "SELECT date WHERE narration = :who", "params": {"who": "NULL"}}));
        // the host's failures raise with the host's text; a positioned one names the position
        assert_eq!(
            case('c'),
            json!({"error": "query error: parameter :who has the type \"money\", which is not one of bool, int, decimal, str, date, set"})
        );
        assert_eq!(
            case('d'),
            json!({"error": "query error: parameter :from must be a date string YYYY-MM-DD or null"})
        );
        assert_eq!(case('e'), json!({"error": "query error at line 1, column 27: parameter :from is not bound"}));
        assert_eq!(
            case('f'),
            json!({"error": "parameter :from must be made with zhang.str/int/decimal/bool/date/set or be a {type = ..., value = ...} table"})
        );
        // a decimal is a string or an integer; a float is refused before it is sent, as in a write batch
        let refused = case('g');
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .ends_with("router_query_params.lua:28: decimal must be a string or an integer, got 12.5"),
            "{refused}"
        );
        // (the fake host writes a bound decimal as BQL, where an integer-valued one carries a trailing dot)
        assert_eq!(case('h'), json!({"echo": "SELECT date WHERE number > :min", "params": {"min": "12."}}));
        // no parameters cross as an empty object, never as an empty array
        assert_eq!(case('n'), json!({"echo": "SELECT date", "params": {}}));
        assert_eq!(case('s'), json!({"tables": [{"name": "postings"}], "functions": [{"name": "sum"}]}));
    }

    #[test]
    fn a_lua_router_reads_the_schema_the_time_and_the_ledger_info() {
        let (_dir, ledger) = router_ledger(&["router_schema.lua"]);
        assert_eq!(
            json_body(&call(&ledger, "router-schema", &get("/")).unwrap()),
            json!({"tables": [{"name": "postings"}], "functions": [{"name": "sum"}]})
        );

        let dir = ledger_dir(&["router_now.lua"]);
        let content = format!("option \"timezone\" \"Asia/Shanghai\"\n{}", with_plugins(&plugin(&dir, "router_now.lua")));
        let ledger = load_with_clock(&dir, &content, Clock::Fixed(fixed_now()));
        assert_eq!(
            json_body(&call(&ledger, "router-now", &get("/")).unwrap()),
            json!({"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"})
        );
        // a request is not a load: the ledger does not start depending on the date
        assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);

        let dir = ledger_dir(&["router_info.lua"]);
        let content = format!(
            "option \"features.plugin\" \"true\"\noption \"title\" \"Home\"\n{}  greeting: \"hi\"\n{LEDGER}",
            plugin(&dir, "router_info.lua")
        );
        let ledger = load(&dir, &content);
        // a config key that is not set is nil, so it is absent from the answer
        assert_eq!(
            json_body(&call(&ledger, "router-info", &get("/")).unwrap()),
            json!({"title": "Home", "operating_currency": "CNY", "timezone": ledger.options.timezone.name(), "greeting": "hi", "abi": "1"})
        );
    }

    #[test]
    fn a_looping_lua_router_stops_at_its_timeout() {
        let dir = ledger_dir(&["router_loop.lua"]);
        let ledger = load(&dir, &with_plugins(&format!("{}  timeout: \"500ms\"\n", plugin(&dir, "router_loop.lua"))));
        let started = Instant::now();

        let result = call(&ledger, "router-loop", &get("/"));

        assert_eq!(result.unwrap_err(), RouterError::Timeout);
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(10), "the call took {elapsed:?}, not about half a second");
    }

    #[test]
    fn lua_router_failures_are_errors_not_panics() {
        let (_dir, ledger) = router_ledger(&["router_trap.lua", "router_malformed.lua"]);

        // a Lua error names the script's line and carries a traceback, for the server log
        let failed = call(&ledger, "router-trap", &get("/")).unwrap_err();
        assert!(
            matches!(&failed, RouterError::Failed(message) if message.contains("router_trap.lua:6: boom") && message.contains("stack traceback:")),
            "{failed}"
        );
        assert_eq!(
            call(&ledger, "router-malformed", &get("/")).unwrap_err(),
            RouterError::BadResponse("the response is not a JSON object".to_owned())
        );
        // the plugin still serves after a failure
        assert!(call(&ledger, "router-trap", &get("/")).is_err());
    }

    #[test]
    fn a_memory_hog_is_stopped_at_the_budget_with_a_clear_error() {
        let dir = ledger_dir(&["memory_hog.lua"]);
        let ledger = load(&dir, &with_plugins(&format!("{}  timeout: \"60s\"\n", plugin(&dir, "memory_hog.lua"))));

        let failed = call(&ledger, "memory-hog", &get("/")).unwrap_err();

        assert!(
            matches!(&failed, RouterError::Failed(message) if message.contains("memory_hog.lua` exceeded 64 MiB")),
            "{failed}"
        );
    }

    #[test]
    fn the_budget_is_reported_when_it_is_reached_with_a_deep_stack() {
        // unwinding from the memory error shrinks the Lua stack; a shrink must not clear the budget flag
        let dir = ledger_dir(&["memory_hog_deep.lua"]);
        let ledger = load(&dir, &with_plugins(&format!("{}  timeout: \"60s\"\n", plugin(&dir, "memory_hog_deep.lua"))));

        let failed = call(&ledger, "memory-hog-deep", &get("/")).unwrap_err();

        assert!(
            matches!(&failed, RouterError::Failed(message) if message.contains("memory_hog_deep.lua` exceeded 64 MiB")),
            "{failed}"
        );
    }

    #[test]
    fn a_script_that_does_not_parse_fails_the_load_with_its_line() {
        let dir = ledger_dir(&["syntax_error.lua"]);
        let error = try_load(&dir, &with_plugins(&plugin(&dir, "syntax_error.lua")))
            .err()
            .expect("must not load")
            .to_string();
        assert!(error.contains("plugin script "), "{error}");
        assert!(error.contains("syntax_error.lua:4: unexpected symbol near '='"), "{error}");
    }

    #[test]
    fn a_script_that_is_not_utf8_or_too_big_or_of_the_wrong_shape_fails_the_load() {
        let error = load_error("not_utf8.lua", b"-- latin1: \xE9\nreturn {}");
        assert!(error.contains("not_utf8.lua is not UTF-8 text: the byte at offset 11 is not UTF-8"), "{error}");

        let mut too_big = b"-- ".to_vec();
        too_big.resize(MAX_SCRIPT_LEN + 1, b'x');
        let error = load_error("too_big.lua", &too_big);
        assert!(error.contains("too_big.lua is 1048577 bytes, over the 1048576 byte limit"), "{error}");

        let error = load_error("no_table.lua", b"return 1");
        assert!(error.contains("the script must return a module table"), "{error}");
        let error = load_error("no_name.lua", b"return { version = \"0.1.0\" }");
        assert!(error.contains("the module table needs a non-empty string `name`"), "{error}");
        let error = load_error("no_version.lua", b"return { name = \"x\" }");
        assert!(error.contains("the module table needs a string `version`"), "{error}");
        let error = load_error(
            "runtime_error.lua",
            b"local M = { name = \"x\", version = \"0.1.0\" }\nerror(\"at load\")\nreturn M",
        );
        assert!(error.contains("runtime_error.lua:2: at load"), "{error}");

        // a script without a `router` function registers, and fails when a request reaches it
        let (_dir, ledger) = script_ledger("no_router.lua", b"return { name = \"no-router\", version = \"0.1.0\" }", "");
        let ledger = ledger.unwrap();
        let failed = call(&ledger, "no-router", &get("/")).unwrap_err();
        assert!(
            matches!(&failed, RouterError::Failed(message) if message.contains("the module table has no `router` function")),
            "{failed}"
        );
    }

    #[test]
    fn a_script_declaring_a_processor_or_mapper_is_refused() {
        let dir = ledger_dir(&["declares_processor.lua"]);
        let error = try_load(&dir, &with_plugins(&plugin(&dir, "declares_processor.lua")))
            .err()
            .expect("must not load")
            .to_string();
        assert!(
            error.contains(
                "Lua processors and mappers are not supported yet (#698 P1): a directive's amounts cross as JSON numbers and Lua would read them as floats"
            ),
            "{error}"
        );
        let error = load_error("mapper.lua", b"return { name = \"m\", version = \"0.1.0\", types = { \"Mapper\" } }");
        assert!(error.contains("Lua processors and mappers are not supported yet"), "{error}");
        // any other value is named, and `types` must be a list
        let error = load_error("lower.lua", b"return { name = \"l\", version = \"0.1.0\", types = { \"router\" } }");
        assert!(
            error.contains("unknown plugin type \"router\"; Lua scripts declare types = {\"Router\"}"),
            "{error}"
        );
        let error = load_error("set.lua", b"return { name = \"s\", version = \"0.1.0\", types = { Router = true } }");
        assert!(
            error.contains("`types` must be a list such as {\"Router\"}, got a table with keys that are not 1, 2, ..."),
            "{error}"
        );
        // `types` defaults to Router, and an empty list registers a plugin that serves nothing
        let (_dir, ledger) = script_ledger("none.lua", b"return { name = \"none\", version = \"0.1.0\", types = {} }", "");
        assert_eq!(registered(&ledger.unwrap()), vec![("none".to_owned(), vec![])]);
    }

    #[test]
    fn a_module_with_another_extension_is_refused_by_name() {
        let (_dir, ledger) = script_ledger("report.txt", b"return {}", "");
        let error = ledger.err().expect("must not load").to_string();
        assert!(error.contains("report.txt is not a `.wasm`/`.wat`/`.lua` module"), "{error}");
    }

    #[test]
    fn the_sandbox_hides_what_a_script_must_not_reach() {
        let (_dir, ledger) = router_ledger(&["sandbox.lua"]);

        let body = json_body(&call(&ledger, "sandbox", &get("/")).unwrap());

        assert_eq!(body["leaked"], json!([]), "{body}");
        assert_eq!(body["debug_names"], json!(["traceback"]), "{body}");
        let flags = body
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, value)| value.is_boolean())
            .map(|(key, value)| (key.as_str(), value.as_bool().unwrap()));
        let off: Vec<&str> = flags.filter(|(_, on)| !on).map(|(key, _)| key).collect();
        assert_eq!(off, Vec::<&str>::new(), "{body}");
        let libraries = body["libraries"].as_object().unwrap();
        let missing: Vec<&String> = libraries.iter().filter(|(_, on)| !on.as_bool().unwrap()).map(|(key, _)| key).collect();
        assert_eq!(missing, Vec::<&String>::new(), "{body}");
    }

    /// a host answering the query of `json_roundtrip.lua` with the values the JSON rules are about
    struct JsonHost;

    impl RouterHost for JsonHost {
        fn query(&self, _bql: &str) -> Result<serde_json::Value, QueryFailure> {
            Ok(json!({"dec": "12.50", "big": 9223372036854775807i64, "empty": {}, "list": []}))
        }

        fn query_with_params(&self, _bql: &str, _params: &BTreeMap<String, RawQueryParam>) -> Result<serde_json::Value, QueryFailure> {
            unreachable!()
        }

        fn schema(&self) -> Result<serde_json::Value, QueryFailure> {
            unreachable!()
        }
    }

    #[test]
    fn json_round_trips_through_the_bridge_by_the_documented_rules() {
        let (_dir, ledger) = router_ledger(&["json_roundtrip.lua"]);
        let posted = json!({"null": null, "big": 4611686018427387904i64, "nul": "a\u{0}b", "empty": {}, "list": [], "dec": "0.10", "nested": {"inner": []}});
        let request = PluginRequest::new("POST", "/", vec![], vec![], serde_json::to_vec(&posted).unwrap());
        let plugin = ledger.plugins.router("json-roundtrip").unwrap();

        let response = plugin.execute_as_router(&request, &ledger, Arc::new(JsonHost)).unwrap();

        let body = json_body(&response);
        assert_eq!(
            body,
            json!({
                "null": null,
                "posted_null_is_null": true,
                // decoded objects and arrays keep their kind, empty ones included
                "posted": posted,
                "empty_object": {},
                "empty_table": [],
                "empty_array": [],
                "filled_object": {"k": "v"},
                "big": 4611686018427387904i64,
                "big_back": 4611686018427387904i64,
                "big_is_exact": true,
                "max": 9223372036854775807i64,
                "min": -9223372036854775808i64,
                "float": 1.5,
                "tenth": 0.1,
                "third": 0.33333333333333,
                "nul": "a\u{0}b",
                "nul_back": "a\u{0}b",
                "nul_back_len": 3,
                "decimal": "12.50",
                "decimal_is_string": true,
                "unicode": "héllo 世界 \u{1F600}",
                "escaped": "quote\" backslash\\ newline\n tab\t",
                "yes": true,
                "no": false,
                "bad_json": "invalid JSON: no valid JSON value at line 1, column 7",
                "trailing": "invalid JSON: unexpected data at byte 4",
            })
        );
    }

    #[test]
    fn identical_requests_give_identical_bytes() {
        // the string hash seed is fixed at build time, so the key order of an encoded table never changes
        let (_dir, ledger) = router_ledger(&["json_order.lua"]);

        let first = call(&ledger, "json-order", &get("/a")).unwrap();
        let second = call(&ledger, "json-order", &get("/a")).unwrap();

        assert_eq!(first.body(), second.body(), "{}", String::from_utf8_lossy(first.body()));
        let body = json_body(&first);
        assert_eq!(body.as_object().unwrap().len(), 13);
        assert_eq!(body["nested"]["t"], json!(7));
    }

    #[test]
    fn the_first_declared_script_serves_a_shared_name() {
        let (_dir, ledger) = router_ledger(&["router_echo.lua", "router_echo_twin.lua"]);

        assert_eq!(
            registered(&ledger),
            vec![
                ("router-echo".to_owned(), vec![PluginType::Router]),
                ("router-echo".to_owned(), vec![PluginType::Router])
            ]
        );
        assert_eq!(ledger.plugins.routers.len(), 2);
        assert_eq!(ledger.plugins.router("router-echo").unwrap().version, "0.1.0");
        let response = call(&ledger, "router-echo", &get("/")).unwrap();
        assert_eq!(response.status(), 201, "the first declared answers");
    }

    /// a ledger declaring `router_write.lua` with the `writes` config `declared` (a JSON list, its `capabilities`
    /// export) and the `allowed_writes` grants `granted`
    fn lua_write_ledger(declared: Option<&str>, granted: &[&str]) -> (TempDir, Ledger) {
        let dir = ledger_dir(&["router_write.lua"]);
        let mut directive = plugin(&dir, "router_write.lua");
        if let Some(declared) = declared {
            directive.push_str(&format!("  writes: \"{}\"\n", declared.replace('"', "\\\"")));
        }
        for grant in granted {
            directive.push_str(&format!("  allowed_writes: \"{grant}\"\n"));
        }
        let ledger = load(&dir, &with_plugins(&directive));
        (dir, ledger)
    }

    /// what `router-write` (the script) answers for the request path `/{case}` with `body`: the host's answer as
    /// the script decoded it, or `{"error": ...}`
    fn lua_write_case(ledger: &Ledger, case: &str, body: &str) -> serde_json::Value {
        let request = PluginRequest::new("POST", format!("/{case}"), vec![], vec![], body.as_bytes().to_vec());
        let response = call(ledger, "router-write", &request).unwrap();
        assert_eq!(response.status(), 200);
        json_body(&response)
    }

    #[test]
    fn a_lua_router_previews_and_commits_through_the_host() {
        let (_dir, ledger) = lua_write_ledger(Some(r#"["transaction", "open"]"#), &["open", "transaction"]);
        // the module table's `writes` is the `capabilities` export
        assert_eq!(
            ledger.plugins.router("router-write").unwrap().declared_writes(),
            &[WriteKind::Transaction, WriteKind::Open]
        );
        let batch = json!([
            {"kind": "open", "date": "2024-01-02", "account": "Expenses:Coffee", "commodities": ["USD"]},
            {"kind": "transaction", "datetime": "2024-01-02", "payee": "a \"quoted\" payee", "postings": [{"account": "Assets:Cash", "unit": {"number": "4.50", "commodity": "USD"}}], "tags": [], "links": [], "metas": []},
        ]);

        // the fake host echoes the batch as the token: what the script sent is what the host read
        let preview = lua_write_case(&ledger, "preview", &json!({"directives": batch}).to_string());
        assert_eq!(
            preview,
            json!({"ok": true, "file": "router-write.zhang", "generation": 1, "entries": [], "token": batch.to_string()})
        );
        let commit = lua_write_case(&ledger, "commit", &json!({"directives": batch, "token": "t0k"}).to_string());
        assert_eq!(commit, json!({"file": "router-write:t0k", "bytes": 0, "entries": 2, "generation": 2}));

        // the lint refuses a float before the host sees it, naming where it is; the host refuses an integer
        let float = lua_write_case(&ledger, "f", "");
        let message = float["error"].as_str().unwrap_or_else(|| panic!("{float}"));
        assert!(
            message.contains("amount `12.3` must be a string (directives[1].postings[1].unit.number)"),
            "{message}"
        );
        let integer = lua_write_case(&ledger, "i", "");
        let message = integer["error"].as_str().unwrap_or_else(|| panic!("{integer}"));
        assert!(message.contains("invalid: directives[0].metas[0].value is the number 3"), "{message}");
        let empty = lua_write_case(&ledger, "e", "");
        assert!(empty["error"].as_str().unwrap().contains("invalid: the batch holds no directive"), "{empty}");
        let no_token = lua_write_case(&ledger, "x", "");
        assert!(
            no_token["error"]
                .as_str()
                .unwrap()
                .contains("zhang.write.commit expects the token as a string, got number"),
            "{no_token}"
        );

        // without `writes` in the module table the script declares nothing; without the grant it is denied
        let (_dir, ledger) = lua_write_ledger(None, &["open"]);
        assert_eq!(ledger.plugins.router("router-write").unwrap().declared_writes(), &[]);
        let undeclared = lua_write_case(&ledger, "preview", &json!({"directives": [{"kind": "open"}]}).to_string());
        assert!(
            undeclared["error"].as_str().unwrap().contains("unavailable: plugin did not declare writes"),
            "{undeclared}"
        );
        let (_dir, ledger) = lua_write_ledger(Some(r#"["open"]"#), &[]);
        let denied = lua_write_case(&ledger, "preview", &json!({"directives": [{"kind": "open"}]}).to_string());
        assert!(
            denied["error"]
                .as_str()
                .unwrap()
                .contains("denied: plugin is not granted the writes it declares (open)"),
            "{denied}"
        );
    }

    /// a `tracing` writer collecting what a subscriber writes, so a test reads the host's log
    #[derive(Clone, Default)]
    struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;

        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    #[test]
    fn print_goes_to_the_hosts_log_prefixed_with_the_script() {
        let (dir, ledger) = router_ledger(&["router_print.lua"]);
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();

        let response = tracing::subscriber::with_default(subscriber, || call(&ledger, "router-print", &get("/")).unwrap());

        assert_eq!(response.status(), 204);
        let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let script = dir.path().join("router_print.lua").display().to_string();
        assert!(log.contains(&format!("{script}: hello\t42\tnil\ttrue")), "{log}");
        assert!(
            log.contains(&format!("{script}: \n")) || log.contains(&format!("{script}:  ")),
            "an empty print still logs: {log}"
        );
    }

    #[test]
    fn a_lua_router_gets_no_file_access_even_with_allowed_paths() {
        let dir = documents_dir();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugins/router_files.lua"),
            dir.path().join("router_files.lua"),
        )
        .unwrap();
        let directive = format!(
            "{}  allowed_paths: \"documents\"\n  read: \"documents/receipt.txt\"\n",
            plugin(&dir, "router_files.lua")
        );
        let ledger = load(&dir, &with_plugins(&directive));

        let body = json_body(&call(&ledger, "router-files", &get("/")).unwrap());

        for call in ["file", "dir"] {
            let error = body[call]["error"].as_str().unwrap_or_else(|| panic!("{call} must raise: {body}"));
            assert!(error.starts_with("denied: "), "{error}");
            assert!(
                error.contains("files can only be read while the plugin runs as a processor or mapper"),
                "{error}"
            );
        }
    }
}
