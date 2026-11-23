//! The compiled code of the default plugins, kept across loads.
//!
//! A load registers every plugin anew, and a plugin's runtime compiles its module for the settings it runs with
//! ([`PluginRuntime`]): hundreds of milliseconds for a module the size of the Lua interpreter or of an analyzer written
//! with the SDK, at every load, and so at every write, which reloads the ledger. A default plugin
//! ([`crate::analysis::LoadOptions::default_plugins`]) is the same module at every load of every ledger, so its
//! runtime is kept here, by its module and its settings ([`Manifest`]: its config, the ledger's options included, its
//! allowed hosts, its timeout and its memory limit): a load with the same module and settings reuses the runtime, and
//! with it the compiled code the runtime keeps idle. Other settings, such as an option of the ledger changed, get a
//! runtime of their own, compiled on first use.
//!
//! A kept runtime is shared by the loads that use it, each instance leasing compiled code of its own as before, and
//! keeps no ledger: the host functions reach each instance's host through the instance ([`PluginRuntime`],
//! "InstanceBindings"). At most [`KEPT`] runtimes are kept, the least recently used one going first.

use std::sync::{Arc, Mutex, PoisonError};

use extism::Manifest;

use super::runtime::PluginRuntime;

/// the most runtimes kept: a server serves one ledger, whose default plugins are a few
const KEPT: usize = 4;

/// a runtime kept, and the settings it is kept for
struct Kept {
    manifest: Manifest,
    runtime: Arc<PluginRuntime>,
}

/// the runtimes kept, the most recently used last
static RUNTIMES: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// The runtime of the module `module_bytes` for the settings of `manifest`: the one kept for them, or a new one, kept
/// from now on. A runtime kept is found by the module's bytes and the settings, compared whole.
pub(crate) fn runtime(module_bytes: Vec<u8>, manifest: &Manifest) -> Arc<PluginRuntime> {
    let mut kept = RUNTIMES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = kept
        .iter()
        .position(|it| it.manifest == *manifest && it.runtime.module_bytes() == module_bytes.as_slice())
    {
        let found = kept.remove(at);
        let runtime = found.runtime.clone();
        kept.push(found);
        return runtime;
    }
    let runtime = Arc::new(PluginRuntime::new(module_bytes));
    if kept.len() >= KEPT {
        kept.remove(0);
    }
    kept.push(Kept {
        manifest: manifest.clone(),
        runtime: runtime.clone(),
    });
    runtime
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
    use crate::plugin_policy::PluginPolicy;

    /// a router: registered at every load, and never run by one
    const MODULE: &[u8] = include_bytes!("../../tests/plugins/router_now.wat");

    fn options(module: &str) -> LoadOptions {
        LoadOptions {
            plugin_policy: Arc::new(PluginPolicy::default()),
            analysis: None,
            default_plugins: vec![DefaultPlugin {
                module: module.to_owned(),
                file: "router_now.wat".to_owned(),
                content: Arc::from(MODULE),
            }],
        }
    }

    fn load(dir: &Path, options: LoadOptions) -> Ledger {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        Ledger::load_with_options(dir.to_path_buf(), "main.zhang".to_owned(), source, Clock::System, options).expect("the ledger loads")
    }

    /// The runtime of the default plugin of `ledger`.
    fn runtime(ledger: &Ledger) -> Arc<super::PluginRuntime> {
        let (plugin, _) = &ledger.plugins.ordered[0];
        assert!(plugin.is_default());
        plugin.runtime.clone()
    }

    #[test]
    fn a_reload_compiles_an_unchanged_default_module_no_more() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let first = load(dir.path(), options("zhang:cache-test-reload"));
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
        // the settings changed with an option of the ledger: compiled again, once, for them
        std::fs::write(dir.path().join("main.zhang"), "option \"title\" \"Home\"\n1970-01-01 open Assets:Cash\n").unwrap();
        let third = second.reloader()().expect("the ledger reloads");
        assert!(!Arc::ptr_eq(&runtime(&second), &runtime(&third)));
        assert_eq!(runtime(&third).compilations(), 1);
        let fourth = third.reloader()().expect("the ledger reloads");
        assert!(Arc::ptr_eq(&runtime(&third), &runtime(&fourth)));
        assert_eq!(runtime(&fourth).compilations(), 1);
        assert!(
            fourth.errors.iter().all(|error| error.error_type != ErrorKind::PluginError),
            "{:?}",
            fourth.errors
        );
    }

    #[test]
    fn a_ledgers_own_plugin_is_compiled_at_every_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("router_now.wat"), MODULE).unwrap();
        std::fs::write(
            dir.path().join("main.zhang"),
            "option \"features.plugin\" \"true\"\nplugin \"router_now.wat\"\n1970-01-01 open Assets:Cash\n",
        )
        .unwrap();
        let first = load(dir.path(), LoadOptions::with_policy(Arc::new(PluginPolicy::default())));
        let second = first.reloader()().expect("the ledger reloads");
        let (one, _) = &first.plugins.ordered[0];
        let (two, _) = &second.plugins.ordered[0];
        assert!(!one.is_default());
        assert!(!Arc::ptr_eq(&one.runtime, &two.runtime));
        assert_eq!((one.runtime.compilations(), two.runtime.compilations()), (1, 1));
    }
}
