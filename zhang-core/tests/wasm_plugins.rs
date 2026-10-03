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

use indoc::{formatdoc, indoc};
use itertools::Itertools;
use serde_json::json;
use tempfile::TempDir;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
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
