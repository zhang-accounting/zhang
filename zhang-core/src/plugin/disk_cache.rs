//! The compiled code of plugins, kept on disk across restarts.
//!
//! Compiling a plugin's module to native code is the expensive part of running a plugin ([`super::runtime`]): hundreds
//! of milliseconds for the Lua interpreter every Lua plugin runs on. wasmtime can keep the code it compiles in a
//! directory and read it back whenever the same module is compiled with the same settings again, in this process or a
//! later one, so a restart, or a load compiling a ledger's own plugins anew, costs a fraction of that. zhang sets this
//! cache up itself rather than leaving it to extism, whose default reads wasmtime's configuration file and fails every
//! compilation when wasmtime's default directory cannot be created (a read-only root file system, a user without a
//! home directory):
//!
//! - the directory is [`CACHE_DIR_ENV`] when it is set, else `$XDG_CACHE_HOME/zhang/wasmtime`, else
//!   `$HOME/.cache/zhang/wasmtime`; [`CACHE_DIR_ENV`] set to nothing turns the cache off;
//! - wasmtime keeps the files in it under [`SOFT_LIMIT`] in all: when a compilation adds one and the last clean-up is
//!   an hour old, it deletes the least recently used files down to 70 % of the limit. It also deletes whatever it does
//!   not recognise in the directory, so the directory must be one only the cache uses;
//! - a directory that cannot be created or written to turns the cache off with one warning, instead of failing every
//!   compilation: plugins still load, compiled anew each time.
//!
//! The directory holds native code the server runs, so whoever can write to it can run code as the server: it must be
//! as trusted as the zhang binary. A directory zhang creates is readable by its own user only (on unix).

use std::ffi::OsString;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use extism::{Cache, CacheConfig};
use log::{info, warn};

use crate::plugin_policy::{Size, MIB};

/// the environment variable setting the directory of the disk cache; set to nothing, it turns the cache off
pub const CACHE_DIR_ENV: &str = "ZHANG_PLUGIN_CACHE_DIR";

/// how many bytes of compiled code the cache keeps before wasmtime trims it (its own default is 512 MiB). A module is kept
/// once whatever the config of the plugins running it: the Lua interpreter every Lua plugin runs on takes about half a
/// MiB, compiled and compressed, so this leaves room for a ledger's other modules, and for the code earlier versions of
/// zhang compiled, until it is trimmed
pub const SOFT_LIMIT: u64 = 128 * MIB;

/// The disk cache every plugin compiles with, set up from the environment at the first call: none when
/// [`CACHE_DIR_ENV`] turns it off, or when its directory cannot be used, which the first call logs as a warning.
pub fn shared() -> Option<&'static Cache> {
    static SHARED: OnceLock<Option<Cache>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let env = |name: &str| std::env::var_os(name);
            set_up(env(CACHE_DIR_ENV), env("XDG_CACHE_HOME"), env("HOME"))
        })
        .as_ref()
}

/// The disk cache the values of [`CACHE_DIR_ENV`], `XDG_CACHE_HOME` and `HOME` give, logging where it is, or why there
/// is none.
pub(super) fn set_up(cache_dir: Option<OsString>, xdg_cache_home: Option<OsString>, home: Option<OsString>) -> Option<Cache> {
    let opened = directory(cache_dir, xdg_cache_home, home).and_then(|dir| dir.map(|dir| open(&dir).map(|cache| (dir, cache))).transpose());
    match opened {
        Ok(Some((dir, cache))) => {
            info!(
                "plugins keep their compiled code in {} ({CACHE_DIR_ENV}), {} at most",
                dir.display(),
                Size(SOFT_LIMIT)
            );
            Some(cache)
        }
        Ok(None) => {
            info!("plugins compile without a disk cache: {CACHE_DIR_ENV} is set to nothing");
            None
        }
        Err(reason) => {
            warn!(
                "plugins compile without a disk cache, so each start compiles them anew: {reason}. Set {CACHE_DIR_ENV} to a directory zhang can write to, or to nothing to turn the cache off"
            );
            None
        }
    }
}

/// The directory of the disk cache, from the values of [`CACHE_DIR_ENV`], `XDG_CACHE_HOME` and `HOME`: none when
/// [`CACHE_DIR_ENV`] is set to nothing (or only spaces), an error when it is not set and there is no home directory to
/// put it in. A relative [`CACHE_DIR_ENV`] is taken from the current directory; a relative `XDG_CACHE_HOME` or `HOME` is
/// ignored, as the XDG base directory specification says.
fn directory(cache_dir: Option<OsString>, xdg_cache_home: Option<OsString>, home: Option<OsString>) -> Result<Option<PathBuf>, String> {
    if let Some(dir) = cache_dir {
        if dir.to_str().is_some_and(|it| it.trim().is_empty()) {
            return Ok(None);
        }
        return std::path::absolute(&dir).map(Some).map_err(|e| format!("{CACHE_DIR_ENV}: {e}"));
    }
    let absolute = |value: Option<OsString>| value.map(PathBuf::from).filter(|it| it.is_absolute());
    let base = match (absolute(xdg_cache_home), absolute(home)) {
        (Some(cache_home), _) => cache_home,
        (None, Some(home)) => home.join(".cache"),
        (None, None) => return Err(format!("{CACHE_DIR_ENV} is not set, and neither XDG_CACHE_HOME nor HOME is an absolute path")),
    };
    Ok(Some(base.join("zhang").join("wasmtime")))
}

/// The disk cache in `dir`, keeping at most [`SOFT_LIMIT`]. The directory is created when it is missing (readable by this
/// user only, on unix), and written to once, to see that it can be; the error says why it cannot be used.
fn open(dir: &Path) -> Result<Cache, String> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|e| format!("{} cannot be created: {e}", dir.display()))?;
    // wasmtime only creates the directory: without this, a directory it cannot write to would fail every compilation's
    // write, quietly. The probe is a file of the top level, which wasmtime's clean-up would delete if it were left
    let probe = dir.join(format!(".zhang-probe-{}", std::process::id()));
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
        .and_then(|mut file| file.write_all(b"zhang"));
    let _ = std::fs::remove_file(&probe);
    written.map_err(|e| format!("{} cannot be written to: {e}", dir.display()))?;
    let mut config = CacheConfig::new();
    config.with_directory(dir).with_files_total_size_soft_limit(SOFT_LIMIT);
    Cache::new(config).map_err(|e| format!("{} cannot be used: {e:#}", dir.display()))
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{directory, open, set_up, SOFT_LIMIT};

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn the_directory_is_the_variable_else_the_xdg_cache_home_else_the_home() {
        let set = directory(os("/srv/zhang-cache"), os("/xdg"), os("/home/me"));
        assert_eq!(set, Ok(Some(PathBuf::from("/srv/zhang-cache"))), "the variable wins");
        assert_eq!(
            directory(os(""), os("/xdg"), os("/home/me")),
            Ok(None),
            "set to nothing, it turns the cache off"
        );
        assert_eq!(directory(os("  "), None, None), Ok(None));
        assert_eq!(
            directory(os("cache"), None, None),
            Ok(Some(std::env::current_dir().unwrap().join("cache"))),
            "a relative directory is taken from the current one"
        );
        assert_eq!(directory(None, os("/xdg"), os("/home/me")), Ok(Some(PathBuf::from("/xdg/zhang/wasmtime"))));
        assert_eq!(directory(None, None, os("/home/me")), Ok(Some(PathBuf::from("/home/me/.cache/zhang/wasmtime"))));
        assert_eq!(
            directory(None, os("relative"), os("/home/me")),
            Ok(Some(PathBuf::from("/home/me/.cache/zhang/wasmtime"))),
            "a relative XDG_CACHE_HOME is ignored"
        );
        assert_eq!(
            directory(None, os(""), os("/home/me")),
            Ok(Some(PathBuf::from("/home/me/.cache/zhang/wasmtime")))
        );
        assert!(directory(None, None, None).unwrap_err().contains("neither XDG_CACHE_HOME nor HOME"));
        assert!(directory(None, None, os("")).is_err(), "no home, no default directory");
    }

    #[test]
    fn a_usable_directory_is_created_for_this_user_only_and_left_clean() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("zhang").join("wasmtime");
        let cache = open(&dir).expect("a directory zhang can create is used");
        assert_eq!(cache.files_total_size_soft_limit(), SOFT_LIMIT);
        assert_eq!(cache.directory(), &dir.canonicalize().unwrap());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the probe file is removed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        }
        assert!(set_up(Some(dir.into_os_string()), None, None).is_some());
    }

    #[test]
    fn a_file_in_the_way_turns_the_cache_off() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cache");
        std::fs::write(&file, "not a directory").unwrap();
        let error = open(&file).expect_err("a file is not a directory");
        assert!(error.contains("cannot be created"), "{error}");
        let error = open(&file.join("wasmtime")).expect_err("nor is a directory below a file");
        assert!(error.contains("cannot be created"), "{error}");
        assert!(set_up(Some(file.into_os_string()), None, None).is_none());
        assert_eq!(
            std::fs::read_to_string(root.path().join("cache")).unwrap(),
            "not a directory",
            "the file is left alone"
        );
    }

    /// whether this process can write to a directory whose mode forbids it, as root can: a read-only directory is no
    /// test then
    fn writes_past_permissions(dir: &Path) -> bool {
        std::fs::write(dir.join("privileged"), "").is_ok()
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_turns_the_cache_off() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("read-only");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        if writes_past_permissions(&dir) {
            eprintln!("skipped: this user writes to read-only directories");
            return;
        }
        let error = open(&dir).expect_err("a directory zhang cannot write to is not used");
        assert!(error.contains("cannot be written to"), "{error}");
        let below = open(&dir.join("wasmtime")).expect_err("nor one it cannot create");
        assert!(below.contains("cannot be created"), "{below}");
        assert!(set_up(Some(dir.clone().into_os_string()), None, None).is_none());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}
