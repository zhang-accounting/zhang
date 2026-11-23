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
//!
//! Compiling goes through the disk cache of [`crate::plugin::disk_cache`]: a module compiled before, by this process or
//! an earlier one, is read back instead of compiled again. The cache keeps code by the module's bytes and wasmtime's
//! settings, which the settings of a [`Manifest`] do not change, so one entry serves every config of a module.
//!
//! A compilation runs on one thread of its own, which gives the memory the compilation took back before it ends (when
//! it compiled, rather than read the code back from the disk cache). wasmtime compiles a module's functions in parallel
//! by default, on rayon's global pool: a thread per CPU the process may use, each taking about 10 MiB for the Lua
//! interpreter every Lua plugin runs on, which the allocator kept after the compilation (mimalloc keeps what a thread
//! freed for that thread, and rayon's threads stay, idle). A cold start with one Lua plugin took 192 MiB on 4 CPUs, 270
//! on 8 and 906 on 64, and kept it; on a host of 96 CPUs, containers of 384 MiB to 1 GiB were killed at start. Compiled
//! on one thread, the peak is the same on any machine (103 to 111 MiB with the plugin, 49 without), and the memory goes
//! back once the compilation is over (57 to 63 MiB). It takes longer, once per module and disk cache: 0.4 s for the Lua
//! interpreter, where 64 threads took 0.06 s, 8 took 0.08 s and 4 took 0.12 s; reading the code back from the disk
//! cache takes 3 ms either way. See [`compile_config`] and [`crate::memory`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use extism::{Cache, CompiledPlugin, CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, ValType, Wasm, EXTISM_USER_MODULE};
use uuid::Uuid;

use crate::plugin::disk_cache;
use crate::plugin::host::PluginHost;

/// the name of the thread a compilation runs on ([`PluginRuntime::compile`])
const COMPILE_THREAD: &str = "zhang-compile";

/// The wasmtime settings every plugin compiles with, under the ones extism sets: one thread compiles all of a module's
/// functions, the one that asked (see the module's documentation for why). wasmtime's cache keeps code by settings that
/// do not include this one, so code an earlier version compiled in parallel is read back as before.
fn compile_config() -> wasmtime::Config {
    let mut config = wasmtime::Config::new();
    config.parallel_compilation(false);
    config
}

/// the most compiled plugins a runtime keeps idle for its current settings. Concurrent calls each
/// lease their own, so a burst past this count compiles afresh and drops the extra when it ends:
/// a plugin never holds more than this many engines once the burst is over
const IDLE_LIMIT: usize = 4;

/// the least margin before its timeout a call must end with to return its compiled plugin to the
/// pool; see [`near_timeout`]
const NEAR_TIMEOUT_FLOOR: Duration = Duration::from_millis(100);

/// how close to `timeout` a call may end and still return its compiled plugin to the pool: a tenth
/// of the timeout, and at least [`NEAR_TIMEOUT_FLOOR`]. extism's timer bumps the engine's epoch at
/// the deadline from its own thread, so a call ending within a moment of its deadline can be
/// followed by a late bump that would stop the next instance on that engine. Such a call's compiled
/// plugin is dropped instead; the next call compiles afresh
fn near_timeout(timeout: Duration) -> Duration {
    (timeout / 10).max(NEAR_TIMEOUT_FLOOR)
}

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
    /// how many times the module was compiled
    compilations: AtomicUsize,
    /// the disk cache compiling reads and writes, [`disk_cache::shared`]; none compiles without one
    disk_cache: Option<Cache>,
    /// what the plugin declared when it registered, once a default plugin's registration read it
    registration: std::sync::OnceLock<super::store::Registration>,
}

impl PluginRuntime {
    /// the runtime of the module `module_bytes`, compiled on first use
    pub fn new(module_bytes: Vec<u8>) -> Self {
        Self {
            module_bytes,
            bindings: InstanceBindings::default(),
            pool: Arc::default(),
            compilations: AtomicUsize::new(0),
            disk_cache: disk_cache::shared().cloned(),
            registration: std::sync::OnceLock::new(),
        }
    }

    /// the runtime compiling with the disk cache `cache` instead of the shared one, or with none
    #[cfg(test)]
    pub(crate) fn with_disk_cache(mut self, cache: Option<Cache>) -> Self {
        self.disk_cache = cache;
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
        &self.module_bytes
    }

    /// the module compiled with the settings of `manifest`, with the host functions linked in: on a thread of its own
    /// ([`COMPILE_THREAD`]), which compiles alone ([`compile_config`]) and, when it compiled rather than read the code
    /// back from the disk cache, gives back what the compilation freed before it ends ([`crate::memory::return_freed`]),
    /// so that the memory of the thread that asked, a ledger's load or a request, is left as it was
    fn compile(&self, manifest: Manifest) -> Result<CompiledPlugin, extism::Error> {
        self.compilations.fetch_add(1, Ordering::Relaxed);
        let builder = PluginBuilder::new(manifest.with_wasm(Wasm::data(self.module_bytes.clone())))
            .with_functions(PluginHost::functions_for_instances(&self.bindings))
            .with_wasi(true)
            .with_wasmtime_config(compile_config());
        // never extism's own cache, which `EXTISM_CACHE_CONFIG` or wasmtime's configuration file set up
        let builder = match &self.disk_cache {
            Some(cache) => builder.with_cache(cache.clone()),
            None => builder.with_cache_disabled(),
        };
        let cache = self.disk_cache.clone();
        std::thread::scope(|scope| {
            let compiling = std::thread::Builder::new().name(COMPILE_THREAD.to_owned()).spawn_scoped(scope, move || {
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
            })?;
            compiling.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    /// how many times the module was compiled, for any settings
    pub fn compilations(&self) -> usize {
        self.compilations.load(Ordering::Relaxed)
    }

    /// how many compiled plugins are idle, and the pool's generation, which moves only when the settings change
    #[cfg(test)]
    pub(crate) fn pool_state(&self) -> (usize, u64) {
        let pool = lock(&self.pool);
        (pool.idle.len(), pool.generation)
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
                in_call: false,
                spent: false,
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
    /// taken when the lease drops
    compiled: Option<CompiledPlugin>,
    /// set while a call runs and cleared once it returned, so a call that unwinds leaves it set
    in_call: bool,
    /// set once a call of this instance ended near its timeout; it stays set
    spent: bool,
    generation: u64,
    timeout: Option<Duration>,
    pool: Arc<Mutex<Pool>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(compiled) = self.compiled.take() else { return };
        // the engine's epoch may still be bumped by a call's timer: not reused
        if self.in_call || self.spent {
            return;
        }
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

    /// let each later call of this instance run at most `at_most`, when that is less than the timeout of its settings:
    /// a shorter timeout for one instance, without compiling the module for other settings
    pub fn limit_timeout(&mut self, at_most: Duration) {
        if self.lease.timeout.is_none_or(|timeout| at_most < timeout) {
            self.plugin.set_timeout(Some(at_most));
            self.lease.timeout = Some(at_most);
        }
    }

    /// call the export `name` with `input`
    pub fn call<'a, 'b, T: extism::ToBytes<'a>, U: extism::FromBytes<'b>>(&'b mut self, name: impl AsRef<str>, input: T) -> Result<U, extism::Error> {
        // the lease is spent unless the call returns in time: a call that panics (in a host function
        // or in wasmtime) unwinds through extism before it stops the call's timer, so the engine's
        // epoch is still bumped at the deadline, which would stop the next instance on that engine
        self.lease.in_call = true;
        let started = Instant::now();
        let result = self.plugin.call(name, input);
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
        runtime.pool_state()
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
        assert_eq!(pool_state(&runtime), (1, 1));
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
        // the settings are the same, so the compiled plugins are; the one stopped at its deadline is not returned
        assert_eq!(pool_state(&runtime), (0, 1));
        let plugin = runtime.instantiate(settings, host("2026-10-05T00:00:00Z")).unwrap();
        assert_eq!(
            plugin.lease.timeout,
            Some(Duration::from_secs(30)),
            "the next instance has the settings' timeout"
        );
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
        // a quick call of the same instance afterwards does not make its engine safe again
        assert_eq!(plugin.call::<_, String>("name", ()).unwrap(), "\"router-loop\"");
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
