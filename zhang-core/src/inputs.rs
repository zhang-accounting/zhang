//! Extra inputs: what a load depended on besides the ledger's own files.
//!
//! The ledger's `.zhang`/`.bean` files are listed in [`Ledger::visited_files`](crate::ledger::Ledger::visited_files),
//! which also feeds the file editor. Everything else a load read (a plugin's module, a file or directory a plugin
//! read, the current date), or looked for (an included file that does not exist), is an [`ExtraInput`] in
//! [`Ledger::extra_inputs`](crate::ledger::Ledger::extra_inputs),
//! so a server can reload the ledger when one of them changes without listing receipts or modules in the editor.

use std::path::{Component, Path, PathBuf};

/// something a load read besides the ledger's own files; a change to it makes the loaded ledger stale
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExtraInput {
    /// a file, relative to the ledger root. Creating, changing or removing it makes the ledger stale
    File(PathBuf),
    /// a directory, relative to the ledger root (empty for the root itself). Any change under it makes the ledger
    /// stale, including files created in it
    Dir(PathBuf),
    /// the current date in the ledger timezone: the ledger is stale once the date changes
    Clock,
}

impl ExtraInput {
    /// the input for the module of a `plugin` directive, which the data source fetches relative to the ledger root.
    ///
    /// `None` for a remote module (a URL such as `https://…` or `s3://…`), which is not a file, and for a module
    /// outside the ledger root, which a change to the root never touches
    pub fn plugin_module(root: &Path, module: &str) -> Option<ExtraInput> {
        if module.contains("://") {
            return None;
        }
        ExtraInput::ledger_file(root, Path::new(module))
    }

    /// the input for the file at `path`, absolute or relative to the ledger root `root`; `None` for a file outside
    /// the root, which a change to the root never touches
    pub fn ledger_file(root: &Path, path: &Path) -> Option<ExtraInput> {
        relative_to_root(root, path).filter(|path| !path.as_os_str().is_empty()).map(ExtraInput::File)
    }
}

/// `path` relative to `root`, normalized lexically (no `.` or `..` components); `None` when it is outside the root.
/// A relative `path` is taken as relative to the root already
fn relative_to_root(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = if path.has_root() { path.strip_prefix(root).ok()? } else { path };
    normalize_relative(relative)
}

/// a relative `path` normalized lexically (no `.` or `..` components, no empty ones); `None` when it is absolute or
/// climbs above its start. Inputs a plugin's file functions record are cleaned the same way
pub(crate) fn normalize_relative(relative: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod test {
    use std::path::{Path, PathBuf};

    use super::ExtraInput;

    fn module(module: &str) -> Option<ExtraInput> {
        ExtraInput::plugin_module(Path::new("/ledger"), module)
    }

    fn file(path: &str) -> Option<ExtraInput> {
        Some(ExtraInput::File(PathBuf::from(path)))
    }

    #[test]
    fn should_record_a_local_module_relative_to_the_root() {
        assert_eq!(module("plugins/echo.wasm"), file("plugins/echo.wasm"));
        assert_eq!(module("./plugins//echo.wasm"), file("plugins/echo.wasm"));
        assert_eq!(module("plugins/old/../echo.wasm"), file("plugins/echo.wasm"));
        assert_eq!(module("/ledger/plugins/echo.wasm"), file("plugins/echo.wasm"));
    }

    #[test]
    fn should_not_record_a_remote_module() {
        assert_eq!(module("https://example.com/echo.wasm"), None);
        assert_eq!(module("s3://bucket/echo.wasm"), None);
    }

    #[test]
    fn should_not_record_a_module_outside_the_root() {
        assert_eq!(module("../shared/echo.wasm"), None);
        assert_eq!(module("plugins/../../echo.wasm"), None);
        assert_eq!(module("/elsewhere/echo.wasm"), None);
        assert_eq!(module("/ledger-other/echo.wasm"), None);
        // the root itself is not a module file
        assert_eq!(module("."), None);
        assert_eq!(module("/ledger"), None);
    }
}
