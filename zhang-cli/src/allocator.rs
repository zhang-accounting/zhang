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
