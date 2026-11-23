//! A hook into the allocator of the process, which zhang-core does not choose: the binary does (the `zhang` binary
//! uses mimalloc, `zhang-cli/src/allocator.rs`), and only the binary can ask it for what a global allocator does not
//! offer through [`std::alloc::GlobalAlloc`].
//!
//! The one request is [`return_freed`]: give the free memory the allocator keeps back to the system, at once. Compiling
//! a plugin is a burst of allocations that are all freed but the compiled code: about 50 MiB for the Lua interpreter
//! every Lua plugin runs on. mimalloc keeps the pages a thread freed in that thread's heap, to serve its next allocations from them, and what it hands back to its shared
//! pool it returns to the system only after a delay, at a later allocation that checks: in a server left idle after
//! it started, never. So a compilation runs on a thread of its own, which calls [`return_freed`] before it ends when it
//! compiled rather than read the code back (`plugin::runtime`): what the compilation took goes back to the system, and
//! the heaps of the threads that load ledgers and answer requests are left as they were.
//!
//! The binary registers the hook once, at startup ([`set_return_freed`]); without one (a library user, a test, the
//! system's allocator) [`return_freed`] does nothing. It is meant for a thread about to end after such a burst, never
//! for a path every request or every load takes (a load reads its plugins' code back from the disk cache): memory
//! returned to the system is faulted in again by the next allocations that need it (an allocator told to return its
//! free pages at once loaded ledgers 20 to 30% slower), and returned more often, or on a thread that goes on
//! allocating, more stayed resident, not less (`zhang-cli/src/allocator.rs`).

use std::sync::OnceLock;

/// how the binary gives the allocator's free memory back to the system, if it said ([`set_return_freed`])
static RETURN_FREED: OnceLock<fn()> = OnceLock::new();

/// Set how to give the free memory the allocator keeps back to the system: `hook` runs on every [`return_freed`].
/// The binary calls it once, at startup, before anything allocates much; a later call is ignored, and answers `false`.
pub fn set_return_freed(hook: fn()) -> bool {
    RETURN_FREED.set(hook).is_ok()
}

/// Give the free memory the allocator keeps back to the system, with the hook the binary set ([`set_return_freed`]);
/// nothing when it set none. On a thread about to end, right after the large buffers it allocated were freed: see the
/// module's documentation for when.
pub fn return_freed() {
    if let Some(hook) = RETURN_FREED.get() {
        hook();
    }
}
