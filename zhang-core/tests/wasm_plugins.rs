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
use zhang_ast::Directive;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
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
