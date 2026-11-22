//! A hook into the allocator of the process, which zhang-core does not choose: the binary does (the `zhang` binary
//! uses mimalloc, `zhang-cli/src/allocator.rs`), and only the binary can ask it for what a global allocator does not
//! offer through [`std::alloc::GlobalAlloc`].
//!
//! The one request is [`return_freed`]: give up the free memory the calling thread's heap keeps after a burst of large
//! allocations that are gone. mimalloc keeps the pages a thread freed in that thread's heap, to serve its next
//! allocations from them; a burst of large, short-lived buffers the thread does not repeat leaves them there, resident,
//! for as long as that thread allocates nothing like them again. Handed back, they go to mimalloc's shared pool, where
//! any thread's allocations reuse them, and which returns what stays unused to the system after its purge delay.
//! Compiling a plugin is such a burst, done once per plugin and settings: reading its compiled code back from the disk
//! cache (or compiling it, when the cache has none) allocates buffers that are freed once the code is mapped, about 10
//! MiB for the Lua interpreter every Lua plugin runs on.
//!
//! The binary registers the hook once, at startup ([`set_return_freed`]); without one (a library user, a test, the
//! system's allocator) [`return_freed`] does nothing. It is meant for rare moments, never for a path every request or
//! every load takes: collecting costs a pass over the thread's pages, and memory returned to the system is faulted in
//! again by the next allocations that need it (an allocator told to return its free pages at once loaded ledgers 20 to
//! 30% slower).

use std::sync::OnceLock;

/// how the binary gives the allocator's free memory back to the system, if it said ([`set_return_freed`])
static RETURN_FREED: OnceLock<fn()> = OnceLock::new();

/// Set how to give up the free memory a thread's heap keeps: `hook` runs on every [`return_freed`].
/// The binary calls it once, at startup, before anything allocates much; a later call is ignored, and answers `false`.
pub fn set_return_freed(hook: fn()) -> bool {
    RETURN_FREED.set(hook).is_ok()
}

/// Give up the free memory the calling thread's heap keeps, with the hook the binary set ([`set_return_freed`]); nothing
/// when it set none. Right after the large buffers the thread allocated were freed: see the module's documentation for
/// when.
pub fn return_freed() {
    if let Some(hook) = RETURN_FREED.get() {
        hook();
    }
}
