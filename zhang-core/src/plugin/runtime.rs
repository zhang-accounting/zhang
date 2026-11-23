//! Compile a plugin's module once per process and instantiate it per call.
//!
//! Compiling a wasm module is the expensive part of running a plugin (hundreds of milliseconds for
//! an interpreter-sized module), while instantiating compiled code costs a fraction of a
//! millisecond. A module's compiled code ([`ModuleCode`]) is a pool of [`CompiledPlugin`]s, shared
//! by every [`PluginRuntime`] running the module ([`crate::plugin::cache::code`]); every call into
//! a plugin (registration, a stage run, a router request) leases one of them and gets a fresh
//! [`PluginInstance`] built from it with [`Plugin::new_from_compiled_with_manifest`], so no wasm
//! state survives between calls (see [`crate::plugin::router`], "Isolation").
//!
//! Compiled code does not depend on a plugin's settings. extism reads the settings of a
//! [`Manifest`] (its config, a Lua script included, its allowed hosts, its timeout and its memory
//! limits) only into the state of an instance, which each instance gets afresh from the manifest
//! of its call; the module's bytes and wasmtime's settings, which are the same for every module
//! ([`compile_config`]), are all compiling reads. So one compiled plugin serves every plugin of its
//! module, whatever its settings, one call at a time: the Lua interpreter every Lua plugin runs on
//! is compiled once per process, not once per script, per load or per changed option.
//!
//! Calls running at the same time never share compiled code. A `CompiledPlugin` owns the wasmtime
//! engine, and extism enforces a call's timeout by bumping that engine's epoch, which interrupts
//! every instance on the engine: shared code would let one request's timeout fail a concurrent
//! request of the same plugin. So a compiled plugin is leased to exactly one instance at a time, and a
//! call finding none idle compiles another.
//!
//! What is kept idle is bounded, as each copy holds the module's native code: about 1.8 MiB for the Lua interpreter,
//! 3 MiB for a plugin of about 800 KiB built with the SDK, 0.1 MiB for a small hand-written module. A module keeps one
//! copy idle, which every load and every call after another reuses. Only calls running at the same time need more,
//! and only the server's own modules keep more for them ([`ModuleCode::keep_spares`]: the Lua interpreter every script
//! runs on, a default plugin's module), at most [`SPARE_LIMIT`] over all of them; a ledger's own module keeps none.
//! A burst past that compiles afresh (or reads the code back from the disk cache) and drops the extra copies when it
//! ends.
//!
//! The host functions are linked once, when the module is compiled, so they cannot capture the host
//! of one call. They resolve the host of the instance calling them through [`InstanceBindings`], a
//! map from extism's instance id to a weak reference to the instance's context, registered as soon
//! as the instance exists and removed when it drops: the compiled code never retains a call's ledger
//! host, clock or file access.
//!
//! Compiling goes through the disk cache of [`crate::plugin::disk_cache`]: a module compiled before, by this process or
//! an earlier one, is read back instead of compiled again. The cache keeps code by the module's bytes and wasmtime's
//! settings, so one entry serves every config of a module.
//!
//! A compilation runs on one thread of its own, which gives the memory the compilation took back before it ends (when
//! it compiled, rather than read the code back from the disk cache). wasmtime compiles a module's functions in parallel
//! by default, on rayon's global pool: a thread per CPU the process may use, each taking about 10 MiB for the Lua
//! interpreter every Lua plugin runs on, which the allocator kept after the compilation (mimalloc keeps what a thread
//! freed for that thread, and rayon's threads stay, idle). A cold start with one Lua plugin took 192 MiB on 4 CPUs, 270
//! on 8 and 906 on 64, and kept it; on a host of 96 CPUs, containers of 384 MiB to 1 GiB were killed at start. Compiled
//! on one thread, the peak is the same on any machine (103 to 111 MiB with the plugin, 49 without), and the memory goes
//! back once the compilation is over (57 to 63 MiB). It takes longer, once per module and process (or disk cache): 0.4 s
//! for the Lua interpreter, where 64 threads took 0.06 s, 8 took 0.08 s and 4 took 0.12 s; reading the code back from
//! the disk cache takes 3 ms either way. See [`compile_config`] and [`crate::memory`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use extism::{Cache, CompiledPlugin, CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, ValType, Wasm, EXTISM_USER_MODULE};
use log::warn;
use uuid::Uuid;

use crate::plugin::disk_cache;
use crate::plugin::host::PluginHost;

/// the name of the thread a compilation runs on ([`ModuleCode::compile`])
const COMPILE_THREAD: &str = "zhang-compile";

/// The wasmtime settings every plugin compiles with, under the ones extism sets: one thread compiles all of a module's
/// functions, the one that asked (see the module's documentation for why). wasmtime's cache keeps code by settings that
/// do not include this one, so code an earlier version compiled in parallel is read back as before.
fn compile_config() -> wasmtime::Config {
    let mut config = wasmtime::Config::new();
    config.parallel_compilation(false);
    config
}

/// the most spare compiled plugins kept idle over every module that keeps spares ([`SPARES`]): copies beyond a module's
/// first idle one, which only calls running at the same time use. With the one every module keeps, the Lua interpreter
/// keeps at most 4 idle, about 7 MiB, as many as it kept before every module's were bounded together
pub(crate) const SPARE_LIMIT: usize = 3;

/// A count of the spare compiled plugins kept idle by the modules sharing it, at most `limit`: a module's idle copies
/// beyond its first
pub(crate) struct Spares {
    kept: Mutex<usize>,
    limit: usize,
}

impl Spares {
    pub(crate) const fn new(limit: usize) -> Self {
        Self { kept: Mutex::new(0), limit }
    }

    fn lock(&self) -> MutexGuard<'_, usize> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// count one more spare, unless `limit` are kept already: whether it may be kept
    fn take(&self) -> bool {
        let mut kept = self.lock();
        let taken = *kept < self.limit;
        if taken {
            *kept += 1;
        }
        taken
    }

    /// count `count` spares fewer, leased again or dropped
    fn release(&self, count: usize) {
        *self.lock() -= count;
    }

    /// how many spares are kept
    #[cfg(test)]
    pub(crate) fn kept(&self) -> usize {
        *self.lock()
    }
}

/// the spares of the server's own modules ([`ModuleCode::keep_spares`])
pub(crate) static SPARES: Spares = Spares::new(SPARE_LIMIT);

/// the least margin before its timeout a call must end with to return its compiled plugin to the
/// pool; see [`near_timeout`]
const NEAR_TIMEOUT_FLOOR: Duration = Duration::from_millis(100);

/// how close to `timeout` a call may end, or an instance be built, and still return its compiled
/// plugin to the pool: a tenth of the timeout, and at least [`NEAR_TIMEOUT_FLOOR`]. extism's timer
/// bumps the engine's epoch at the deadline from its own thread, so a call ending within a moment of
/// its deadline can be followed by a late bump that would stop the next instance on that engine.
/// Such a call's compiled plugin is dropped instead; the next call compiles afresh. Building an
/// instance runs the module's start function and initialisation under the same timer
fn near_timeout(timeout: Duration) -> Duration {
    (timeout / 10).max(NEAR_TIMEOUT_FLOOR)
}

/// what the host functions of one instance reach: the host of the call it runs
pub(super) struct InstanceContext {
    pub host: PluginHost,
}

/// the contexts of the live instances of a module's compiled plugins, by extism instance id. The
/// compiled host functions look up the instance calling them here, so they hold no state of any one
/// call, nor of any one plugin of the module
#[derive(Clone, Default)]
pub(super) struct InstanceBindings(Arc<Mutex<HashMap<Uuid, Weak<InstanceContext>>>>);

impl InstanceBindings {
    /// the host function `name` of the `extism:host/user` namespace, running `callback` with the
    /// context of the instance calling it. The lookup lock is released before `callback` runs, so a
    /// slow host call (a query) never blocks another instance's. An instance without a context gets
    /// an error instead of a host: that is what a wasm start function or `_initialize` gets when it
    /// calls a host function, since both run while [`Plugin::new_from_compiled_with_manifest`]
    /// builds the instance, before its id exists and the context can be registered (the host
    /// functions a `Plugin::new` bound per instance aborted the process there instead)
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

/// A module and its compiled code, for every plugin running the module whatever its settings: the compiled plugins no
/// instance is using, each with an engine of its own, and the bindings their host functions reach the instances
/// through. Shared by the runtimes of the module ([`crate::plugin::cache::code`]) and the instances they build
pub(crate) struct ModuleCode {
    module_bytes: Vec<u8>,
    bindings: InstanceBindings,
    /// the compiled plugins no instance is using: one, and the spares [`ModuleCode::spares`] lets the module keep
    idle: Mutex<Vec<CompiledPlugin>>,
    /// where the module's spare idle copies are counted, once it keeps spares ([`ModuleCode::keep_spares`])
    spares: OnceLock<&'static Spares>,
    /// the disk cache compiling reads and writes, [`disk_cache::shared`]; none compiles without one
    disk_cache: Option<Cache>,
}

impl ModuleCode {
    /// the module `module_bytes`, compiled on first use, keeping no spare copy idle
    pub(crate) fn new(module_bytes: Vec<u8>) -> Self {
        Self::with_disk_cache(module_bytes, disk_cache::shared().cloned())
    }

    /// the module `module_bytes`, compiled on first use with the disk cache `disk_cache`, or none
    fn with_disk_cache(module_bytes: Vec<u8>, disk_cache: Option<Cache>) -> Self {
        Self {
            module_bytes,
            bindings: InstanceBindings::default(),
            idle: Mutex::default(),
            spares: OnceLock::new(),
            disk_cache,
        }
    }

    /// Let the module keep spare idle copies, counted in `spares` with those of the other modules counted there: the
    /// server's own modules, whose calls run at the same time (the Lua interpreter's scripts serve requests). A module
    /// keeps spares from the first call on: later ones change nothing
    pub(crate) fn keep_spares(&self, spares: &'static Spares) {
        let _ = self.spares.set(spares);
    }

    /// whether the module keeps spare idle copies, counted in `spares`
    #[cfg(test)]
    pub(crate) fn keeps_spares_in(&self, spares: &'static Spares) -> bool {
        self.spares.get().is_some_and(|it| std::ptr::eq(*it, spares))
    }

    /// an idle compiled plugin of the module, if any, now leased: a spare no longer
    fn lease_idle(&self) -> Option<CompiledPlugin> {
        let mut idle = self.idle();
        let compiled = idle.pop()?;
        // one is still idle: the one leased was a spare
        if !idle.is_empty() {
            self.release_spares(1);
        }
        Some(compiled)
    }

    /// keep `compiled`, whose instance is gone, idle: as the module's first idle copy, or as a spare while it keeps
    /// spares and fewer than their limit are kept. Dropped otherwise
    fn return_idle(&self, compiled: CompiledPlugin) {
        let mut idle = self.idle();
        if idle.is_empty() || self.spares.get().is_some_and(|spares| spares.take()) {
            idle.push(compiled);
        }
    }

    fn release_spares(&self, count: usize) {
        if let Some(spares) = self.spares.get() {
            spares.release(count);
        }
    }

    /// the module's bytes
    pub(crate) fn module_bytes(&self) -> &[u8] {
        &self.module_bytes
    }

    fn idle(&self) -> MutexGuard<'_, Vec<CompiledPlugin>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// the module compiled, with the host functions linked in and no settings of any plugin, which each instance gets
    /// when it is built: on a thread of its own ([`COMPILE_THREAD`]), which compiles alone ([`compile_config`]) and,
    /// when it compiled rather than read the code back from the disk cache, gives back what the compilation freed
    /// before it ends ([`crate::memory::return_freed`]), so that the memory of the thread that asked, a ledger's load
    /// or a request, is left as it was
    fn compile(&self) -> Result<CompiledPlugin, extism::Error> {
        let builder = PluginBuilder::new(Manifest::default().with_wasm(Wasm::data(self.module_bytes.clone())))
            .with_functions(PluginHost::functions_for_instances(&self.bindings))
            .with_wasi(true)
            .with_wasmtime_config(compile_config());
        // never extism's own cache, which `EXTISM_CACHE_CONFIG` or wasmtime's configuration file set up
        let builder = match &self.disk_cache {
            Some(cache) => builder.with_cache(cache.clone()),
            None => builder.with_cache_disabled(),
        };
        let cache = self.disk_cache.clone();
        on_a_thread_of_its_own(std::thread::Builder::new().name(COMPILE_THREAD.to_owned()), move || {
            let misses = cache.as_ref().map(Cache::cache_misses);
            let compiled = builder.compile();
            // compiled rather than read back from the disk cache (a count another compilation may move too, which
            // only gives back once more): what it took and freed goes back to the system before the thread ends.
            // Reading the code back takes a few MiB, which the end of the thread hands to the allocator to reuse
            let compiled_anew = match (&cache, misses) {
                (Some(cache), Some(misses)) => cache.cache_misses() > misses,
                _ => true,
            };
            if compiled_anew {
                crate::memory::return_freed();
            }
            compiled
        })
    }
}

impl Drop for ModuleCode {
    fn drop(&mut self) {
        // its spares go with it
        let spares = self.idle.get_mut().unwrap_or_else(PoisonError::into_inner).len().saturating_sub(1);
        self.release_spares(spares);
    }
}

/// Run `job` on the thread `thread` starts, and answer what it answers; on the calling thread when no thread can be
/// started (the process is at its limit of threads or of memory), which is logged: a compilation then takes the memory
/// of the thread that asked, rather than fail the plugin's load
fn on_a_thread_of_its_own<T: Send>(thread: std::thread::Builder, job: impl FnOnce() -> T + Send) -> T {
    let mut job = Some(job);
    let pending = &mut job;
    let started = std::thread::scope(|scope| {
        thread
            .spawn_scoped(scope, move || (pending.take().expect("the job runs once"))())
            .map(|running| running.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    });
    started.unwrap_or_else(|error| {
        warn!("cannot start a thread to compile a plugin on ({error}): compiling it on this thread");
        // a thread that did not start dropped its closure, which never ran
        (job.take().expect("the job did not run"))()
    })
}

/// a registered plugin's module and its compiled code, shared by every clone of the registration
/// (the router list and the stage list hold one each) and every instance they build
pub struct PluginRuntime {
    /// the module's compiled code, shared with the other runtimes of the module
    code: Arc<ModuleCode>,
    /// how many times building an instance of this runtime compiled the module, finding no compiled plugin idle
    compilations: AtomicUsize,
    /// what the plugin declared when it registered, once a default plugin's registration read it
    registration: std::sync::OnceLock<super::store::Registration>,
}

impl PluginRuntime {
    /// the runtime of the module `module_bytes`, with compiled code of its own, compiled on first use
    pub fn new(module_bytes: Vec<u8>) -> Self {
        Self::of(Arc::new(ModuleCode::new(module_bytes)))
    }

    /// the runtime of the module whose compiled code is `code`
    pub(crate) fn of(code: Arc<ModuleCode>) -> Self {
        Self {
            code,
            compilations: AtomicUsize::new(0),
            registration: std::sync::OnceLock::new(),
        }
    }

    /// the runtime compiling with the disk cache `cache` instead of the shared one, or with none, into compiled code of
    /// its own
    #[cfg(test)]
    pub(crate) fn with_disk_cache(mut self, cache: Option<Cache>) -> Self {
        self.code = Arc::new(ModuleCode::with_disk_cache(self.code.module_bytes.clone(), cache));
        self
    }

    /// the runtime whose module keeps spare idle copies, at most `limit`, counted for it alone
    #[cfg(test)]
    pub(crate) fn keeping_spares(self, limit: usize) -> Self {
        self.code.keep_spares(Box::leak(Box::new(Spares::new(limit))));
        self
    }

    /// what the plugin declared when it registered with this runtime, if a registration kept it
    pub(crate) fn registration(&self) -> Option<super::store::Registration> {
        self.registration.get().cloned()
    }

    /// keep what the plugin declared when it registered: the next registration with this runtime reads it here
    pub(crate) fn remember_registration(&self, registration: super::store::Registration) {
        let _ = self.registration.set(registration);
    }

    /// the module's bytes, as the load read them
    pub fn module_bytes(&self) -> &[u8] {
        self.code.module_bytes()
    }

    /// the module's compiled code, shared with the other runtimes of the module
    #[cfg(test)]
    pub(crate) fn code(&self) -> &Arc<ModuleCode> {
        &self.code
    }

    /// how many times building an instance of this runtime compiled the module: none when compiled code of the module
    /// was idle, an earlier load's or another plugin's of the same module
    pub fn compilations(&self) -> usize {
        self.compilations.load(Ordering::Relaxed)
    }

    /// how many compiled plugins of the module are idle
    #[cfg(test)]
    pub(crate) fn idle(&self) -> usize {
        self.code.idle().len()
    }

    /// a fresh instance of the plugin with the settings of `manifest`, whose host functions reach
    /// `host`. It leases an idle compiled plugin of the module, whatever settings the calls before
    /// it had, or compiles one when every one is in use
    pub fn instantiate(&self, manifest: Manifest, host: PluginHost) -> Result<PluginInstance, extism::Error> {
        debug_assert!(manifest.wasm.is_empty(), "the runtime owns the module bytes");
        let timeout = manifest.timeout_ms.map(Duration::from_millis);
        let idle = self.code.lease_idle();
        // compiling happens outside the lock: a call never waits for another's compilation
        let compiled = match idle {
            Some(compiled) => compiled,
            None => {
                self.compilations.fetch_add(1, Ordering::Relaxed);
                self.code.compile()?
            }
        };
        // this runs the module's start function and a reactor's `_initialize`, if it has them, before the context below
        // exists, within the timeout of `manifest`: an instance whose initialisation runs out of it fails, saying so
        let started = Instant::now();
        let plugin = Plugin::new_from_compiled_with_manifest(&compiled, manifest).map_err(|error| match error.to_string().as_str() {
            // extism reports wasm it interrupted at the timeout as exactly "timeout"
            "timeout" => extism::Error::msg(format!(
                "its start function or initialisation (`_initialize`) ran longer than its timeout of {:?}",
                timeout.unwrap_or_default()
            )),
            _ => error,
        })?;
        // built near its timeout: the timer's bump may still come, as after a call ending near it
        let spent = timeout.is_some_and(|timeout| started.elapsed() + near_timeout(timeout) >= timeout);
        let context = Arc::new(InstanceContext { host });
        self.code.bindings.lock().insert(plugin.id, Arc::downgrade(&context));
        Ok(PluginInstance {
            plugin,
            _context: context,
            lease: Lease {
                compiled: Some(compiled),
                in_call: false,
                spent,
                timeout,
                code: self.code.clone(),
            },
        })
    }
}

/// one instance of a plugin, with wasm state, settings and a host of its own, for exactly one call
/// (registration, a stage run or a router request). Calls keep extism's timeout, error and non-zero
/// exit behaviour
pub struct PluginInstance {
    // dropped first, so the compiled plugin is idle again only once its instance is gone
    plugin: Plugin,
    /// keeps the context the bindings refer to alive for as long as the instance
    _context: Arc<InstanceContext>,
    lease: Lease,
}

/// the compiled plugin an instance was built from, returned to its module's pool when the instance drops
struct Lease {
    /// taken when the lease drops
    compiled: Option<CompiledPlugin>,
    /// set while a call runs and cleared once it returned, so a call that unwinds leaves it set
    in_call: bool,
    /// set once the instance was built, or a call of it ended, near its timeout; it stays set
    spent: bool,
    timeout: Option<Duration>,
    code: Arc<ModuleCode>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(compiled) = self.compiled.take() else { return };
        // the engine's epoch may still be bumped by a call's timer: not reused
        if self.in_call || self.spent {
            return;
        }
        self.code.return_idle(compiled);
    }
}

impl PluginInstance {
    /// whether the plugin exports `name`
    pub fn function_exists(&self, name: impl AsRef<str>) -> bool {
        self.plugin.function_exists(name)
    }

    /// let each later call of this instance run at most `at_most`, when that is less than the timeout of its settings:
    /// a shorter timeout for one instance
    pub fn limit_timeout(&mut self, at_most: Duration) {
        if self.lease.timeout.is_none_or(|timeout| at_most < timeout) {
            self.plugin.set_timeout(Some(at_most));
            self.lease.timeout = Some(at_most);
        }
    }

    /// call the export `name` with `input`
    pub fn call<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(&'b mut self, name: impl AsRef<str>, input: T) -> Result<U, extism::Error> {
        self.timed(|plugin| plugin.call(name, input))
    }

    /// Call the export `name` with `input`, and on an error the code the export returned with it. The code is the
    /// module's to choose, and an export that returns a code other than 0 without an error succeeds here: for the Lua
    /// interpreter, which sets an error with every failure and returns
    /// [`OUT_OF_LUA_MEMORY`](crate::plugin::lua::OUT_OF_LUA_MEMORY) when the script ran out of its Lua memory
    pub fn call_get_error_code<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(
        &'b mut self, name: impl AsRef<str>, input: T,
    ) -> Result<U, (extism::Error, i32)> {
        self.timed(|plugin| plugin.call_get_error_code(name, input))
    }

    /// run `call` on the plugin, spending the lease when the call does not return in time
    fn timed<'b, R>(&'b mut self, call: impl FnOnce(&'b mut Plugin) -> R) -> R {
        // the lease is spent unless the call returns in time: a call that panics (in a host function
        // or in wasmtime) unwinds through extism before it stops the call's timer, so the engine's
        // epoch is still bumped at the deadline, which would stop the next instance on that engine
        self.lease.in_call = true;
        let started = Instant::now();
        let result = call(&mut self.plugin);
        self.lease.in_call = false;
        // a call ending near its timeout spends the lease for good: a later call of this instance
        // (a mapper makes one per directive) returning in time does not make the engine safe again
        if self.lease.timeout.is_some_and(|timeout| started.elapsed() + near_timeout(timeout) >= timeout) {
            self.lease.spent = true;
        }
        result
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        self.lease.code.bindings.lock().remove(&self.plugin.id);
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

    /// how many compiled plugins of the runtime's module are idle
    fn idle(runtime: &PluginRuntime) -> usize {
        runtime.idle()
    }

    #[test]
    fn compiled_code_is_reused_by_later_calls_whatever_their_settings() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec()).keeping_spares(SPARE_LIMIT);
        let first = Manifest::default().with_config([("zhang.plugin", "first")].into_iter());
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(idle(&runtime), 0, "a live instance holds its compiled plugin");
        assert_eq!(echoed_config(&mut plugin), "first");
        drop(plugin);
        assert_eq!(idle(&runtime), 1, "a dropped instance returns its compiled plugin");
        let mut plugin = runtime.instantiate(first.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(idle(&runtime), 0, "the same settings reuse the compiled plugin");
        assert_eq!(echoed_config(&mut plugin), "first");
        drop(plugin);

        // other settings reuse it too, each instance with its own config
        let changed = first.clone().with_config([("zhang.plugin", "second")].into_iter());
        let mut changed_plugin = runtime.instantiate(changed.clone(), host("2026-10-06T00:00:00Z")).unwrap();
        assert_eq!(idle(&runtime), 0, "changed settings reuse the compiled plugin");
        assert_eq!(echoed_config(&mut changed_plugin), "second");
        // an instance of the earlier settings, at the same time, gets compiled code of its own and keeps its settings
        let mut plugin = runtime.instantiate(first, host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(echoed_config(&mut plugin), "first");
        assert_eq!(echoed_config(&mut changed_plugin), "second");
        assert_eq!(runtime.compilations(), 2, "two instances at the same time, two compiled plugins");
        drop((plugin, changed_plugin));
        assert_eq!(idle(&runtime), 2, "both are returned");

        for settings in [
            changed.clone().with_allowed_host("example.com"),
            // not within 100 ms of it: an instance built that near its timeout does not return its compiled plugin
            changed.with_timeout(Duration::from_secs(5)),
        ] {
            drop(runtime.instantiate(settings, host("2026-10-06T00:00:00Z")).unwrap());
            assert_eq!(idle(&runtime), 2, "capabilities are settings of the instance");
        }
        assert_eq!(runtime.compilations(), 2, "compiled no more");
    }

    /// the memory limit and the timeout are the instance's, from the settings of its call: an instance gets neither
    /// those of the instance before it on the same compiled plugin, nor those it was compiled with
    #[test]
    fn each_instance_runs_within_the_limits_of_its_own_settings() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/memory_hog.wat").to_vec());
        // the processor grows its memory by 32 MiB
        let small = Manifest::default().with_memory_max(256);
        let error = runtime
            .instantiate(small.clone(), host("2026-10-05T00:00:00Z"))
            .unwrap()
            .call::<&str, &str>("processor", "[]")
            .unwrap_err();
        assert_eq!(error.to_string(), "oom", "16 MiB are not enough");
        let mut roomy = runtime
            .instantiate(Manifest::default().with_memory_max(4096), host("2026-10-05T00:00:00Z"))
            .unwrap();
        assert_eq!(roomy.call::<&str, &str>("processor", "[]").unwrap(), "[]", "256 MiB are");
        drop(roomy);
        let error = runtime
            .instantiate(small, host("2026-10-05T00:00:00Z"))
            .unwrap()
            .call::<&str, &str>("processor", "[]")
            .unwrap_err();
        assert_eq!(error.to_string(), "oom", "and 16 MiB still are not, on the same compiled plugin");
        assert_eq!(runtime.compilations(), 1);

        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/loop.wat").to_vec());
        drop(
            runtime
                .instantiate(Manifest::default().with_timeout(Duration::from_secs(30)), host("2026-10-05T00:00:00Z"))
                .unwrap(),
        );
        let mut plugin = runtime
            .instantiate(Manifest::default().with_timeout(Duration::from_millis(200)), host("2026-10-05T00:00:00Z"))
            .unwrap();
        let started = Instant::now();
        assert_eq!(plugin.call::<&str, &str>("processor", "[]").unwrap_err().to_string(), "timeout");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stopped at its own timeout: {:?}",
            started.elapsed()
        );
        assert_eq!(runtime.compilations(), 1);
    }

    /// a compilation that cannot have a thread of its own runs on the thread that asked, rather than fail the load
    #[test]
    fn a_compilation_runs_on_the_calling_thread_when_no_thread_can_be_started() {
        let here = std::thread::current().id();
        // a stack larger than the address space: the thread cannot be started
        let impossible = std::thread::Builder::new().name(COMPILE_THREAD.to_owned()).stack_size(1 << 50);
        assert_eq!(on_a_thread_of_its_own(impossible, || std::thread::current().id()), here);
        let possible = std::thread::Builder::new().name(COMPILE_THREAD.to_owned());
        assert_ne!(on_a_thread_of_its_own(possible, || std::thread::current().id()), here);
    }

    /// a compilation runs on a thread of its own, which gives back what the compilation freed through the hook the
    /// binary set ([`crate::memory::set_return_freed`]) before it ends; reusing the compiled code compiles nothing
    #[test]
    fn a_compilation_returns_the_memory_it_freed_on_a_thread_of_its_own() {
        static RETURNED_ON: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());
        fn record() {
            RETURNED_ON.lock().unwrap().push(std::thread::current().name().map(str::to_owned));
        }
        crate::memory::set_return_freed(record);
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec()).with_disk_cache(None);
        let settings = Manifest::default().with_config([("zhang.plugin", "returned")].into_iter());
        drop(runtime.instantiate(settings.clone(), host("2026-10-05T00:00:00Z")).unwrap());
        drop(runtime.instantiate(settings, host("2026-10-05T00:00:00Z")).unwrap());
        assert_eq!(runtime.compilations(), 1, "the second instance reuses the compiled code");
        let returned_on = RETURNED_ON.lock().unwrap();
        assert!(!returned_on.is_empty(), "the compilation gave up what it freed");
        // the tests compiling at the same time call the hook too, as every compilation does: on its own thread
        assert!(returned_on.iter().all(|name| name.as_deref() == Some(COMPILE_THREAD)), "{returned_on:?}");
    }

    /// wasmtime compiles a plugin's functions on the thread compiling it, rather than in parallel on a thread per CPU,
    /// whose memory grew with the CPUs (see the module's documentation)
    #[test]
    fn plugins_compile_on_one_thread() {
        let config = format!("{:?}", compile_config());
        assert!(config.contains("parallel_compilation: false"), "{config}");
    }

    #[test]
    fn a_compilation_reads_back_the_code_an_earlier_one_kept_in_the_disk_cache() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache");
        let cache = disk_cache::set_up(Some(dir.clone().into_os_string()), None, None, None).expect("a writable directory is used");
        let module = include_bytes!("../../tests/plugins/config_echo.wat").to_vec();
        let settings = Manifest::default().with_config([("zhang.plugin", "cached")].into_iter());

        let first = PluginRuntime::new(module.clone()).with_disk_cache(Some(cache.clone()));
        assert_eq!(
            echoed_config(&mut first.instantiate(settings.clone(), host("2026-10-05T00:00:00Z")).unwrap()),
            "cached"
        );
        let compiled = cache.cache_misses();
        assert!(compiled > 0, "the first compilation writes the code");
        assert_eq!(cache.cache_hits(), 0);
        assert!(
            dir.join(disk_cache::SUBDIRECTORY).join("modules").is_dir(),
            "the code is kept in zhang's subdirectory"
        );

        // another runtime, as a restart or another load has: the same module and settings are read back
        let second = PluginRuntime::new(module).with_disk_cache(Some(cache.clone()));
        assert_eq!(
            echoed_config(&mut second.instantiate(settings, host("2026-10-05T00:00:00Z")).unwrap()),
            "cached"
        );
        assert_eq!((cache.cache_hits(), cache.cache_misses()), (compiled, compiled), "nothing is compiled again");
    }

    /// a directory holding a ledger keeps every file of it when the cache is set there by mistake: wasmtime's clean-up,
    /// which deletes what it does not recognise as its own, reads zhang's subdirectory only
    #[test]
    fn a_compilation_into_a_directory_holding_a_ledger_keeps_every_file_of_it() {
        let root = tempfile::tempdir().unwrap();
        let volume = root.path().join("data");
        let files = [
            ("main.zhang", "include \"data/2024/01.zhang\"\n"),
            ("data/2024/01.zhang", "2024-01-01 open Assets:Cash\n"),
            ("documents/2024/receipts/lunch.pdf", "%PDF-1.4"),
            (".zhang/passkeys.json", "[]"),
            ("notes.txt", "kept"),
        ];
        for (file, content) in files {
            let path = volume.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let cache = disk_cache::set_up(Some(volume.clone().into_os_string()), None, None, None).expect("the volume is usable");
        // a file wasmtime does not recognise in its own directory: its clean-up deletes it, which says that it ran
        let canary = volume.join(disk_cache::SUBDIRECTORY).join("canary");
        std::fs::write(&canary, "").unwrap();
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec()).with_disk_cache(Some(cache.clone()));
        let settings = Manifest::default().with_config([("zhang.plugin", "volume")].into_iter());
        assert_eq!(
            echoed_config(&mut runtime.instantiate(settings, host("2026-10-05T00:00:00Z")).unwrap()),
            "volume"
        );
        assert!(cache.cache_misses() > 0, "the compilation wrote to the cache, which starts its clean-up");
        let started = Instant::now();
        while canary.exists() {
            assert!(started.elapsed() < Duration::from_secs(30), "wasmtime's clean-up did not run");
            std::thread::sleep(Duration::from_millis(20));
        }
        for (file, content) in files {
            assert_eq!(std::fs::read_to_string(volume.join(file)).unwrap(), content, "{file} is kept");
        }
    }

    #[test]
    fn plugins_load_when_the_disk_cache_directory_cannot_be_used() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cache");
        std::fs::write(&file, "in the way").unwrap();
        let ledger = root.path().join("ledger");
        std::fs::create_dir(&ledger).unwrap();
        let mut unusable = vec![(file.clone(), None), (file.join("below"), None), (ledger.join("cache"), Some(ledger.as_path()))];
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let read_only = root.path().join("read-only");
            std::fs::create_dir(&read_only).unwrap();
            std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o500)).unwrap();
            // root writes to it all the same: no test then
            if std::fs::write(read_only.join("privileged"), "").is_err() {
                unusable.push((read_only, None));
            }
            let shared = root.path().join("shared");
            std::fs::create_dir(&shared).unwrap();
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
            unusable.push((shared, None));
        }
        for (dir, ledger_root) in unusable {
            let cache = disk_cache::set_up(Some(dir.clone().into_os_string()), None, None, ledger_root);
            assert!(cache.is_none(), "{} turns the cache off", dir.display());
            let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/config_echo.wat").to_vec()).with_disk_cache(cache);
            let settings = Manifest::default().with_config([("zhang.plugin", "uncached")].into_iter());
            let mut plugin = runtime
                .instantiate(settings, host("2026-10-05T00:00:00Z"))
                .expect("the plugin compiles without a cache");
            assert_eq!(echoed_config(&mut plugin), "uncached");
        }
    }

    #[test]
    fn an_instance_may_get_less_time_than_its_settings_without_compiling_again() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/loop.wat").to_vec());
        let settings = Manifest::default().with_timeout(Duration::from_secs(30));
        drop(runtime.instantiate(settings.clone(), host("2026-10-05T00:00:00Z")).unwrap());
        assert_eq!(idle(&runtime), 1);
        let mut plugin = runtime.instantiate(settings.clone(), host("2026-10-05T00:00:00Z")).unwrap();
        // more time than the settings give is no limit
        plugin.limit_timeout(Duration::from_secs(60));
        plugin.limit_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let error = plugin.call::<&str, &str>("processor", "[]").unwrap_err();
        assert_eq!(error.to_string(), "timeout");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stopped at the shorter timeout: {:?}",
            started.elapsed()
        );
        drop(plugin);
        // the one stopped at its deadline is not returned
        assert_eq!(idle(&runtime), 0);
        let plugin = runtime.instantiate(settings, host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(
            plugin.lease.timeout,
            Some(Duration::from_secs(30)),
            "the next instance has the settings' timeout"
        );
    }

    /// `count` instances of `runtime` at the same time
    fn burst(runtime: &PluginRuntime, count: usize) -> Vec<PluginInstance> {
        (0..count)
            .map(|_| runtime.instantiate(Manifest::default(), host("2026-10-05T00:00:00Z")).unwrap())
            .collect()
    }

    /// `router_now.wat` as bytes of their own, with the comment `tag`: compiled code no other test shares
    fn module(tag: &str) -> Vec<u8> {
        [
            include_bytes!("../../tests/plugins/router_now.wat").as_slice(),
            format!("\n;; {tag}\n").as_bytes(),
        ]
        .concat()
    }

    #[test]
    fn a_module_keeps_one_compiled_plugin_idle_whatever_the_burst() {
        let runtime = PluginRuntime::new(module("one idle"));
        let instances = burst(&runtime, 3);
        assert_eq!(idle(&runtime), 0, "every live instance leases its own compiled plugin");
        assert_eq!(runtime.compilations(), 3);
        drop(instances);
        assert_eq!(idle(&runtime), 1, "a burst leaves one idle");
        // which the calls after it, one at a time, reuse
        for _ in 0..3 {
            drop(burst(&runtime, 1));
            assert_eq!(idle(&runtime), 1);
        }
        assert_eq!(runtime.compilations(), 3, "and compile nothing");
    }

    #[test]
    fn the_modules_keeping_spares_keep_at_most_their_limit_over_all_of_them() {
        let spares: &'static Spares = Box::leak(Box::new(Spares::new(3)));
        let (one, two) = (PluginRuntime::new(module("spares one")), PluginRuntime::new(module("spares two")));
        one.code.keep_spares(spares);
        two.code.keep_spares(spares);
        drop(burst(&one, 6));
        assert_eq!((idle(&one), spares.kept()), (4, 3), "its first idle copy and every spare");
        drop(burst(&two, 2));
        assert_eq!(idle(&two), 1, "the spares are all kept: its first idle copy only");

        // a spare leased is a spare no more, and its place may go to another module
        let leased = burst(&one, 1);
        assert_eq!((idle(&one), spares.kept()), (3, 2));
        drop(burst(&two, 2));
        assert_eq!((idle(&two), spares.kept()), (2, 3));
        drop(leased);
        assert_eq!(idle(&one), 3, "returned past the limit: dropped");
        assert_eq!(spares.kept(), 3);

        // the code of a module gone, its spares go with it
        drop(one);
        assert_eq!(spares.kept(), 1);
        drop(two);
        assert_eq!(spares.kept(), 0);
    }

    #[test]
    fn a_call_running_to_its_timeout_drops_its_compiled_code() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/router_loop.wat").to_vec());
        let manifest = Manifest::default().with_timeout(Duration::from_millis(200));
        let mut plugin = runtime.instantiate(manifest, host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(plugin.call::<_, String>("router", "{}").unwrap_err().to_string(), "timeout");
        // a quick call of the same instance afterwards does not make its engine safe again
        assert_eq!(plugin.call::<_, String>("name", ()).unwrap(), "\"router-loop\"");
        drop(plugin);
        assert_eq!(idle(&runtime), 0, "an engine whose epoch may still be bumped is not reused");
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
        assert!(runtime.code.bindings.lock().is_empty(), "dropped instances must not accumulate bindings");
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

    /// an instance of `runtime` with the timeout `timeout`, built on a thread of its own and its `name` called: what
    /// that came to and how long it took, or `None` when it had not ended after 10 s
    fn built_and_called(runtime: Arc<PluginRuntime>, timeout: Duration) -> Option<(Result<String, String>, Duration)> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let outcome = runtime
                .instantiate(Manifest::default().with_timeout(timeout), host("2026-10-05T00:00:00Z"))
                .map_err(|error| format!("instantiate: {error:#}"))
                .and_then(|mut plugin| plugin.call::<&str, String>("name", "").map_err(|error| format!("call: {error:#}")));
            let _ = sender.send((outcome, started.elapsed()));
        });
        receiver.recv_timeout(Duration::from_secs(10)).ok()
    }

    /// A start function or a reactor's `_initialize` runs while the instance is built, before any call: one that never
    /// ends is stopped at the instance's timeout, and fails it saying so (it ran with no timer, and hung its host)
    #[test]
    fn an_initialisation_that_never_ends_fails_the_instance_at_its_timeout() {
        let fixtures = [
            ("start_loop.wat", include_bytes!("../../tests/plugins/start_loop.wat").as_slice()),
            ("initialize_loop.wat", include_bytes!("../../tests/plugins/initialize_loop.wat").as_slice()),
        ];
        for (fixture, bytes) in fixtures {
            let runtime = Arc::new(PluginRuntime::new(bytes.to_vec()));
            let (outcome, took) = built_and_called(runtime.clone(), Duration::from_millis(200)).unwrap_or_else(|| panic!("{fixture} still runs after 10 s"));
            assert_eq!(
                outcome,
                Err("instantiate: its start function or initialisation (`_initialize`) ran longer than its timeout of 200ms".to_owned()),
                "{fixture}"
            );
            assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(5), "{fixture}: {took:?}");
            assert_eq!(runtime.idle(), 0, "{fixture}: compiled code whose epoch was bumped is not reused");
        }
    }

    /// extism runs a WASI module's constructors (`__wasm_call_ctors`) when the first call instantiates the module,
    /// before the export: within the call's timeout
    #[test]
    fn constructors_that_never_end_fail_the_first_call_at_its_timeout() {
        let runtime = Arc::new(PluginRuntime::new(include_bytes!("../../tests/plugins/ctors_loop.wat").to_vec()));
        let (outcome, took) = built_and_called(runtime.clone(), Duration::from_millis(200)).expect("the call still runs after 10 s");
        assert_eq!(outcome, Err("call: timeout".to_owned()));
        assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(5), "{took:?}");
        assert_eq!(runtime.idle(), 0);
    }

    /// an instance built near its timeout does not return its compiled plugin, as a call ending near it does not: the
    /// timer's bump may still come
    #[test]
    fn an_instance_built_near_its_timeout_does_not_return_its_compiled_plugin() {
        let runtime = PluginRuntime::new(include_bytes!("../../tests/plugins/router_now.wat").to_vec());
        // the least margin is 100 ms: an instance with 50 ms is always built near its timeout
        drop(
            runtime
                .instantiate(Manifest::default().with_timeout(Duration::from_millis(50)), host("2026-10-05T00:00:00Z"))
                .unwrap(),
        );
        assert_eq!(runtime.idle(), 0);
        drop(
            runtime
                .instantiate(Manifest::default().with_timeout(Duration::from_secs(30)), host("2026-10-05T00:00:00Z"))
                .unwrap(),
        );
        assert_eq!(runtime.idle(), 1);
    }

    #[test]
    fn a_start_function_calling_a_host_function_fails_cleanly() {
        // a start function runs while `Plugin::new_from_compiled_with_manifest` builds the instance, before its
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
        assert!(runtime.code.bindings.lock().is_empty(), "a failed instantiation leaves no binding");
    }
}
