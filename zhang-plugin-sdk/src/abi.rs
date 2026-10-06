//! The raw plugin ABI: config reads and the zhang host functions.
//!
//! Inside a WASM module these go through the Extism kernel. Every host function has its own wrapper, so a
//! plugin imports only the host functions it actually calls: importing one makes the plugin fail to load on a
//! zhang that predates it. On any other target nothing is linked: config reads find nothing and host
//! functions answer [`HostErrorKind::Unavailable`](crate::HostErrorKind::Unavailable), so plugin logic can be
//! unit tested natively.

pub(crate) use zhang_shared::plugin_abi::import::{LEDGER_INFO, LIST_DIR, NOW, QUERY, READ_FILE};

use crate::error::HostError;

#[cfg(target_arch = "wasm32")]
mod imp {
    use extism_pdk::Memory;

    use super::{LEDGER_INFO, LIST_DIR, NOW, QUERY, READ_FILE};
    use crate::error::{HostError, HostErrorKind};

    #[link(wasm_import_module = "extism:host/user")]
    extern "C" {
        fn zhang_emit_error(payload: u64);
        fn zhang_now() -> u64;
        fn zhang_read_file(path: u64) -> u64;
        fn zhang_list_dir(path: u64) -> u64;
        fn zhang_query(bql: u64) -> u64;
        fn zhang_ledger_info() -> u64;
    }

    pub(crate) fn config_get(key: &str) -> Option<String> {
        extism_pdk::config::get(key).ok().flatten()
    }

    /// the bytes of the memory block a host function answered with, freeing the block
    fn answer(function: &str, offset: u64) -> Result<Vec<u8>, HostError> {
        let memory = Memory::find(offset).ok_or_else(|| HostError::new(HostErrorKind::Other, format!("{function} answered no memory block")))?;
        let bytes = memory.to_vec();
        memory.free();
        Ok(bytes)
    }

    /// call the host function `call`, named `function`, with `argument` in a new memory block
    fn call_with(function: &str, argument: &[u8], call: unsafe extern "C" fn(u64) -> u64) -> Result<Vec<u8>, HostError> {
        let memory = Memory::from_bytes(argument).map_err(|e| HostError::new(HostErrorKind::Other, format!("cannot pass the argument of {function}: {e}")))?;
        // SAFETY: the host function takes the offset of a memory block and returns the offset of another
        let offset = unsafe { call(memory.offset()) };
        memory.free();
        answer(function, offset)
    }

    pub(crate) fn emit_error(payload: &[u8]) {
        let Ok(memory) = Memory::from_bytes(payload) else {
            return;
        };
        // SAFETY: the host reads the block and never traps
        unsafe { zhang_emit_error(memory.offset()) };
        memory.free();
    }

    pub(crate) fn now() -> Result<Vec<u8>, HostError> {
        // SAFETY: the host function takes nothing and returns the offset of a memory block
        answer(NOW, unsafe { zhang_now() })
    }

    pub(crate) fn read_file(path: &str) -> Result<Vec<u8>, HostError> {
        call_with(READ_FILE, path.as_bytes(), zhang_read_file)
    }

    pub(crate) fn list_dir(path: &str) -> Result<Vec<u8>, HostError> {
        call_with(LIST_DIR, path.as_bytes(), zhang_list_dir)
    }

    pub(crate) fn query(bql: &str) -> Result<Vec<u8>, HostError> {
        call_with(QUERY, bql.as_bytes(), zhang_query)
    }

    pub(crate) fn ledger_info() -> Result<Vec<u8>, HostError> {
        // SAFETY: the host function takes nothing and returns the offset of a memory block
        answer(LEDGER_INFO, unsafe { zhang_ledger_info() })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{LEDGER_INFO, LIST_DIR, NOW, QUERY, READ_FILE};
    use crate::error::{outside_zhang, HostError};

    pub(crate) fn config_get(_key: &str) -> Option<String> {
        None
    }

    pub(crate) fn emit_error(_payload: &[u8]) {}

    pub(crate) fn now() -> Result<Vec<u8>, HostError> {
        Err(outside_zhang(NOW))
    }

    pub(crate) fn read_file(_path: &str) -> Result<Vec<u8>, HostError> {
        Err(outside_zhang(READ_FILE))
    }

    pub(crate) fn list_dir(_path: &str) -> Result<Vec<u8>, HostError> {
        Err(outside_zhang(LIST_DIR))
    }

    pub(crate) fn query(_bql: &str) -> Result<Vec<u8>, HostError> {
        Err(outside_zhang(QUERY))
    }

    pub(crate) fn ledger_info() -> Result<Vec<u8>, HostError> {
        Err(outside_zhang(LEDGER_INFO))
    }
}

/// the config value of `key`, if the host set one
pub(crate) fn config_get(key: &str) -> Option<String> {
    imp::config_get(key)
}

/// `zhang_emit_error(payload)`; it has no answer
pub(crate) fn emit_error(payload: &[u8]) {
    imp::emit_error(payload)
}

/// `zhang_now()`, the JSON answer
pub(crate) fn now() -> Result<Vec<u8>, HostError> {
    imp::now()
}

/// `zhang_read_file(path)`, the JSON answer
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>, HostError> {
    imp::read_file(path)
}

/// `zhang_list_dir(path)`, the JSON answer
pub(crate) fn list_dir(path: &str) -> Result<Vec<u8>, HostError> {
    imp::list_dir(path)
}

/// `zhang_query(bql)`, the JSON answer
pub(crate) fn query(bql: &str) -> Result<Vec<u8>, HostError> {
    imp::query(bql)
}

/// `zhang_ledger_info()`, the JSON answer
pub(crate) fn ledger_info() -> Result<Vec<u8>, HostError> {
    imp::ledger_info()
}
