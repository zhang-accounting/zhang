//! A hook into the allocator of the process, which zhang-core does not choose: the binary does (the `zhang` binary
//! uses mimalloc, `zhang-cli/src/allocator.rs`), and only the binary can ask it for what a global allocator does not
//! offer through [`std::alloc::GlobalAlloc`].
//!
//! The one request is [`return_freed`]: give back to the system the free memory the allocator keeps after a large
//! allocation that is gone. mimalloc keeps the pages a freed buffer used, to serve the next allocations from them, and
//! returns them only later, if at all; a burst of large, short-lived buffers that the process does not repeat soon
//! leaves its resident memory higher for as long as nothing reuses them. Compiling a plugin is such a burst, done once
//! per plugin and settings: reading its compiled code back from the disk cache (or compiling it, when the cache has
//! none) allocates buffers that are freed once the code is mapped, about 10 MiB for the Lua interpreter every Lua
//! plugin runs on, which a server serving a ledger of 10,000 transactions otherwise kept through every reload.
//!
//! The binary registers the hook once, at startup ([`set_return_freed`]); without one (a library user, a test, the
//! system's allocator) [`return_freed`] does nothing. It is meant for rare moments, never for a path every request or
//! every load takes: returning memory costs a pass over the allocator's free pages, and pages returned are faulted in
//! again by the next allocations that need them (an allocator told to return its free pages at once loaded ledgers 20
//! to 30% slower).

use std::sync::OnceLock;

/// how the binary gives the allocator's free memory back to the system, if it said ([`set_return_freed`])
static RETURN_FREED: OnceLock<fn()> = OnceLock::new();

/// Set how to give the free memory the allocator keeps back to the system: `hook` runs on every [`return_freed`].
/// The binary calls it once, at startup, before anything allocates much; a later call is ignored, and answers `false`.
pub fn set_return_freed(hook: fn()) -> bool {
    RETURN_FREED.set(hook).is_ok()
}

/// Give the free memory the allocator keeps back to the system, with the hook the binary set ([`set_return_freed`]);
/// nothing when it set none. On the calling thread, right after the large buffers it allocated were freed: see the
/// module's documentation for when.
pub fn return_freed() {
    if let Some(hook) = RETURN_FREED.get() {
        hook();
    }
}
