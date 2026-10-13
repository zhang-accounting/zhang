//! Compile a plugin's module once per registration and instantiate it per call.
//!
//! Compiling a wasm module is the expensive part of running a plugin (hundreds of milliseconds for
//! an interpreter-sized module), while instantiating compiled code costs a fraction of a
//! millisecond. A [`PluginRuntime`] therefore owns a registered plugin's module bytes and keeps the
//! [`CompiledPlugin`] of the latest [`Manifest`] it was asked for; every call into the plugin
//! (registration, a stage run, a router request) gets a fresh [`PluginInstance`] built from it with
//! [`Plugin::new_from_compiled`], so no wasm state survives between calls (see
//! [`crate::plugin::router`], "Isolation").
//!
//! The host functions are linked once, when the module is compiled, so they cannot capture the host
//! of one call. They resolve the host of the instance calling them through [`InstanceBindings`], a
//! map from extism's instance id to a weak reference to the instance's context, registered before
//! the instance can run anything and removed when it drops: the compiled code never retains a
//! call's ledger host, clock or file access.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use extism::{CompiledPlugin, CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, ValType, Wasm, EXTISM_USER_MODULE};
use uuid::Uuid;

use crate::plugin::host::PluginHost;

/// what the host functions of one instance reach: the host of the call it runs
pub(super) struct InstanceContext {
    pub host: PluginHost,
}

/// the contexts of the live instances of a compiled plugin, by extism instance id. The compiled host
/// functions look up the instance calling them here, so they hold no state of any one call
#[derive(Clone, Default)]
pub(super) struct InstanceBindings(Arc<Mutex<HashMap<Uuid, Weak<InstanceContext>>>>);

impl InstanceBindings {
    /// the host function `name` of the `extism:host/user` namespace, running `callback` with the
    /// context of the instance calling it. The lookup lock is released before `callback` runs, so a
    /// slow host call (a query) never blocks another instance's. An instance without a context gets
    /// an error; [`PluginRuntime::instantiate`] registers one before the instance can run, so that
    /// is a host bug and nothing a plugin can cause
    pub fn function(
        &self, name: &str, params: impl IntoIterator<Item = ValType>, results: impl IntoIterator<Item = ValType>,
        callback: impl Fn(&InstanceContext, &mut CurrentPlugin, &[Val], &mut [Val]) -> Result<(), extism::Error> + Send + Sync + 'static,
    ) -> Function {
        let bindings = self.clone();
        Function::new(name, params, results, UserData::new(()), move |plugin, inputs, outputs, _| {
            let context = bindings
                .lock()
                .get(&plugin.id())
                .and_then(Weak::upgrade)
                .ok_or_else(|| extism::Error::msg("plugin instance has no host context"))?;
            callback(&context, plugin, inputs, outputs)
        })
        .with_namespace(EXTISM_USER_MODULE)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Weak<InstanceContext>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// the compiled code of one manifest
struct Compiled {
    /// the settings the code was compiled with, without module bytes: the runtime owns those
    manifest: Manifest,
    plugin: Arc<CompiledPlugin>,
}

/// a registered plugin's module and its compiled code, shared by every clone of the registration
/// (the router list and the stage list hold one each) and every instance they build
pub struct PluginRuntime {
    module_bytes: Vec<u8>,
    bindings: InstanceBindings,
    /// the compiled code of the manifest last asked for. A manifest with other settings (config,
    /// allowed hosts, timeout) replaces it, so the cache never holds more than one entry per plugin
    compiled: Mutex<Option<Compiled>>,
}

impl PluginRuntime {
    /// the runtime of the module `module_bytes`, compiled on first use
    pub fn new(module_bytes: Vec<u8>) -> Self {
        Self {
            module_bytes,
            bindings: InstanceBindings::default(),
            compiled: Mutex::new(None),
        }
    }

    /// the compiled code of the module with the settings of `manifest`: the code compiled last when
    /// its settings are the same, else a new compilation replacing it. The lock is held across the
    /// compilation, so concurrent first calls compile once; that is once per load per plugin
    fn compile(&self, manifest: Manifest) -> Result<Arc<CompiledPlugin>, extism::Error> {
        debug_assert!(manifest.wasm.is_empty(), "the runtime owns the module bytes");
        let mut cached = self.compiled.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(compiled) = cached.as_ref().filter(|compiled| compiled.manifest == manifest) {
            return Ok(compiled.plugin.clone());
        }
        let plugin = Arc::new(
            PluginBuilder::new(manifest.clone().with_wasm(Wasm::data(self.module_bytes.clone())))
                .with_functions(PluginHost::functions_for_instances(&self.bindings))
                .with_wasi(true)
                .compile()?,
        );
        *cached = Some(Compiled {
            manifest,
            plugin: plugin.clone(),
        });
        Ok(plugin)
    }

    /// a fresh instance of the plugin with the settings of `manifest`, whose host functions reach `host`
    pub fn instantiate(&self, manifest: Manifest, host: PluginHost) -> Result<PluginInstance, extism::Error> {
        let compiled = self.compile(manifest)?;
        // extism instantiates the module lazily, on the first call, so the binding is in place before
        // any of the plugin's code (a start function included) can call a host function
        let plugin = Plugin::new_from_compiled(&compiled)?;
        let context = Arc::new(InstanceContext { host });
        self.bindings.lock().insert(plugin.id, Arc::downgrade(&context));
        Ok(PluginInstance {
            plugin,
            bindings: self.bindings.clone(),
            _context: context,
        })
    }
}

/// one instance of a plugin, with wasm state and a host of its own, for exactly one call
/// (registration, a stage run or a router request). Calls keep extism's timeout, error and non-zero
/// exit behaviour
pub struct PluginInstance {
    plugin: Plugin,
    bindings: InstanceBindings,
    /// keeps the context the bindings refer to alive for as long as the instance
    _context: Arc<InstanceContext>,
}

impl PluginInstance {
    /// whether the plugin exports `name`
    pub fn function_exists(&self, name: impl AsRef<str>) -> bool {
        self.plugin.function_exists(name)
    }

    /// call the export `name` with `input`
    pub fn call<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(&'b mut self, name: impl AsRef<str>, input: T) -> Result<U, extism::Error> {
        self.plugin.call(name, input)
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        self.bindings.lock().remove(&self.plugin.id);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use serde_json::{json, Value};

    use super::*;
    use crate::clock::{Clock, LoadClock};

    fn host(date: &str) -> PluginHost {
        let instant = DateTime::parse_from_rfc3339(date).unwrap().with_timezone(&Utc);
        PluginHost::new("cache-test", Default::default(), LoadClock::new(Clock::Fixed(instant)), chrono_tz::UTC)
    }

    /// the content of the single comment `config_echo.wat` answers: its `zhang.plugin` config
    fn echoed_config(plugin: &mut PluginInstance) -> Value {
        let output: String = plugin.call("processor", "[]").unwrap();
        serde_json::from_str::<Value>(&output).unwrap()[0]["data"]["Comment"]["content"].clone()
    }

    #[test]
    fn compiled_modules_are_shared_and_changed_settings_are_not_reused() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec());
        let first = Manifest::default().with_config([("zhang.plugin", "first")].into_iter());
        let compiled = runtime.compile(first.clone()).unwrap();
        assert!(
            Arc::ptr_eq(&compiled, &runtime.compile(first.clone()).unwrap()),
            "the same settings must reuse the compiled code"
        );
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(echoed_config(&mut plugin), "first");

        let changed = first.clone().with_config([("zhang.plugin", "second")].into_iter());
        assert!(!Arc::ptr_eq(&compiled, &runtime.compile(changed.clone()).unwrap()));
        let mut changed_plugin = runtime.instantiate(changed.clone(), host("2026-10-06T00:00:00Z")).unwrap();
        assert_eq!(echoed_config(&mut changed_plugin), "second");
        // a live instance of the replaced code keeps its own settings
        assert_eq!(echoed_config(&mut plugin), "first");

        let mut previous = runtime.compile(changed.clone()).unwrap();
        for settings in [
            changed.clone().with_allowed_host("example.com"),
            changed.with_timeout(std::time::Duration::from_millis(100)),
        ] {
            let next = runtime.compile(settings).unwrap();
            assert!(!Arc::ptr_eq(&previous, &next), "capabilities are part of the cache key");
            previous = next;
        }
    }

    #[test]
    fn cached_callbacks_use_each_instances_host_and_release_it_on_drop() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/router_now.wat").to_vec());
        let manifest = Manifest::default();
        let mut first = runtime.instantiate(manifest.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        let mut second = runtime.instantiate(manifest, host("2026-10-06T00:00:00Z")).unwrap();
        assert_ne!(first.plugin.id, second.plugin.id);
        let weak_context = Arc::downgrade(&first._context);
        for (use_second, expected) in [(true, "2026-10-06"), (false, "2026-10-05"), (true, "2026-10-06")] {
            let plugin = if use_second { &mut second } else { &mut first };
            let output: String = plugin.call("router", "{}").unwrap();
            let response: Value = serde_json::from_str(&output).unwrap();
            let now: Value = serde_json::from_str(response["body"].as_str().unwrap()).unwrap();
            assert_eq!(now["Ok"]["today"], json!(expected));
        }
        drop(first);
        assert!(weak_context.upgrade().is_none(), "the compiled code must not retain an instance's host context");
        drop(second);
        assert!(runtime.bindings.lock().is_empty(), "dropped instances must not accumulate bindings");
    }

    #[test]
    fn cached_instances_preserve_nonzero_exit_errors() {
        let runtime = PluginRuntime::new(br#"(module (func (export "fail") (result i32) (i32.const 7)))"#.to_vec());
        let mut plugin = runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap();
        assert!(plugin.call::<_, String>("fail", ()).unwrap_err().to_string().contains("non-zero exit code: 7"));
    }

    #[test]
    fn reusing_compilation_does_not_reuse_wasm_globals() {
        let runtime = PluginRuntime::new(
            br#"(module
            (global $calls (mut i32) (i32.const 0))
            (func (export "once") (result i32)
                (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
                (if (i32.ne (global.get $calls) (i32.const 1)) (then unreachable))
                (i32.const 0)))"#
                .to_vec(),
        );
        let mut first = runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap();
        first.call::<_, ()>("once", ()).unwrap();
        let mut second = runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap();
        second.call::<_, ()>("once", ()).unwrap();
        assert!(first.call::<_, ()>("once", ()).is_err(), "the fixture detects reuse of one instance");
    }
}
