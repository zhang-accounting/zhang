//! WASM plugins through the real load path, using the hand-written WAT fixtures in `tests/plugins/`.
//!
//! extism compiles WAT text itself, so the fixtures need no wasm toolchain. A test copies the fixtures
//! it uses into its own ledger directory and declares them by absolute path: a local ledger resolves
//! a plugin module against the working directory, and a path per test gives every test its own
//! module cache file.
#![cfg(feature = "plugin_runtime")]

use std::path::Path;
use std::sync::Arc;

use indoc::indoc;
use itertools::Itertools;
use tempfile::TempDir;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::PluginType;

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

fn load(dir: &TempDir, content: &str) -> Ledger {
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source)).unwrap_or_else(|e| panic!("ledger should load: {e}"))
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
fn plugin_declaring_only_the_retired_router_type_loads_and_runs_as_nothing() {
    let dir = ledger_dir(&["router.wat"]);
    let ledger = load(&dir, &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "router.wat")));

    assert_eq!(registered(&ledger), vec![("router".to_owned(), vec![])]);
    assert!(ledger.plugins.build_stages().is_empty());
    let plain = load(&ledger_dir(&[]), LEDGER);
    assert_eq!(store_summary(&ledger), store_summary(&plain));
}

#[test]
fn plugin_declaring_router_and_processor_runs_as_a_processor() {
    let dir = ledger_dir(&["router_processor.wat"]);
    let ledger = load(
        &dir,
        &format!("option \"features.plugin\" \"true\"\n{}{LEDGER}", plugin(&dir, "router_processor.wat")),
    );

    // Router is ignored, the processor still runs: it drops the whole stream
    assert_eq!(registered(&ledger), vec![("router-processor".to_owned(), vec![PluginType::Processor])]);
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
