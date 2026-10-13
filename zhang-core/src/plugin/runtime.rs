//! Compile a plugin's module once per registration and instantiate it per call.
//!
//! Compiling a wasm module is the expensive part of running a plugin (hundreds of milliseconds for
//! an interpreter-sized module), while instantiating compiled code costs a fraction of a
//! millisecond. A [`PluginRuntime`] therefore owns a registered plugin's module bytes and a pool of
//! [`CompiledPlugin`]s for the latest [`Manifest`] it was asked for; every call into the plugin
//! (registration, a stage run, a router request) leases one of them and gets a fresh
//! [`PluginInstance`] built from it with [`Plugin::new_from_compiled`], so no wasm state survives
//! between calls (see [`crate::plugin::router`], "Isolation").
//!
//! Calls running at the same time never share compiled code. A `CompiledPlugin` owns the wasmtime
//! engine, and extism enforces a call's timeout by bumping that engine's epoch, which interrupts
//! every instance on the engine: shared code would let one request's timeout fail a concurrent
//! request of the same plugin. So a compiled plugin is leased to exactly one instance at a time, a
//! call finding none idle compiles another, and at most [`IDLE_LIMIT`] are kept idle per plugin.
//!
//! The host functions are linked once, when the module is compiled, so they cannot capture the host
//! of one call. They resolve the host of the instance calling them through [`InstanceBindings`], a
//! map from extism's instance id to a weak reference to the instance's context, registered as soon
//! as the instance exists and removed when it drops: the compiled code never retains a call's ledger
//! host, clock or file access.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use extism::{CompiledPlugin, CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, ValType, Wasm, EXTISM_USER_MODULE};
use uuid::Uuid;

use crate::plugin::host::PluginHost;

/// the most compiled plugins a runtime keeps idle for its current settings. Concurrent calls each
/// lease their own, so a burst past this count compiles afresh and drops the extra when it ends:
/// a plugin never holds more than this many engines once the burst is over
const IDLE_LIMIT: usize = 4;

/// how close to its timeout a call may end and still return its compiled plugin to the pool. extism's
/// timer bumps the engine's epoch at the deadline from its own thread, so a call ending within a
/// moment of its deadline can be followed by a late bump that would stop the next instance on that
/// engine. Such a call's compiled plugin is dropped instead; the next call compiles afresh
const NEAR_TIMEOUT: Duration = Duration::from_millis(100);

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
    /// an error instead of a host: that is what a wasm start function or `_initialize` gets when it
    /// calls a host function, since both run while [`Plugin::new_from_compiled`] builds the
    /// instance, before its id exists and the context can be registered (the host functions a
    /// `Plugin::new` bound per instance aborted the process there instead)
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

/// the compiled code of a runtime's current settings: the compiled plugins no call is using
#[derive(Default)]
struct Pool {
    /// the settings the idle plugins were compiled with; none before the first call
    manifest: Option<Manifest>,
    /// bumped whenever `manifest` changes, so a lease taken under earlier settings is not returned
    generation: u64,
    idle: Vec<CompiledPlugin>,
}

/// a registered plugin's module and its compiled code, shared by every clone of the registration
/// (the router list and the stage list hold one each) and every instance they build
pub struct PluginRuntime {
    module_bytes: Vec<u8>,
    bindings: InstanceBindings,
    /// the compiled code of the manifest last asked for. A manifest with other settings (config,
    /// allowed hosts, timeout) replaces it, so the pool never outlives one setting per plugin
    pool: Arc<Mutex<Pool>>,
}

impl PluginRuntime {
    /// the runtime of the module `module_bytes`, compiled on first use
    pub fn new(module_bytes: Vec<u8>) -> Self {
        Self {
            module_bytes,
            bindings: InstanceBindings::default(),
            pool: Arc::default(),
        }
    }

    /// the module compiled with the settings of `manifest`, with the host functions linked in
    fn compile(&self, manifest: Manifest) -> Result<CompiledPlugin, extism::Error> {
        PluginBuilder::new(manifest.with_wasm(Wasm::data(self.module_bytes.clone())))
            .with_functions(PluginHost::functions_for_instances(&self.bindings))
            .with_wasi(true)
            .compile()
    }

    /// a fresh instance of the plugin with the settings of `manifest`, whose host functions reach
    /// `host`. It leases an idle compiled plugin of these settings, or compiles one when every one
    /// is in use (or the settings changed since the last call, which drops the idle ones). Within a
    /// load that is a compilation at registration, whose manifest has the timeout only, and one more
    /// at the first stage run or router request, which carry the full config
    pub fn instantiate(&self, manifest: Manifest, host: PluginHost) -> Result<PluginInstance, extism::Error> {
        debug_assert!(manifest.wasm.is_empty(), "the runtime owns the module bytes");
        let timeout = manifest.timeout_ms.map(Duration::from_millis);
        let (idle, generation) = {
            let mut pool = lock(&self.pool);
            if pool.manifest.as_ref() != Some(&manifest) {
                pool.manifest = Some(manifest.clone());
                pool.idle.clear();
                pool.generation += 1;
            }
            (pool.idle.pop(), pool.generation)
        };
        // compiling happens outside the lock: a call never waits for another's compilation
        let compiled = match idle {
            Some(compiled) => compiled,
            None => self.compile(manifest)?,
        };
        // this runs the module's start function, if it has one, before the context below exists
        let plugin = Plugin::new_from_compiled(&compiled)?;
        let context = Arc::new(InstanceContext { host });
        self.bindings.lock().insert(plugin.id, Arc::downgrade(&context));
        Ok(PluginInstance {
            plugin,
            bindings: self.bindings.clone(),
            _context: context,
            lease: Lease {
                compiled: Some(compiled),
                generation,
                timeout,
                pool: self.pool.clone(),
            },
        })
    }
}

fn lock(pool: &Mutex<Pool>) -> MutexGuard<'_, Pool> {
    pool.lock().unwrap_or_else(PoisonError::into_inner)
}

/// one instance of a plugin, with wasm state and a host of its own, for exactly one call
/// (registration, a stage run or a router request). Calls keep extism's timeout, error and non-zero
/// exit behaviour
pub struct PluginInstance {
    // dropped first, so the compiled plugin is idle again only once its instance is gone
    plugin: Plugin,
    bindings: InstanceBindings,
    /// keeps the context the bindings refer to alive for as long as the instance
    _context: Arc<InstanceContext>,
    lease: Lease,
}

/// the compiled plugin an instance was built from, returned to the pool when the instance drops
struct Lease {
    /// none once the lease is spent: a call ran up to its timeout on this engine
    compiled: Option<CompiledPlugin>,
    generation: u64,
    timeout: Option<Duration>,
    pool: Arc<Mutex<Pool>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(compiled) = self.compiled.take() else { return };
        let mut pool = lock(&self.pool);
        // settings changed under the lease, or enough are idle already: drop it
        if pool.generation == self.generation && pool.idle.len() < IDLE_LIMIT {
            pool.idle.push(compiled);
        }
    }
}

impl PluginInstance {
    /// whether the plugin exports `name`
    pub fn function_exists(&self, name: impl AsRef<str>) -> bool {
        self.plugin.function_exists(name)
    }

    /// call the export `name` with `input`
    pub fn call<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(&'b mut self, name: impl AsRef<str>, input: T) -> Result<U, extism::Error> {
        let started = Instant::now();
        let result = self.plugin.call(name, input);
        if self.lease.timeout.is_some_and(|timeout| started.elapsed() + NEAR_TIMEOUT >= timeout) {
            self.lease.compiled = None;
        }
        result
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

    /// how many compiled plugins are idle, and the pool's generation
    fn pool_state(runtime: &PluginRuntime) -> (usize, u64) {
        let pool = lock(&runtime.pool);
        (pool.idle.len(), pool.generation)
    }

    #[test]
    fn compiled_code_is_reused_by_later_calls_and_dropped_when_settings_change() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec());
        let first = Manifest::default().with_config([("zhang.plugin", "first")].into_iter());
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(pool_state(&runtime), (0, 1), "a live instance holds its compiled plugin");
        assert_eq!(echoed_config(&mut plugin), "first");
        drop(plugin);
        assert_eq!(pool_state(&runtime), (1, 1), "a dropped instance returns its compiled plugin");
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(pool_state(&runtime), (0, 1), "the same settings reuse the compiled plugin");
        assert_eq!(echoed_config(&mut plugin), "first");

        let changed = first.clone().with_config([("zhang.plugin", "second")].into_iter());
        let mut changed_plugin = runtime.instantiate(changed.clone(), host("2026-10-06T00:00:00Z")).unwrap();
        assert_eq!(pool_state(&runtime), (0, 2), "changed settings start a new generation");
        assert_eq!(echoed_config(&mut changed_plugin), "second");
        // a live instance of the replaced code keeps its own settings, and is not returned
        assert_eq!(echoed_config(&mut plugin), "first");
        drop(plugin);
        assert_eq!(pool_state(&runtime), (0, 2), "a lease of earlier settings is dropped");
        drop(changed_plugin);
        assert_eq!(pool_state(&runtime), (1, 2));

        let mut generation = 2;
        for settings in [
            changed.clone().with_allowed_host("example.com"),
            changed.with_timeout(Duration::from_millis(100)),
        ] {
            drop(runtime.instantiate(settings, host("2026-10-06T00:00:00Z")).unwrap());
            generation += 1;
            assert_eq!(pool_state(&runtime), (1, generation), "capabilities are part of the settings");
        }
    }

    #[test]
    fn concurrent_instances_get_their_own_compiled_code_and_the_pool_stays_bounded() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/router_now.wat").to_vec());
        let instances: Vec<_> = (0..IDLE_LIMIT + 2)
            .map(|_| runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap())
            .collect();
        assert_eq!(pool_state(&runtime), (0, 1), "every live instance leases its own compiled plugin");
        drop(instances);
        assert_eq!(pool_state(&runtime), (IDLE_LIMIT, 1), "a burst leaves at most the limit idle");
        let one = runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(pool_state(&runtime), (IDLE_LIMIT - 1, 1));
        drop(one);
        assert_eq!(pool_state(&runtime), (IDLE_LIMIT, 1));
    }

    #[test]
    fn a_call_running_to_its_timeout_drops_its_compiled_code() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/router_loop.wat").to_vec());
        let manifest = Manifest::default().with_timeout(Duration::from_millis(200));
        let mut plugin = runtime.instantiate(manifest, host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(plugin.call::<_, String>("router", "{}").unwrap_err().to_string(), "timeout");
        drop(plugin);
        assert_eq!(pool_state(&runtime), (0, 1), "an engine whose epoch may still be bumped is not reused");
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
        drop(first);
        // the second instance reuses the first's compiled plugin
        let mut second = runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap();
        second.call::<_, ()>("once", ()).unwrap();
        assert!(second.call::<_, ()>("once", ()).is_err(), "the fixture detects reuse of one instance");
    }

    #[test]
    fn a_start_function_calling_a_host_function_fails_cleanly() {
        // a start function runs while `Plugin::new_from_compiled` builds the instance, before its
        // id exists and a context can be bound: the host call answers an error instead of a host
        // (a host function bound per `Plugin::new` aborted the whole process here)
        let runtime = PluginRuntime::new(
            br#"(module
            (import "extism:host/user" "zhang_now" (func $now (result i64)))
            (func $init (drop (call $now)))
            (start $init)
            (func (export "name") (result i32) (i32.const 0)))"#
                .to_vec(),
        );
        let error = runtime
            .instantiate(Manifest::default(), host("2026-10-05T00:00:00Z"))
            .err()
            .expect("the start function fails");
        assert!(format!("{error:#}").contains("plugin instance has no host context"), "{error:#}");
        assert!(runtime.bindings.lock().is_empty(), "a failed instantiation leaves no binding");
    }
}
