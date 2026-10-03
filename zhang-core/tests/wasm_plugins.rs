//! WASM plugins through the real load path, using the hand-written WAT fixtures in `tests/plugins/`.
//!
//! extism compiles WAT text itself, so the fixtures need no wasm toolchain. A test copies the fixtures
//! it uses into its own ledger directory and declares them by absolute path: a local ledger resolves
//! a plugin module against the working directory, and a path per test gives every test its own
//! module cache file.
#![cfg(feature = "plugin_runtime")]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use serde_json::json;
use tempfile::TempDir;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo};
use zhang_core::clock::Clock;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::plugin::capabilities::plugin_seed;
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::router::{QueryFailure, RouterError, RouterHost};
use zhang_core::plugin::PluginType;
use zhang_core::ZhangResult;

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
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source))
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

/// what the store holds: accounts, the postings of every transaction, and the reported errors
fn store_summary(ledger: &Ledger) -> (Vec<String>, Vec<String>, Vec<String>) {
    let store = ledger.store.read().unwrap();
    let accounts = store.accounts.keys().cloned().sorted().collect();
    let postings = store
        .postings
        .iter()
        .map(|it| format!("{} {}", it.account.name(), it.inferred_amount))
        .sorted()
        .collect();
    let errors = store.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
    (accounts, postings, errors)
}

/// the errors in the store with their spans and metas
fn errors(ledger: &Ledger) -> Vec<(ErrorKind, SpanInfo, HashMap<String, String>)> {
    let store = ledger.store.read().unwrap();
    store
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
    let stages = ledger.plugins.build_stages();
    assert_eq!(stages.iter().map(|it| it.name()).collect_vec(), vec!["echo"]);

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

    let store = ledger.store.read().unwrap();
    let errors = store
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
    drop(store);
    assert_eq!(registered(&ledger), vec![("echo".to_owned(), vec![PluginType::Processor])]);
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

/// a router host answering every query with `{"echo": <bql>}`, or with a failure for `FAIL`
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
