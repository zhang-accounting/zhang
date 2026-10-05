//! Reuse compiled modules while keeping every execution's WASM and host state separate.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use extism::{CompiledPlugin, CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, ValType, Wasm, EXTISM_USER_MODULE};
use uuid::Uuid;

use super::host::PluginHost;
use super::router::RouterCall;

pub(super) struct InstanceContext {
    pub host: PluginHost,
    pub router: RouterCall,
}

/// Compiled callbacks resolve state by instance id, including calls made by a WASM
/// start function. No request's ledger or permissions are captured by the cache.
#[derive(Clone, Default)]
pub(super) struct InstanceBindings(Arc<Mutex<HashMap<Uuid, Weak<InstanceContext>>>>);

impl InstanceBindings {
    pub fn function(
        &self, name: &str, params: impl IntoIterator<Item = ValType>, results: impl IntoIterator<Item = ValType>,
        callback: impl Fn(&InstanceContext, &mut CurrentPlugin, &[Val], &mut [Val]) -> Result<(), extism::Error> + Send + Sync + 'static,
    ) -> Function {
        let bindings = self.clone();
        Function::new(name, params, results, UserData::new(()), move |plugin, inputs, outputs, _| {
            let context = bindings
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&plugin.id())
                .and_then(Weak::upgrade)
                .ok_or_else(|| extism::Error::msg("plugin instance has no host context"))?;
            callback(&context, plugin, inputs, outputs)
        })
        .with_namespace(EXTISM_USER_MODULE)
    }
}

struct CachedPlugin {
    /// The complete manifest without module bytes; modules belong to this runtime.
    manifest: Manifest,
    compiled: Arc<CompiledPlugin>,
}

pub(super) struct PluginRuntime {
    module_bytes: Vec<u8>,
    bindings: InstanceBindings,
    /// Keep only the latest configuration, so changing options cannot grow this cache.
    compiled: Mutex<Option<CachedPlugin>>,
}

impl PluginRuntime {
    pub fn new(module_bytes: Vec<u8>) -> Self {
        Self {
            module_bytes,
            bindings: InstanceBindings::default(),
            compiled: Mutex::new(None),
        }
    }

    fn compile(&self, manifest: Manifest) -> Result<Arc<CompiledPlugin>, extism::Error> {
        debug_assert!(manifest.wasm.is_empty(), "the runtime owns its module bytes");
        // Hold the lock through compilation: concurrent first requests compile once.
        let mut cached = self.compiled.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = cached.as_ref().filter(|cached| cached.manifest == manifest) {
            return Ok(cached.compiled.clone());
        }
        let functions = PluginHost::functions_for_instances(&self.bindings)
            .into_iter()
            .chain(super::router::functions_for_instances(&self.bindings));
        let compiled = Arc::new(
            PluginBuilder::new(manifest.clone().with_wasm(Wasm::data(self.module_bytes.clone())))
                .with_functions(functions)
                .with_wasi(true)
                .compile()?,
        );
        *cached = Some(CachedPlugin {
            manifest,
            compiled: compiled.clone(),
        });
        Ok(compiled)
    }

    pub fn instantiate(&self, manifest: Manifest, host: PluginHost, router: RouterCall) -> Result<PluginInstance, extism::Error> {
        let compiled = self.compile(manifest)?;
        let plugin = Plugin::new_from_compiled(&compiled)?;
        let context = Arc::new(InstanceContext { host, router });
        self.bindings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(plugin.id, Arc::downgrade(&context));
        Ok(PluginInstance {
            plugin,
            bindings: self.bindings.clone(),
            _context: context,
        })
    }
}

/// An independent Extism instance with host bindings that live exactly as long as
/// it does. Calls retain Extism's normal timeout, error and nonzero-exit behavior.
pub struct PluginInstance {
    plugin: Plugin,
    bindings: InstanceBindings,
    _context: Arc<InstanceContext>,
}

impl PluginInstance {
    pub fn function_exists(&self, name: impl AsRef<str>) -> bool {
        self.plugin.function_exists(name)
    }

    pub fn call<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(&'b mut self, name: impl AsRef<str>, input: T) -> Result<U, extism::Error> {
        self.plugin.call(name, input)
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        self.bindings.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.plugin.id);
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

    #[test]
    fn compiled_modules_are_shared_and_changed_settings_are_not_reused() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec());
        let first = Manifest::default().with_config([("zhang.plugin", "first")].into_iter());
        let compiled = runtime.compile(first.clone()).unwrap();
        assert!(
            Arc::ptr_eq(&compiled, &runtime.compile(first.clone()).unwrap()),
            "the same settings must reuse compilation"
        );
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z"), RouterCall::default()).unwrap();
        let output: String = plugin.call("processor", "[]").unwrap();
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap()[0]["data"]["Comment"]["content"], "first");

        let changed = first.clone().with_config([("zhang.plugin", "second")].into_iter());
        assert!(!Arc::ptr_eq(&compiled, &runtime.compile(changed.clone()).unwrap()));
        let mut changed_plugin = runtime
            .instantiate(changed.clone(), host("2026-10-06T00:00:00Z"), RouterCall::default())
            .unwrap();
        let output: String = changed_plugin.call("processor", "[]").unwrap();
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap()[0]["data"]["Comment"]["content"], "second");
        // A live old instance keeps its own configuration even after replacement.
        let output: String = plugin.call("processor", "[]").unwrap();
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap()[0]["data"]["Comment"]["content"], "first");

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
        let mut first = runtime
            .instantiate(manifest.clone(), host("2026-10-05T00:00:00Z"), RouterCall::default())
            .unwrap();
        let mut second = runtime.instantiate(manifest, host("2026-10-06T00:00:00Z"), RouterCall::default()).unwrap();
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
        assert!(
            weak_context.upgrade().is_none(),
            "the compilation cache must not retain an execution's host context"
        );
        drop(second);
        assert!(runtime.bindings.0.lock().unwrap().is_empty(), "dropped instances must not accumulate bindings");
    }

    #[test]
    fn cached_instances_preserve_nonzero_exit_errors() {
        let runtime = PluginRuntime::new(br#"(module (func (export "fail") (result i32) (i32.const 7)))"#.to_vec());
        let mut plugin = runtime
            .instantiate(Manifest::default(), host("2026-10-05T00:00:00Z"), RouterCall::default())
            .unwrap();
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
        let mut first = runtime
            .instantiate(Manifest::default(), host("2026-10-05T00:00:00Z"), RouterCall::default())
            .unwrap();
        first.call::<_, ()>("once", ()).unwrap();
        let mut second = runtime
            .instantiate(Manifest::default(), host("2026-10-05T00:00:00Z"), RouterCall::default())
            .unwrap();
        second.call::<_, ()>("once", ()).unwrap();
        assert!(first.call::<_, ()>("once", ()).is_err(), "the fixture detects reuse of one instance");
    }
}
