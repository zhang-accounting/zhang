//! Host functions zhang offers WASM plugins.
//!
//! They live in extism's `extism:host/user` namespace. A plugin opts into one by importing it: a plugin
//! that imports none of them never notices them, and a plugin that imports one fails to load on a
//! zhang that predates it, with an "unknown import" error.
//!
//! - `zhang_emit_error(payload: i64)`, no result: report a problem without failing the load.
//!   `payload` is the offset of a kernel memory block holding the JSON
//!   `{"message": "...", "span": {...}?, "metas": {"key": "value"}?}`, where `span` is the span of a
//!   directive the plugin received. The problem becomes a [`ErrorKind::PluginError`] in the ledger's
//!   error list, on that span or else on the plugin's directive, with the metas `plugin` (the plugin's
//!   name), `message` and the plugin's own `metas`. A payload that cannot be read is reported the same
//!   way, with a message saying it is invalid. While a router plugin handles a request there is no
//!   error list, so the problem is logged as a warning instead (see [`crate::plugin::router`]).
//! - `zhang_now() -> i64`: the current time of the load (see [`NOW`]).
//! - `zhang_read_file(path: i64) -> i64` and `zhang_list_dir(path: i64) -> i64`: read a file or list a
//!   directory the plugin's `allowed_paths` grant, read-only (see [`crate::plugin::files`] for the rules).
//!   `path` is the offset of a kernel memory block holding the path as UTF-8 text, relative to the ledger
//!   root and written with `/` (`"."` is the root). The result is the offset of a block holding JSON:
//!   `{"Ok": {"content": "...", "encoding": "utf8" | "base64"}}` for a file (base64 when it is not valid
//!   UTF-8 or is mostly control characters), `{"Ok": {"entries": [{"name": "...", "kind": "file" |
//!   "dir"}]}}` for a directory (sorted by name), or `{"Err": {"kind": "denied" | "not_found" |
//!   "too_large" | "unsupported" | "invalid", "message": "..."}}`. They never trap: a path block that is
//!   not readable UTF-8 of at most [`FILE_PATH_MAX_LEN`] bytes gives `invalid`. Only a processor or
//!   mapper reads files: a plugin without `allowed_paths` gets `denied` for every path, and so does any
//!   plugin while it registers (`name`, `version`, `supported_type`) or handles a request as a router.
//!
//! Every plugin instance gets its own [`PluginHost`]. It keeps what the host functions collect while
//! the instance runs, until the stage running the plugin hands it to the pipeline.

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, SecondsFormat};
use chrono_tz::Tz;
use extism::convert::MemoryHandle;
use extism::{CurrentPlugin, Function, UserData, Val, ValType, EXTISM_USER_MODULE, PTR};
use log::warn;
use serde::{Deserialize, Serialize};
use zhang_ast::error::ErrorKind;
use zhang_ast::SpanInfo;
pub use zhang_shared::plugin_abi::import::{EMIT_ERROR, LIST_DIR, NOW, READ_FILE};
// `zhang_now` cannot fail, so it always answers `HostOk::Ok`
use zhang_shared::plugin_abi::{HostResult as HostOk, LedgerInfo, Now as NowPayload};

use crate::clock::LoadClock;
use crate::inputs::ExtraInput;
use crate::pipeline::{StageContext, StageError};
use crate::plugin::files::{FileAccess, FileCall, FileError, FileErrorKind};
use crate::plugin::router::{self, RouterHost, LEDGER_INFO_FUNCTION, QUERY_FUNCTION};

/// meta of a [`ErrorKind::PluginError`] holding the name of the plugin that reported it
pub const PLUGIN_META: &str = "plugin";
/// meta of a [`ErrorKind::PluginError`] holding the problem the plugin described
pub const MESSAGE_META: &str = "message";

/// the most bytes [`read_input`] will copy out of a plugin for a `zhang_emit_error` payload. The
/// payload is a small JSON object (a message, an optional span and a few metas), so one mebibyte is
/// far more than a real caller needs while still being a hard ceiling against a runaway allocation.
const EMIT_ERROR_MAX_LEN: usize = 1024 * 1024;

// --- reading plugin memory safely -------------------------------------------------------------
//
// A host function receives the offset of a block in the plugin's linear ("kernel") memory and has to
// read the bytes there. extism 1.8 reads them with `CurrentPlugin::memory_from_val` followed by
// `memory_bytes`/`memory_str`. `memory_from_val` trusts the length the kernel's `length` export
// reports, which is the block header's `used` field, and `memory_bytes` builds the slice with
// `std::slice::from_raw_parts` without any bounds check (this is still true in extism 1.30.0, so a
// version bump does not fix it upstream). A plugin can overwrite its own block header -- e.g.
// `store_u8($p - 1, 0x7f)` sets `used` to about 2 GiB -- so the host then slices far past the mapped
// memory and reading it kills the whole process with SIGBUS. A smaller forge leaks the plugin's own
// kernel memory instead of crashing. These functions never trap, so that is a host crash the plugin
// controls.
//
// extism 1.8 exposes no accessor for the size of the kernel's linear memory (`CurrentPlugin::memory`
// and the wasmtime store are private). The kernel does record it: its `MemoryRoot` struct, at a
// fixed address in linear memory, has a `length` field holding the current size of the data region.
// We read that one field directly through the only public primitive we have, `memory_bytes`, and use
// it to bound every read. The constants below pin the `MemoryRoot` layout of the extism-runtime
// kernel 0.1 that ships inside extism 1.8 (see the kernel's `#[repr(C)] struct MemoryRoot`).

/// linear-memory address of the kernel's `MemoryRoot` (the kernel hard-codes it at offset 1)
const KERNEL_ROOT_ADDR: u64 = 1;
/// size of `MemoryRoot`: an `AtomicBool` padded to 8, then seven `u64`s = 64 bytes
const KERNEL_ROOT_SIZE: u64 = 64;
/// byte offset of `MemoryRoot::length` within the struct: past `initialized` (padded to 8) and
/// `position` (`u64`), i.e. the second `u64` field
const KERNEL_LENGTH_FIELD: u64 = 16;
/// linear-memory address of `MemoryRoot::length`, the current size of the data region in bytes.
///
/// This is the one value a plugin cannot forge. The kernel's `store_u8`/`store_u64` exports guard
/// every write with `pointer_in_bounds_fast`, which rejects any address below `size_of::<MemoryRoot>()`
/// (64); the `MemoryRoot` is pinned at address 1, so this field (addresses 17..25) is below that floor
/// and no store export can reach it (the `router_query_root_forge`/`emit_error_root_forge` fixtures
/// prove a plugin's attempt to overwrite it is a no-op). `length` only ever changes when the kernel
/// allocator grows the memory. The other mutators (`input_set`, `output_set`, `error_set`) write
/// their own dedicated root fields, never `length`.
const KERNEL_LENGTH_ADDR: u64 = KERNEL_ROOT_ADDR + KERNEL_LENGTH_FIELD;
/// first address a data block can occupy: right after the `MemoryRoot`
const KERNEL_DATA_START: u64 = KERNEL_ROOT_ADDR + KERNEL_ROOT_SIZE;
/// the largest plausible data-region size: a wasm32 linear memory tops out at 4 GiB, so a `length`
/// larger than that (less the bytes the `MemoryRoot` occupies) is a garbage read and we deny it
const KERNEL_MAX_DATA_LEN: u64 = (1u64 << 32) - KERNEL_DATA_START;

/// why the bytes a plugin handed a host function could not be read
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InputError {
    /// the argument is not the offset of a live block (offset 0, or a freed/zero-length handle)
    NotABlock,
    /// the block's offset and length do not lie inside the kernel's data region. A plugin can forge
    /// the length its block header reports, so the host never trusts it for a read: reading an
    /// out-of-bounds slice crashes the process
    OutOfBounds,
    /// the block is larger than this host function is willing to read
    TooLarge { len: u64, max: usize },
    /// the bytes are not valid UTF-8 (only from [`read_input_str`])
    NotUtf8(std::str::Utf8Error),
}

impl Display for InputError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            InputError::NotABlock => write!(f, "is not a memory block"),
            InputError::OutOfBounds => write!(f, "points outside the plugin's memory"),
            InputError::TooLarge { len, max } => write!(f, "is {len} bytes, over the {max} byte limit"),
            InputError::NotUtf8(e) => write!(f, "is not UTF-8 text: {e}"),
        }
    }
}

/// the current size in bytes of the kernel's data region, read straight from `MemoryRoot::length`,
/// or `None` if it cannot be read or is implausible. This is the ground truth a plugin cannot forge:
/// a block's own header is writable by the plugin, but `MemoryRoot::length` sits below the kernel's
/// store floor (see [`KERNEL_LENGTH_ADDR`]) and only the allocator changes it.
fn kernel_data_len(plugin: &mut CurrentPlugin) -> Option<u64> {
    // SAFETY: `KERNEL_LENGTH_ADDR` (17) with length 8 is inside the first page, which the kernel
    // always maps before any host function can run, so `memory_bytes` slices live memory. We only
    // read it; the bytes are the little-endian `MemoryRoot::length`.
    let handle = unsafe { MemoryHandle::new(KERNEL_LENGTH_ADDR, 8) };
    let bytes = plugin.memory_bytes(handle).ok()?;
    let len = u64::from_le_bytes(bytes.try_into().ok()?);
    // a sanity guard: a wasm32 memory cannot be this large, so an implausible value means the read
    // (or the assumed layout) is wrong. Deny the input rather than trust it.
    (len <= KERNEL_MAX_DATA_LEN).then_some(len)
}

/// validate that a block at `offset` of `len` bytes is a real, in-bounds read of at most `max_len`
/// bytes. `kernel_data_len` is `MemoryRoot::length`. Pure arithmetic, split out so it can be tested
/// without a running plugin.
fn check_bounds(offset: u64, len: u64, kernel_data_len: u64, max_len: usize) -> Result<(), InputError> {
    // reject an oversized length before anything reads it, so even a forged ~2 GiB length can never
    // reach `memory_bytes` and crash the process
    if len > max_len as u64 {
        return Err(InputError::TooLarge { len, max: max_len });
    }
    let end = offset.checked_add(len).ok_or(InputError::OutOfBounds)?;
    let limit = KERNEL_DATA_START.checked_add(kernel_data_len).ok_or(InputError::OutOfBounds)?;
    if offset < KERNEL_DATA_START || end > limit {
        return Err(InputError::OutOfBounds);
    }
    Ok(())
}

/// Read the bytes of the kernel-memory block a plugin passed to a host function in `val`, after
/// validating that the block lies within the kernel's memory and is at most `max_len` bytes.
///
/// This is the one place host functions turn a plugin-supplied offset into bytes. It never trusts
/// the length the plugin's block header reports: it bounds every read against `MemoryRoot::length`,
/// so a forged header cannot make the host read out of bounds (which crashes the process with
/// SIGBUS) or read another plugin allocation. Reusable by any host function that reads plugin
/// memory; [`read_input_str`] is the UTF-8 variant.
pub(crate) fn read_input(plugin: &mut CurrentPlugin, val: &Val, max_len: usize) -> Result<Vec<u8>, InputError> {
    let handle = plugin.memory_from_val(val).ok_or(InputError::NotABlock)?;
    let kernel_data_len = kernel_data_len(plugin).ok_or(InputError::OutOfBounds)?;
    check_bounds(handle.offset, handle.length, kernel_data_len, max_len)?;
    // now safe: the block is within the kernel data region, so the slice `memory_bytes` builds is
    // backed by mapped memory and at most `max_len` bytes long
    let bytes = plugin.memory_bytes(handle).map_err(|_| InputError::OutOfBounds)?;
    Ok(bytes.to_vec())
}

/// Like [`read_input`], but returns the bytes as a `String`, failing with [`InputError::NotUtf8`]
/// when they are not valid UTF-8.
pub(crate) fn read_input_str(plugin: &mut CurrentPlugin, val: &Val, max_len: usize) -> Result<String, InputError> {
    let bytes = read_input(plugin, val, max_len)?;
    String::from_utf8(bytes).map_err(|e| InputError::NotUtf8(e.utf8_error()))
}

/// what a plugin passes to `zhang_emit_error`
#[derive(Deserialize)]
struct EmitErrorPayload {
    message: String,
    /// kept as a value, so a span that is not well-formed falls back to the directive's span
    /// instead of rejecting the whole report
    #[serde(default)]
    span: Option<serde_json::Value>,
    #[serde(default)]
    metas: Option<HashMap<String, String>>,
}

/// the state the host functions of one plugin instance share
pub(super) struct HostState {
    /// the plugin's name, given as the `plugin` meta of the errors it reports
    plugin: String,
    /// the span of the plugin's directive, where an error without a usable span of its own is reported
    directive_span: SpanInfo,
    /// the errors the plugin reported, in call order
    errors: Vec<StageError>,
    /// the clock of the load, which `zhang_now` reads
    clock: LoadClock,
    /// the ledger timezone
    timezone: Tz,
    /// whether a `zhang_now` call is recorded as reading the date; not while the plugin registers or routes
    record: bool,
    /// whether the plugin read the time since its reads were last taken
    read: bool,
    /// what the plugin may read; `None` while it registers or handles a request as a router, when every
    /// file call is denied
    access: Option<FileAccess>,
    /// the inputs its file calls recorded, in call order
    inputs: Vec<ExtraInput>,
    /// what the router host functions reach, the server's query host and the ledger's info; `None` outside
    /// a router call, when they answer that they are unavailable
    pub(super) router: Option<(Arc<dyn RouterHost>, LedgerInfo)>,
}

/// the host side of one plugin instance: the host functions to link into it and what they collected
pub struct PluginHost {
    state: Arc<Mutex<HostState>>,
}

impl PluginHost {
    /// the host of an instance running as a stage: `clock` is the clock of the load, in the ledger timezone
    /// `timezone`, and every `zhang_now` call is recorded. Its file functions deny every path until
    /// [`PluginHost::with_files`] grants some; the hosts of [`PluginHost::registering`] and
    /// [`PluginHost::routing`] never get any
    pub fn new(plugin: impl Into<String>, directive_span: SpanInfo, clock: LoadClock, timezone: Tz) -> Self {
        Self::with_record(plugin.into(), directive_span, clock, timezone, true)
    }

    /// the host of an instance registering the plugin (`name`, `version`, `supported_type`): `zhang_now` returns
    /// the time of the load as for a stage, but records nothing, since what a plugin answers there is not ledger
    /// content
    pub fn registering(plugin: impl Into<String>, directive_span: SpanInfo, clock: LoadClock, timezone: Tz) -> Self {
        Self::with_record(plugin.into(), directive_span, clock, timezone, false)
    }

    /// the host of an instance handling an HTTP request as a router plugin. A request is not a load:
    /// `zhang_now` reads the ledger's clock afresh for each request (so a report page shows the
    /// current date even when the ledger was loaded days ago) and records nothing.
    pub fn routing(plugin: impl Into<String>, directive_span: SpanInfo, clock: LoadClock, timezone: Tz) -> Self {
        Self::with_record(plugin.into(), directive_span, clock, timezone, false)
    }

    fn with_record(plugin: String, directive_span: SpanInfo, clock: LoadClock, timezone: Tz, record: bool) -> Self {
        let state = HostState {
            plugin,
            directive_span,
            errors: vec![],
            clock,
            timezone,
            record,
            read: false,
            access: None,
            inputs: vec![],
            router: None,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// this host with the files `access` grants readable through `zhang_read_file` and `zhang_list_dir`
    pub fn with_files(self, access: FileAccess) -> Self {
        self.lock().access = Some(access);
        self
    }

    /// this host handling a request as a router: `zhang_query` runs queries through `host` and
    /// `zhang_ledger_info` answers `info`
    pub(super) fn with_router(self, host: Arc<dyn RouterHost>, info: LedgerInfo) -> Self {
        self.lock().router = Some((host, info));
        self
    }

    /// every host function zhang offers, bound to this host. The router functions are linked into every
    /// instance, so a plugin importing them still loads; outside a router call they answer that they are
    /// unavailable
    pub fn functions(&self) -> Vec<Function> {
        vec![
            self.bound(EMIT_ERROR, [PTR], [], emit_error),
            self.bound(NOW, [], [PTR], zhang_now),
            self.bound(READ_FILE, [PTR], [PTR], read_file),
            self.bound(LIST_DIR, [PTR], [PTR], list_dir),
            self.bound(QUERY_FUNCTION, [PTR], [PTR], router::zhang_query),
            self.bound(LEDGER_INFO_FUNCTION, [], [PTR], router::zhang_ledger_info),
        ]
    }

    /// the host function `function`, named `name`, bound to this host's state
    fn bound<F>(&self, name: &str, inputs: impl IntoIterator<Item = ValType>, outputs: impl IntoIterator<Item = ValType>, function: F) -> Function
    where
        F: 'static + Fn(&mut CurrentPlugin, &[Val], &mut [Val], UserData<HostState>) -> Result<(), extism::Error> + Sync + Send,
    {
        Function::new(name, inputs, outputs, UserData::Rust(self.state.clone()), function).with_namespace(EXTISM_USER_MODULE)
    }

    fn lock(&self) -> MutexGuard<'_, HostState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// take the errors the plugin reported so far
    pub fn take_errors(&self) -> Vec<StageError> {
        std::mem::take(&mut self.lock().errors)
    }

    /// whether the plugin read the time since the last call; a host that is registering the plugin never records it
    fn take_clock_read(&self) -> bool {
        std::mem::take(&mut self.lock().read)
    }

    /// take the inputs the plugin's file calls recorded so far
    pub fn take_inputs(&self) -> Vec<ExtraInput> {
        std::mem::take(&mut self.lock().inputs)
    }

    /// hand what the plugin reported and read so far to the pipeline: its errors, the date if it read the time,
    /// and the files and directories it read
    pub fn forward_to(&self, ctx: &mut StageContext) {
        for error in self.take_errors() {
            ctx.emit_error(error.kind, error.span, error.metas);
        }
        if self.take_clock_read() {
            ctx.add_input(ExtraInput::Clock);
        }
        for input in self.take_inputs() {
            ctx.add_input(input);
        }
    }
}

/// `zhang_emit_error(payload)`. It never traps: a payload it cannot read is reported as an invalid payload
fn emit_error(plugin: &mut CurrentPlugin, inputs: &[Val], _outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    // read the payload through the bounds-checked helper: a forged block header cannot make this
    // read out of bounds (a crash) or larger than the limit. Anything unreadable becomes `None`,
    // which `plugin_error` reports as an invalid payload, exactly as before.
    let payload = inputs.first().and_then(|offset| read_input(plugin, offset, EMIT_ERROR_MAX_LEN).ok());
    let state = state.get()?;
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let error = plugin_error(&state.plugin, &state.directive_span, payload.as_deref());
    state.errors.push(error);
    Ok(())
}

/// the [`ErrorKind::PluginError`] a `zhang_emit_error` payload reports; `payload` is `None` when the plugin
/// passed no readable memory block.
///
/// - span: the payload's own `span` when it is a well-formed span, otherwise `directive_span`
/// - metas: the payload's `metas`, then `plugin` (the plugin's name) and `message`, which win over a
///   payload meta of the same name
///
/// A payload that is not valid JSON of that shape is reported on `directive_span` with a message saying so.
fn plugin_error(plugin: &str, directive_span: &SpanInfo, payload: Option<&[u8]>) -> StageError {
    let parsed = payload
        .ok_or_else(|| "it is not a memory block".to_owned())
        .and_then(|payload| serde_json::from_slice::<EmitErrorPayload>(payload).map_err(|e| e.to_string()));
    let (message, span, mut metas) = match parsed {
        Ok(payload) => {
            let span = payload
                .span
                .and_then(|span| well_formed_span(plugin, span))
                .unwrap_or_else(|| directive_span.clone());
            (payload.message, span, payload.metas.unwrap_or_default())
        }
        Err(e) => {
            warn!("plugin {plugin} called {EMIT_ERROR} with an invalid payload: {e}");
            (
                format!("the plugin called {EMIT_ERROR} with an invalid payload: {e}"),
                directive_span.clone(),
                HashMap::new(),
            )
        }
    };
    metas.insert(PLUGIN_META.to_owned(), plugin.to_owned());
    metas.insert(MESSAGE_META.to_owned(), message);
    StageError {
        kind: ErrorKind::PluginError,
        span,
        metas,
    }
}

/// the span a plugin gave, if it is well-formed
fn well_formed_span(plugin: &str, span: serde_json::Value) -> Option<SpanInfo> {
    match serde_json::from_value::<SpanInfo>(span) {
        Ok(span) if span.start <= span.end => Some(span),
        Ok(span) => {
            warn!(
                "plugin {plugin} reported an error on a span ending before it starts ({}..{}); using its directive's span",
                span.start, span.end
            );
            None
        }
        Err(e) => {
            warn!("plugin {plugin} reported an error on a span that is not well-formed ({e}); using its directive's span");
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// `zhang_now`: the current time of the load
// ---------------------------------------------------------------------------------------------------------------

// The host reads its clock once per load, on the first call from any plugin, so every call of a load returns the same
// value. A call from a processor or mapper records `ExtraInput::Clock`; a call while the plugin registers does not.
// `NOW` documents the ABI.

/// `zhang_now()`: the current time of the load, read from its clock on the first call
fn zhang_now(plugin: &mut CurrentPlugin, _inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let now = {
        let state = state.get()?;
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.read |= state.record;
        state.clock.now().with_timezone(&state.timezone)
    };
    let payload = serde_json::to_string(&HostOk::Ok(now_payload(&now)))?;
    plugin.memory_set_val(&mut outputs[0], payload.as_str())
}

/// what `zhang_now` returns for the time `now`, in the ledger timezone
fn now_payload(now: &DateTime<Tz>) -> NowPayload {
    NowPayload {
        now: now.to_rfc3339_opts(SecondsFormat::AutoSi, false),
        today: now.date_naive().format("%Y-%m-%d").to_string(),
        timezone: now.timezone().name().to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// File access: `zhang_read_file` and `zhang_list_dir`, the `allowed_paths` capability
// ---------------------------------------------------------------------------------------------------------------

/// the most bytes [`read_input_str`] will read for the path a plugin passes to `zhang_read_file` or
/// `zhang_list_dir`: 4 KiB, Linux's `PATH_MAX`. A path relative to the ledger root is far shorter.
const FILE_PATH_MAX_LEN: usize = 4096;

/// `zhang_read_file(path) -> result`
fn read_file(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    file_call(plugin, inputs, outputs, state, FileAccess::read_file)
}

/// `zhang_list_dir(path) -> result`
fn list_dir(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    file_call(plugin, inputs, outputs, state, FileAccess::list_dir)
}

/// run a file function on the path the plugin passed and hand it the JSON result; a path it cannot read is an
/// `invalid` result, not a trap
fn file_call<T: Serialize>(
    plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>, call: impl FnOnce(&FileAccess, &str) -> FileCall<T>,
) -> Result<(), extism::Error> {
    // through the bounds-checked helper, so a forged block header cannot make this read out of bounds (a
    // crash) or larger than the limit; any unreadable path (an empty one is no block either) is `invalid`
    let path = match inputs.first() {
        None => Err(FileError::new(FileErrorKind::Invalid, "the path is not a memory block")),
        Some(offset) => read_input_str(plugin, offset, FILE_PATH_MAX_LEN).map_err(|e| FileError::new(FileErrorKind::Invalid, format!("the path {e}"))),
    };
    let result = {
        let state = state.get()?;
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let call = run_file_call(state.access.as_ref(), path, call);
        state.inputs.extend(call.input);
        serde_json::to_vec(&call.result).expect("file results always serialize to JSON")
    };
    let output = outputs.first_mut().ok_or_else(|| extism::Error::msg("a file function returns one value"))?;
    plugin.memory_set_val(output, result)
}

/// the outcome of a file call on `path`, the path the plugin passed or why it could not be read. Without
/// `access`, while the plugin registers or handles a request as a router, every path is denied
fn run_file_call<T>(access: Option<&FileAccess>, path: Result<String, FileError>, call: impl FnOnce(&FileAccess, &str) -> FileCall<T>) -> FileCall<T> {
    match (path, access) {
        (Err(error), _) => FileCall::rejected(error),
        (Ok(path), None) => FileCall::rejected(FileError::new(
            FileErrorKind::Denied,
            format!("{path:?}: files can only be read while the plugin runs as a processor or mapper"),
        )),
        (Ok(path), Some(access)) => call(access, &path),
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::SpanInfo;

    use super::{check_bounds, plugin_error, run_file_call, InputError, EMIT_ERROR_MAX_LEN, KERNEL_DATA_START};
    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::plugin::files::{FileAccess, FileError, FileErrorKind};

    /// a plausible data-region size: one page minus the `MemoryRoot`, the value a freshly started
    /// kernel reports
    const DATA_LEN: u64 = 65536 - 64;

    #[test]
    fn check_bounds_accepts_a_block_inside_the_data_region() {
        // the first real block, just past the data start, well under the limit
        assert_eq!(check_bounds(KERNEL_DATA_START + 12, 30, DATA_LEN, EMIT_ERROR_MAX_LEN), Ok(()));
        // a block ending exactly at the end of the data region is still in bounds
        let len = 128;
        let offset = KERNEL_DATA_START + DATA_LEN - len;
        assert_eq!(check_bounds(offset, len, DATA_LEN, EMIT_ERROR_MAX_LEN), Ok(()));
    }

    #[test]
    fn check_bounds_rejects_a_forged_length_past_the_data_region() {
        // the ~2 GiB forge: caught as too large before any read, so it can never crash the host
        assert_eq!(
            check_bounds(KERNEL_DATA_START + 12, 0x7f00_0000, DATA_LEN, EMIT_ERROR_MAX_LEN),
            Err(InputError::TooLarge {
                len: 0x7f00_0000,
                max: EMIT_ERROR_MAX_LEN
            })
        );
        // the smaller "just over the real size" forge: under the limit but out of bounds, so it is
        // rejected instead of leaking the plugin's own kernel memory
        assert_eq!(
            check_bounds(KERNEL_DATA_START + 12, 0x1_0004, DATA_LEN, EMIT_ERROR_MAX_LEN),
            Err(InputError::OutOfBounds)
        );
    }

    #[test]
    fn check_bounds_rejects_an_offset_before_the_data_region() {
        // an offset pointing into the `MemoryRoot` itself, not a data block
        assert_eq!(
            check_bounds(KERNEL_DATA_START - 1, 8, DATA_LEN, EMIT_ERROR_MAX_LEN),
            Err(InputError::OutOfBounds)
        );
        assert_eq!(check_bounds(0, 8, DATA_LEN, EMIT_ERROR_MAX_LEN), Err(InputError::OutOfBounds));
    }

    #[test]
    fn check_bounds_rejects_an_offset_plus_length_that_overflows() {
        assert_eq!(check_bounds(u64::MAX, 1, DATA_LEN, 2), Err(InputError::OutOfBounds));
    }

    #[test]
    fn check_bounds_enforces_the_max_len_for_a_well_formed_block() {
        // a block that really is in bounds but larger than the caller allows
        assert_eq!(check_bounds(KERNEL_DATA_START, 3, DATA_LEN, 2), Err(InputError::TooLarge { len: 3, max: 2 }));
        // exactly the limit is allowed
        assert_eq!(check_bounds(KERNEL_DATA_START, 2, DATA_LEN, 2), Ok(()));
    }

    fn directive_span() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 25,
            content: "plugin \"validator.wasm\"\n".to_owned(),
            filename: Some(PathBuf::from("main.zhang")),
            ..SpanInfo::default()
        }
    }

    fn metas(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn should_report_on_the_directive_span_with_the_plugin_and_message() {
        let error = plugin_error("validator", &directive_span(), Some(br#"{"message": "payee is missing"}"#));

        assert_eq!(error.kind, ErrorKind::PluginError);
        assert_eq!(error.span, directive_span());
        assert_eq!(error.metas, metas(&[("plugin", "validator"), ("message", "payee is missing")]));
    }

    #[test]
    fn should_keep_plugin_metas_but_let_the_host_keys_win() {
        let payload = br#"{"message": "payee is missing", "metas": {"rule": "R1", "plugin": "spoofed", "message": "spoofed"}}"#;

        let error = plugin_error("validator", &directive_span(), Some(payload));

        assert_eq!(error.metas, metas(&[("rule", "R1"), ("plugin", "validator"), ("message", "payee is missing")]));
    }

    #[test]
    fn should_use_a_well_formed_span_from_the_plugin() {
        let payload = br#"{"message": "m", "span": {"start": 40, "end": 90, "content": "2024-01-02 * \"lunch\"", "filename": "other.zhang"}, "metas": null}"#;

        let error = plugin_error("validator", &directive_span(), Some(payload));

        assert_eq!(
            error.span,
            SpanInfo {
                start: 40,
                end: 90,
                content: "2024-01-02 * \"lunch\"".to_owned(),
                filename: Some(PathBuf::from("other.zhang")),
                ..SpanInfo::default()
            }
        );
    }

    #[test]
    fn should_fall_back_to_the_directive_span_for_a_span_that_is_not_well_formed() {
        for span in [
            r#"null"#,
            r#""line 3""#,
            r#"{"start": 3}"#,
            r#"{"start": -1, "end": 4, "content": ""}"#,
            r#"{"start": 9, "end": 4, "content": ""}"#,
        ] {
            let payload = format!(r#"{{"message": "m", "span": {span}}}"#);

            let error = plugin_error("validator", &directive_span(), Some(payload.as_bytes()));

            assert_eq!(error.span, directive_span(), "span {span}");
            assert_eq!(error.metas, metas(&[("plugin", "validator"), ("message", "m")]), "span {span}");
        }
    }

    #[test]
    fn should_report_an_invalid_payload_without_failing() {
        for payload in [
            &b"not json"[..],
            br#"{"metas": {"rule": "R1"}}"#,
            br#"{"message": 42}"#,
            br#"{"message": "m", "metas": {"count": 3}}"#,
            b"\xff\xfe",
        ] {
            let error = plugin_error("validator", &directive_span(), Some(payload));

            assert_eq!(error.kind, ErrorKind::PluginError);
            assert_eq!(error.span, directive_span());
            assert_eq!(error.metas.len(), 2, "only the host metas: {:?}", error.metas);
            assert_eq!(error.metas["plugin"], "validator");
            assert!(
                error.metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
                "{}",
                error.metas["message"]
            );
        }
    }

    #[test]
    fn should_deny_every_file_call_without_file_access() {
        let call = run_file_call(None, Ok("documents/receipt.txt".to_owned()), FileAccess::read_file);

        let error = call.result.unwrap_err();
        assert_eq!(error.kind, FileErrorKind::Denied);
        assert!(
            error.message.contains("only be read while the plugin runs as a processor or mapper"),
            "{}",
            error.message
        );
        assert_eq!(call.input, None);
        assert_eq!(
            serde_json::to_value(run_file_call(None, Ok(".".to_owned()), FileAccess::list_dir).result).unwrap()["Err"]["kind"],
            "denied"
        );
    }

    #[test]
    fn should_pass_on_a_path_the_plugin_could_not_pass() {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let access = FileAccess::new(vec![PathBuf::new()], source, Path::new("/"));
        let unreadable = FileError::new(FileErrorKind::Invalid, "the path is not UTF-8 text");

        let call = run_file_call(Some(&access), Err(unreadable.clone()), FileAccess::read_file);

        assert_eq!(call.result, Err(unreadable));
        assert_eq!(call.input, None);
    }

    #[test]
    fn should_report_a_missing_memory_block_as_an_invalid_payload() {
        let error = plugin_error("validator", &directive_span(), None);

        assert_eq!(error.span, directive_span());
        assert_eq!(
            error.metas["message"],
            "the plugin called zhang_emit_error with an invalid payload: it is not a memory block"
        );
    }

    mod now {
        use chrono::{DateTime, Utc};
        use chrono_tz::Tz;

        use super::super::{now_payload, HostOk, NowPayload};

        fn payload(rfc3339: &str, timezone: Tz) -> NowPayload {
            now_payload(&rfc3339.parse::<DateTime<Utc>>().unwrap().with_timezone(&timezone))
        }

        #[test]
        fn should_give_the_time_date_and_name_of_the_ledger_timezone() {
            assert_eq!(
                payload("2024-03-15T16:30:00Z", Tz::Asia__Shanghai),
                NowPayload {
                    now: "2024-03-16T00:30:00+08:00".to_owned(),
                    today: "2024-03-16".to_owned(),
                    timezone: "Asia/Shanghai".to_owned(),
                }
            );
            assert_eq!(payload("2024-03-15T16:30:00.250Z", Tz::UTC).now, "2024-03-15T16:30:00.250+00:00");
            assert_eq!(payload("2024-03-15T03:00:00Z", Tz::America__New_York).today, "2024-03-14");
        }

        #[test]
        fn should_wrap_the_payload_in_ok() {
            let json = serde_json::to_string(&HostOk::Ok(payload("2024-03-15T16:30:00Z", Tz::Europe__Berlin))).unwrap();

            assert_eq!(
                json,
                r#"{"Ok":{"now":"2024-03-15T17:30:00+01:00","today":"2024-03-15","timezone":"Europe/Berlin"}}"#
            );
        }
    }
}
