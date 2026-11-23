//! What a plugin keeps across loads: the compiled code of every module, and what each default plugin declared.
//!
//! A load registers every plugin anew, and running a plugin needs its module compiled ([`PluginRuntime`]): hundreds of
//! milliseconds for a module the size of the Lua interpreter or of an analyzer written with the SDK. Every write
//! reloads the ledger. So the compiled code of a module is kept here for the process, by the module's bytes
//! ([`code`]), and a load running the same module, a ledger's own plugin as well as a default one, reuses it: the Lua
//! interpreter is the same module for every Lua script, whatever the script, and is compiled once. Compiled code holds
//! no settings of a plugin, which each instance gets from the manifest of its call ([`PluginRuntime::instantiate`]),
//! so a changed script, config or option of the ledger takes effect at the next load without compiling; a changed
//! module is other bytes, compiled at the next load. At most [`MODULES_KEPT`] modules are kept, the least recently used
//! one going first, each with at most a few compiled plugins idle ([`super::runtime`]).
//!
//! A default plugin ([`crate::analysis::LoadOptions::default_plugins`]) is the same module at every load of every
//! ledger, so its runtime is kept here too, by its module and its settings ([`Manifest`]: its config, the ledger's
//! options included, its allowed hosts, its timeout and its memory limit): with it what the plugin declared when it
//! registered, which a load with the same module and settings reads back instead of running the plugin
//! ([`runtime`]). At most [`KEPT`] runtimes are kept, the least recently used one going first.
//!
//! What is kept holds no ledger: the host functions reach each instance's host through the instance
//! ([`PluginRuntime`], "InstanceBindings"), and each instance leases compiled code of its own.

use std::sync::{Arc, Mutex, PoisonError};

use extism::Manifest;

use super::runtime::{ModuleCode, PluginRuntime};

/// the most modules whose compiled code is kept: the Lua interpreter every Lua plugin runs on, and a ledger's wasm
/// modules, a few
const MODULES_KEPT: usize = 8;

/// the most runtimes of default plugins kept: a server serves one ledger, whose default plugins are a few
const KEPT: usize = 4;

/// the compiled code kept, the most recently used last
static CODE: Mutex<Vec<Arc<ModuleCode>>> = Mutex::new(Vec::new());

/// The compiled code of the module `module_bytes`, for every plugin running it: the code kept for these bytes, or new
/// code, compiled on first use and kept from now on. Every module compiles with the same settings of wasmtime and the
/// same host functions, and compiled code holds no settings of a plugin, so the module's bytes, compared whole, are the
/// key.
pub(crate) fn code(module_bytes: Vec<u8>) -> Arc<ModuleCode> {
    let mut kept = CODE.lock().unwrap_or_else(PoisonError::into_inner);
    let found = kept.iter().position(|it| it.module_bytes() == module_bytes.as_slice());
    keep(&mut kept, MODULES_KEPT, found, || Arc::new(ModuleCode::new(module_bytes))).clone()
}

/// a runtime kept, and the settings it is kept for
struct Kept {
    manifest: Manifest,
    runtime: Arc<PluginRuntime>,
}

/// the runtimes kept, the most recently used last
static RUNTIMES: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// The runtime of the default plugin whose module is `module_bytes`, for the settings of `manifest`: the one kept for
/// them, or a new one of the module's compiled code ([`code`]), kept from now on. A runtime kept is found by the
/// module's bytes and the settings, compared whole, as what the plugin declared when it registered depends on both.
pub(crate) fn runtime(module_bytes: Vec<u8>, manifest: &Manifest) -> Arc<PluginRuntime> {
    let mut kept = RUNTIMES.lock().unwrap_or_else(PoisonError::into_inner);
    let found = kept
        .iter()
        .position(|it| it.manifest == *manifest && it.runtime.module_bytes() == module_bytes.as_slice());
    let make = || Kept {
        manifest: manifest.clone(),
        runtime: Arc::new(PluginRuntime::of(code(module_bytes))),
    };
    keep(&mut kept, KEPT, found, make).runtime.clone()
}

/// The item at `found` in `kept`, if any, else the one `make` makes, kept from now on in place of the least recently
/// used one when `limit` are kept already; either way the most recently used. `kept` is in the order of use, the most
/// recent last.
fn keep<T>(kept: &mut Vec<T>, limit: usize, found: Option<usize>, make: impl FnOnce() -> T) -> &T {
    let item = match found {
        Some(at) => kept.remove(at),
        None => {
            if kept.len() >= limit {
                kept.remove(0);
            }
            make()
        }
    };
    kept.push(item);
    kept.last().expect("an item was just kept")
}

#[cfg(test)]
mod test {
    use std::path::Path;
    use std::sync::Arc;

    use zhang_ast::error::ErrorKind;

    use crate::analysis::{DefaultPlugin, LoadOptions};
    use crate::clock::Clock;
    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::ledger::Ledger;
    use crate::plugin::store::RegisteredPlugin;
    use crate::plugin_policy::PluginPolicy;

    /// a router: registered at every load, and never run by one
    const MODULE: &[u8] = include_bytes!("../../tests/plugins/router_now.wat");

    /// [`MODULE`] as bytes of their own, with the comment `tag`: compiled code no other test shares
    fn module(tag: &str) -> Vec<u8> {
        [MODULE, format!("\n;; {tag}\n").as_bytes()].concat()
    }

    fn options(module: &str, content: &[u8]) -> LoadOptions {
        LoadOptions {
            plugin_policy: Arc::new(PluginPolicy::default()),
            analysis: None,
            default_plugins: vec![DefaultPlugin {
                module: module.to_owned(),
                file: "router_now.wat".to_owned(),
                content: Arc::from(content),
            }],
        }
    }

    fn load(dir: &Path, options: LoadOptions) -> Ledger {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        Ledger::load_with_options(dir.to_path_buf(), "main.zhang".to_owned(), source, Clock::System, options).expect("the ledger loads")
    }

    /// The first plugin of `ledger`.
    fn first(ledger: &Ledger) -> &RegisteredPlugin {
        &ledger.plugins.ordered[0].0
    }

    /// The runtime of the default plugin of `ledger`.
    fn runtime(ledger: &Ledger) -> Arc<super::PluginRuntime> {
        assert!(first(ledger).is_default());
        first(ledger).runtime.clone()
    }

    #[test]
    fn a_reload_compiles_an_unchanged_default_module_no_more() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let first = load(dir.path(), options("zhang:cache-test-reload", &module("default")));
        assert_eq!(runtime(&first).compilations(), 1, "registering compiles the module");
        assert!(runtime(&first).registration().is_some(), "and keeps what the plugin declared");
        let second = first.reloader()().expect("the ledger reloads");
        assert!(Arc::ptr_eq(&runtime(&first), &runtime(&second)), "a reload keeps the runtime");
        let (registered, types) = &second.plugins.ordered[0];
        assert_eq!(
            (registered.name.as_str(), types.as_slice()),
            ("router-now", [crate::plugin::PluginType::Router].as_slice())
        );
        assert_eq!(runtime(&second).compilations(), 1, "and compiles nothing again");
        // the settings changed with an option of the ledger: a runtime of their own, which registers the plugin again
        // with the module's compiled code, and compiles nothing
        std::fs::write(dir.path().join("main.zhang"), "option \"title\" \"Home\"\n1970-01-01 open Assets:Cash\n").unwrap();
        let third = second.reloader()().expect("the ledger reloads");
        assert!(!Arc::ptr_eq(&runtime(&second), &runtime(&third)));
        assert!(Arc::ptr_eq(runtime(&second).code(), runtime(&third).code()), "the module's code is kept");
        assert_eq!(runtime(&third).compilations(), 0);
        let fourth = third.reloader()().expect("the ledger reloads");
        assert!(Arc::ptr_eq(&runtime(&third), &runtime(&fourth)));
        assert_eq!(runtime(&fourth).compilations(), 0);
        assert!(
            fourth.errors.iter().all(|error| error.error_type != ErrorKind::PluginError),
            "{:?}",
            fourth.errors
        );
    }

    /// A ledger's own plugin registers at every load, from the compiled code of its module, which a reload, and so a
    /// write, reuses as long as the module is the same
    #[test]
    fn a_ledgers_own_plugin_reuses_the_compiled_code_of_its_module_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("router_now.wat"), module("own")).unwrap();
        std::fs::write(
            dir.path().join("main.zhang"),
            "option \"features.plugin\" \"true\"\nplugin \"router_now.wat\"\n1970-01-01 open Assets:Cash\n",
        )
        .unwrap();
        let loaded = load(dir.path(), LoadOptions::with_policy(Arc::new(PluginPolicy::default())));
        assert!(!first(&loaded).is_default());
        assert_eq!(first(&loaded).compilations(), 1, "the first load compiles the module");
        let mut loads = vec![loaded];
        for _ in 0..2 {
            let reloaded = loads.last().unwrap().reloader()().expect("the ledger reloads");
            loads.push(reloaded);
        }
        let [one, two, three] = [0, 1, 2].map(|at| first(&loads[at]));
        assert!(!Arc::ptr_eq(&one.runtime, &two.runtime), "each load registers the plugin anew");
        assert!(Arc::ptr_eq(one.runtime.code(), two.runtime.code()) && Arc::ptr_eq(two.runtime.code(), three.runtime.code()));
        assert_eq!((two.compilations(), three.compilations()), (0, 0), "and compiles nothing again");

        // a changed module is other code, compiled once, at the next load
        std::fs::write(dir.path().join("router_now.wat"), module("own, changed")).unwrap();
        let changed = loads.last().unwrap().reloader()().expect("the ledger reloads");
        assert!(!Arc::ptr_eq(three.runtime.code(), first(&changed).runtime.code()));
        assert_eq!(first(&changed).compilations(), 1);
        let again = changed.reloader()().expect("the ledger reloads");
        assert_eq!(first(&again).compilations(), 0);
        assert_eq!(first(&again).module_bytes(), module("own, changed").as_slice());
    }

    /// What is kept is bounded, the least recently used going first
    #[test]
    fn the_least_recently_used_goes_first() {
        let mut kept = vec![];
        for item in 0..3 {
            assert_eq!(*super::keep(&mut kept, 3, None, || item), item);
        }
        // 0 used again is the most recently used: 1 is the least
        let found = kept.iter().position(|it| *it == 0);
        assert_eq!(*super::keep(&mut kept, 3, found, || unreachable!("0 is kept")), 0);
        assert_eq!(*super::keep(&mut kept, 3, None, || 3), 3);
        assert_eq!(kept, [2, 0, 3]);
    }
}
