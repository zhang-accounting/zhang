//! The global allocator of the `zhang` binary: mimalloc, unless it is built without the default feature `mimalloc`,
//! which keeps the system's (glibc on Linux).
//!
//! The server frees the ledger a reload replaced on a blocking thread while the requests on the new one run
//! (`zhang_server::ReloadSender::retire`). glibc's allocator handles that badly: the first request after a reload
//! is two to three times slower, allocation-heavy queries stay slower, and the memory freed stays with the process,
//! up to 5 GiB after the budget baseline's run with 100,000 transactions. mimalloc answers the first request as fast
//! as a fresh process does, loads faster, and gives most of the memory back. The evaluation of glibc (with and
//! without `malloc_trim`), mimalloc and jemalloc is on the developer page "Budget Baseline".

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The allocator the binary was built with, for the logs and the budget baseline.
pub const NAME: &str = if cfg!(feature = "mimalloc") { "mimalloc" } else { "system" };

/// Give the free memory mimalloc keeps back to the system, at once: `mi_collect(true)`, which collects the calling
/// thread's heap and purges the free memory of every arena and of the heaps of threads that ended; nothing with the
/// system's allocator. Set as zhang-core's hook at startup ([`install`]), which calls it on the thread a plugin compiles
/// on, right before that thread ends, when it compiled rather than read the code back from the disk cache
/// (`zhang_core::memory`).
///
/// Forced, as the thread ends right after: without force mimalloc returns what it freed only after its purge delay, at
/// a later allocation that checks, and a cold start with one Lua plugin kept 108 MiB resident instead of 61. Forced
/// more often, it keeps more: after every compilation, reading the code back too, as each load of a ledger does for its
/// plugins, 230 MiB more stayed resident after a reload of 100,000 transactions; on the thread loading a ledger of
/// 100,000 transactions, the load ended 142 MiB higher and the peak of the reload after it 42 MiB higher (#878).
pub fn return_freed() {
    #[cfg(feature = "mimalloc")]
    // SAFETY: a function of mimalloc's C API, without arguments to check, on the allocator the binary links and uses
    unsafe {
        mi_collect(true)
    }
}

/// Register [`return_freed`] as zhang-core's way to give the allocator's free memory back (`zhang_core::memory`): at
/// startup, before the first load.
pub fn install() {
    zhang_core::memory::set_return_freed(return_freed);
}

#[cfg(feature = "mimalloc")]
extern "C" {
    /// `void mi_collect(bool force)` of mimalloc's C API (`mimalloc.h`), which the `mimalloc` crate links through
    /// `libmimalloc-sys` and does not expose: collect the calling thread's heap, and with `force` purge the free memory
    /// of every arena too, returning it to the system. `libmimalloc-sys` declares it under its `extended` feature, which
    /// would add a dependency for this one declaration
    fn mi_collect(force: bool);
}
