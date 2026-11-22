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

/// Give the free memory the allocator keeps back to the system, on the calling thread: with mimalloc, `mi_collect(true)`
/// (the thread's free pages, and the free memory of every arena purged); nothing with the system's allocator. Set as
/// zhang-core's hook at startup ([`install`]), which calls it at rare moments only, after a plugin is compiled
/// (`zhang_core::memory`).
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
    /// of every arena, that is, return it to the system. `libmimalloc-sys` declares it under its `extended` feature,
    /// which would add a dependency for this one declaration
    fn mi_collect(force: bool);
}
