use crate::{Error, FromBytesOwned, Plugin, ToBytes};

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, RwLock, Weak},
};

/// `PoolBuilder` is used to configure and create `Pool`s
#[derive(Debug, Clone)]
pub struct PoolBuilder {
    /// Max number of concurrent instances for a plugin - by default this is set to the output of
    /// `std::thread::available_parallelism`
    pub max_instances: usize,
}

impl PoolBuilder {
    /// Create a `PoolBuilder` with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the max number of parallel instances
    pub fn with_max_instances(mut self, n: usize) -> Self {
        self.max_instances = n;
        self
    }

    /// Create a new `Pool` with the given configuration
    pub fn build<F: 'static + Fn() -> Result<Plugin, Error> + Send + Sync>(
        self,
        source: F,
    ) -> Pool {
        Pool::new_from_builder(source, self)
    }
}

impl Default for PoolBuilder {
    fn default() -> Self {
        PoolBuilder {
            max_instances: std::thread::available_parallelism()
                .expect("available parallelism")
                .into(),
        }
    }
}

type PluginSource = dyn Fn() -> Result<Plugin, Error> + Send + Sync;

struct PoolInner {
    plugin_source: Box<PluginSource>,
    /// Available plugins ready to be checked out
    available: VecDeque<Plugin>,
    /// Current number of plugins (checked out + available)
    current_size: usize,
    /// Maximum number of plugins
    max_size: usize,
}

/// `Pool` manages threadsafe access to a limited number of instances of multiple plugins
#[derive(Clone)]
pub struct Pool {
    inner: Arc<Mutex<PoolInner>>,
    cond: Arc<Condvar>,
    existing_functions: Arc<RwLock<HashMap<String, bool>>>,
}

impl Pool {
    /// Create a new pool with the default configuration
    pub fn new<F: 'static + Fn() -> Result<Plugin, Error> + Send + Sync>(source: F) -> Self {
        Self::new_from_builder(source, PoolBuilder::default())
    }

    /// Create a new pool configured using a `PoolBuilder`
    pub fn new_from_builder<F: 'static + Fn() -> Result<Plugin, Error> + Send + Sync>(
        source: F,
        builder: PoolBuilder,
    ) -> Self {
        let cond = Arc::new(Condvar::new());
        Pool {
            inner: Arc::new(Mutex::new(PoolInner {
                plugin_source: Box::new(source),
                available: VecDeque::new(),
                current_size: 0,
                max_size: builder.max_instances,
            })),
            cond,
            existing_functions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of live instances for a plugin (both checked out and available)
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().current_size
    }

    /// Get access to a plugin, this will create a new instance if needed (and allowed by the specified
    /// max_instances). `Ok(None)` is returned if the timeout is reached before an available plugin could be
    /// acquired
    pub fn get(&self, timeout: std::time::Duration) -> Result<Option<PoolPlugin>, Error> {
        let start = std::time::Instant::now();

        // Hold lock throughout except when waiting on condition variable
        let mut inner = self.inner.lock().unwrap();

        loop {
            // Try to pop an available plugin from the queue
            if let Some(plugin) = inner.available.pop_front() {
                return Ok(Some(PoolPlugin {
                    plugin: Some(plugin),
                    pool: Arc::downgrade(&self.inner),
                    cond: self.cond.clone(),
                }));
            }

            // Create new plugin if under capacity
            if inner.current_size < inner.max_size {
                let plugin = (*inner.plugin_source)()?;
                inner.current_size += 1;
                return Ok(Some(PoolPlugin {
                    plugin: Some(plugin),
                    pool: Arc::downgrade(&self.inner),
                    cond: self.cond.clone(),
                }));
            }

            // All plugins busy and at capacity. Check if we should keep waiting.
            let elapsed = std::time::Instant::now() - start;
            if elapsed >= timeout {
                return Ok(None);
            }

            // Wait for a plugin to be returned. wait_timeout releases the lock while
            // waiting and re-acquires it when woken. Loop back to check availability.
            let remaining = timeout - elapsed;
            let (guard, wait_result) = self.cond.wait_timeout(inner, remaining).unwrap();
            inner = guard;

            if wait_result.timed_out() {
                return Ok(None);
            }
        }
    }

    /// Access a plugin in a callback function. This calls `Pool::get` then the provided callback. `Ok(None)`
    /// is returned if the timeout is reached before an available plugin could be acquired
    pub fn with_plugin<T>(
        &self,
        timeout: std::time::Duration,
        f: impl FnOnce(&mut Plugin) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        if let Some(mut plugin) = self.get(timeout)? {
            return f(&mut plugin).map(Some);
        }
        Ok(None)
    }

    /// Returns `true` if the given function exists, otherwise `false`. Results are cached after the first
    /// call.
    pub fn function_exists(&self, name: &str, timeout: std::time::Duration) -> Result<bool, Error> {
        // read current value if any
        let read = self.existing_functions.read().unwrap();
        let exists_opt = read.get(name).cloned();
        drop(read);

        if let Some(exists) = exists_opt {
            Ok(exists)
        } else {
            // load plugin and call function_exists
            let plugin = self.get(timeout)?;
            if let Some(p) = plugin.as_ref() {
                let exists = p.plugin.as_ref().unwrap().function_exists(name);

                // write result to hashmap
                let mut write = self.existing_functions.write().unwrap();
                write.insert(name.to_string(), exists);

                Ok(exists)
            } else {
                // Timeout - return false but don't cache, since we don't
                // actually know whether the function exists
                Ok(false)
            }
        }
    }
}

/// `PoolPlugin` wraps a plugin checked out from a pool. When dropped, the plugin is automatically returned
/// to the pool.
pub struct PoolPlugin {
    /// The checked-out plugin. Wrapped in `Option` so it can be moved out on drop.
    plugin: Option<Plugin>,
    /// Weak reference to the pool, used to return the plugin on drop. Using `Weak` allows the pool
    /// to be fully dropped even if plugins are still checked out; when those plugins are dropped,
    /// they'll see the pool is gone and simply drop themselves.
    pool: Weak<Mutex<PoolInner>>,
    /// Condition variable to notify waiters when this plugin is returned.
    cond: Arc<Condvar>,
}

impl std::fmt::Debug for PoolPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolPlugin")
            .field("plugin", &self.plugin.as_ref().map(|p| p.id))
            .finish()
    }
}

impl PoolPlugin {
    /// Helper to call a plugin function on the underlying plugin
    pub fn call<'a, Input: ToBytes<'a>, Output: FromBytesOwned>(
        &mut self,
        name: impl AsRef<str>,
        input: Input,
    ) -> Result<Output, Error> {
        self.plugin
            .as_mut()
            .expect("plugin is Some until Drop runs")
            .call(name.as_ref(), input)
    }

    /// Helper to get the underlying plugin's ID
    pub fn id(&self) -> uuid::Uuid {
        self.plugin
            .as_ref()
            .expect("plugin is Some until Drop runs")
            .id
    }
}

impl std::ops::Deref for PoolPlugin {
    type Target = Plugin;

    fn deref(&self) -> &Self::Target {
        self.plugin
            .as_ref()
            .expect("plugin is Some until Drop runs")
    }
}

impl std::ops::DerefMut for PoolPlugin {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.plugin
            .as_mut()
            .expect("plugin is Some until Drop runs")
    }
}

impl Drop for PoolPlugin {
    fn drop(&mut self) {
        if let Some(plugin) = self.plugin.take() {
            if let Some(inner) = self.pool.upgrade() {
                let mut guard = inner.lock().unwrap();
                guard.available.push_back(plugin);
                drop(guard);
                self.cond.notify_one();
            }
            // If pool is gone, just drop the plugin
        }
    }
}
