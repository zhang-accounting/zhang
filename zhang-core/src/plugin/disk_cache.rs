//! The compiled code of plugins, kept on disk across restarts.
//!
//! Compiling a plugin's module to native code is the expensive part of running a plugin ([`super::runtime`]): hundreds
//! of milliseconds for the Lua interpreter every Lua plugin runs on. wasmtime can keep the code it compiles in a
//! directory and read it back whenever the same module is compiled with the same settings again, in this process or a
//! later one, so a restart, or a load compiling a ledger's own plugins anew, costs a fraction of that. zhang sets this
//! cache up itself rather than leaving it to extism, whose default reads wasmtime's configuration file and fails every
//! compilation when wasmtime's default directory cannot be created (a read-only root file system, a user without a
//! home directory).
//!
//! **Where.** The cache directory is [`CACHE_DIR_ENV`] when it is set, else `$XDG_CACHE_HOME/zhang`, else
//! `$HOME/.cache/zhang`; [`CACHE_DIR_ENV`] set to nothing turns the cache off. wasmtime never gets that directory
//! itself: its clean-up deletes whatever it does not recognise as its own in the directory it is given, two levels
//! deep, so given an operator's directory (the volume holding the ledger, say) it would delete the ledger. It gets
//! [`SUBDIRECTORY`] inside it, which zhang creates and which holds nothing but the cache. A cache directory in the
//! folder of the ledger served, or holding it, is refused: besides the clean-up, the server's file watcher would see
//! every compilation.
//!
//! **Size.** wasmtime keeps the files of [`SUBDIRECTORY`] under [`SOFT_LIMIT`] in all: when a compilation adds one and
//! the last clean-up is an hour old, it deletes the least recently used files down to 70 % of the limit.
//!
//! **Trust.** wasmtime loads what it finds in the cache as native code, with no integrity check, under names anyone
//! can compute: whoever can write to the cache can run code as the server. So, on unix, the cache directory and
//! [`SUBDIRECTORY`] must belong to the user zhang runs as, and neither may be writable by its group or by others;
//! [`SUBDIRECTORY`] may not be a symbolic link. What zhang creates is readable by its own user only.
//!
//! A cache that cannot be had (a directory that cannot be created or written to, or one refused above) turns the cache
//! off with one warning, instead of failing every compilation: plugins still load, compiled anew each time.

use std::ffi::OsString;
use std::fs::{DirBuilder, Metadata, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use extism::{Cache, CacheConfig};
use log::{info, warn};

use crate::plugin_policy::{Size, MIB};

/// the environment variable setting the cache directory; set to nothing, it turns the cache off
pub const CACHE_DIR_ENV: &str = "ZHANG_PLUGIN_CACHE_DIR";

/// the directory inside the cache directory that wasmtime keeps its files in, and the only one its clean-up reads
pub const SUBDIRECTORY: &str = "zhang-wasmtime-cache";

/// how many bytes of compiled code the cache keeps before wasmtime trims it (its own default is 512 MiB). A module is kept
/// once whatever the config of the plugins running it: the Lua interpreter every Lua plugin runs on takes about half a
/// MiB, compiled and compressed, so this leaves room for a ledger's other modules, and for the code earlier versions of
/// zhang compiled, until it is trimmed
pub const SOFT_LIMIT: u64 = 128 * MIB;

/// the cache every plugin compiles with, set up once
static SHARED: OnceLock<Option<Cache>> = OnceLock::new();

/// The disk cache every plugin compiles with, set up from the environment at the first call (or by
/// [`set_up_serving`]): none when [`CACHE_DIR_ENV`] turns it off, or when it cannot be had, which the first call logs as
/// a warning.
pub fn shared() -> Option<&'static Cache> {
    shared_for(None)
}

/// The disk cache every plugin compiles with, set up now for a server serving the ledger in `ledger_root`, a folder of
/// the local disk (none for a ledger on remote storage), unless it is set up already: a cache directory in that folder,
/// or holding it, is refused. `zhang serve` calls it before it loads the ledger, so where the cache is, or why there is
/// none, is logged when it starts.
pub fn set_up_serving(ledger_root: Option<&Path>) -> Option<&'static Cache> {
    shared_for(ledger_root)
}

fn shared_for(ledger_root: Option<&Path>) -> Option<&'static Cache> {
    SHARED
        .get_or_init(|| {
            let env = |name: &str| std::env::var_os(name);
            set_up(env(CACHE_DIR_ENV), env("XDG_CACHE_HOME"), env("HOME"), ledger_root)
        })
        .as_ref()
}

/// The disk cache the values of [`CACHE_DIR_ENV`], `XDG_CACHE_HOME` and `HOME` give, for the ledger in `ledger_root` if
/// any, logging where it is, or why there is none.
pub(super) fn set_up(cache_dir: Option<OsString>, xdg_cache_home: Option<OsString>, home: Option<OsString>, ledger_root: Option<&Path>) -> Option<Cache> {
    let opened = directory(cache_dir, xdg_cache_home, home).and_then(|dir| dir.map(|dir| open(&dir, ledger_root)).transpose());
    match opened {
        Ok(Some(cache)) => {
            info!(
                "plugins keep their compiled code in {} ({CACHE_DIR_ENV}), {} at most",
                cache.directory().display(),
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
                "plugins compile without a disk cache, so each start compiles them anew: {reason}. Set {CACHE_DIR_ENV} to a directory outside the ledger's folder that only the user running zhang can write to, or to nothing to turn the cache off"
            );
            None
        }
    }
}

/// The cache directory, from the values of [`CACHE_DIR_ENV`], `XDG_CACHE_HOME` and `HOME`: none when [`CACHE_DIR_ENV`] is
/// set to nothing (or only spaces), an error when it is not set and there is no home directory to put it in. A relative
/// [`CACHE_DIR_ENV`] is taken from the current directory; a relative `XDG_CACHE_HOME` or `HOME` is ignored, as the XDG
/// base directory specification says.
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
    Ok(Some(base.join("zhang")))
}

/// The disk cache in [`SUBDIRECTORY`] of the cache directory `dir`, keeping at most [`SOFT_LIMIT`], for the ledger in
/// `ledger_root` if any; the error says why it cannot be had. Each directory is created when it is missing, readable by
/// this user only, then checked ([`trusted`]); the cache directory may be reached through symbolic links, its
/// subdirectory may not. The subdirectory is written to once, to see that it can be: wasmtime would fail each write
/// quietly.
fn open(dir: &Path, ledger_root: Option<&Path>) -> Result<Cache, String> {
    // nothing is created in the ledger's folder: checked as written first, then where the links lead
    let ledger = ledger_root.map(|root| {
        let given = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let real = root.canonicalize().unwrap_or_else(|_| given.clone());
        (given, real)
    });
    if let Some((given, real)) = &ledger {
        overlapping(&dir.join(SUBDIRECTORY), given)?;
        overlapping(&dir.join(SUBDIRECTORY), real)?;
    }
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|e| format!("{} cannot be created: {e}", dir.display()))?;
    let dir = dir.canonicalize().map_err(|e| format!("{} cannot be read: {e}", dir.display()))?;
    trusted(&dir, &std::fs::metadata(&dir).map_err(|e| format!("{} cannot be read: {e}", dir.display()))?)?;
    let code = dir.join(SUBDIRECTORY);
    if let Some((_, real)) = &ledger {
        overlapping(&code, real)?;
    }
    builder.recursive(false);
    match builder.create(&code) {
        Err(e) if e.kind() != ErrorKind::AlreadyExists => return Err(format!("{} cannot be created: {e}", code.display())),
        _ => {}
    }
    let metadata = std::fs::symlink_metadata(&code).map_err(|e| format!("{} cannot be read: {e}", code.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("{} is a symbolic link, not a directory of zhang's", code.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", code.display()));
    }
    trusted(&code, &metadata)?;
    // a file of the top level, which wasmtime's clean-up would delete if it were left
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |it| it.as_nanos());
    let probe = code.join(format!(".zhang-probe-{}-{nanos}", std::process::id()));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .and_then(|mut file| file.write_all(b"zhang"));
    if written.is_ok() {
        let _ = std::fs::remove_file(&probe);
    }
    written.map_err(|e| format!("{} cannot be written to: {e}", code.display()))?;
    let mut config = CacheConfig::new();
    config.with_directory(&code).with_files_total_size_soft_limit(SOFT_LIMIT);
    Cache::new(config).map_err(|e| format!("{} cannot be used: {e:#}", code.display()))
}

/// An error when the cache directory `code` lies in the ledger's folder `ledger`, or holds it.
fn overlapping(code: &Path, ledger: &Path) -> Result<(), String> {
    if code.starts_with(ledger) {
        return Err(format!(
            "{} is inside the folder of the ledger, {}: wasmtime's clean-up and the file watcher would act on it",
            code.display(),
            ledger.display()
        ));
    }
    if ledger.starts_with(code) {
        return Err(format!("{} holds the folder of the ledger, {}", code.display(), ledger.display()));
    }
    Ok(())
}

/// An error when `path`, whose metadata is `metadata`, does not belong to the user zhang runs as, or can be written to by
/// its group or by others: whoever can write to the cache can run code as zhang.
#[cfg(unix)]
fn trusted(path: &Path, metadata: &Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and cannot fail
    let user = unsafe { libc::geteuid() };
    if metadata.uid() != user {
        return Err(format!(
            "{} belongs to uid {}, not to uid {user} zhang runs as, so another user could put code into the cache",
            path.display(),
            metadata.uid()
        ));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} can be written to by its group or by others (mode {:o}), who could put code into the cache",
            path.display(),
            metadata.mode() & 0o7777
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn trusted(_path: &Path, _metadata: &Metadata) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{directory, open, set_up, SOFT_LIMIT, SUBDIRECTORY};

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
        assert_eq!(directory(None, os("/xdg"), os("/home/me")), Ok(Some(PathBuf::from("/xdg/zhang"))));
        assert_eq!(directory(None, None, os("/home/me")), Ok(Some(PathBuf::from("/home/me/.cache/zhang"))));
        assert_eq!(
            directory(None, os("relative"), os("/home/me")),
            Ok(Some(PathBuf::from("/home/me/.cache/zhang"))),
            "a relative XDG_CACHE_HOME is ignored"
        );
        assert_eq!(directory(None, os(""), os("/home/me")), Ok(Some(PathBuf::from("/home/me/.cache/zhang"))));
        assert!(directory(None, None, None).unwrap_err().contains("neither XDG_CACHE_HOME nor HOME"));
        assert!(directory(None, None, os("")).is_err(), "no home, no default directory");
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn wasmtime_gets_a_subdirectory_created_for_this_user_only_and_left_clean() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache");
        let cache = open(&dir, None).expect("a directory zhang can create is used");
        let code = dir.join(SUBDIRECTORY);
        assert_eq!(cache.files_total_size_soft_limit(), SOFT_LIMIT);
        assert_eq!(cache.directory(), &code.canonicalize().unwrap(), "never the directory itself");
        assert_eq!(std::fs::read_dir(&code).unwrap().count(), 0, "the probe file is removed");
        #[cfg(unix)]
        assert_eq!((mode(&dir), mode(&code)), (0o700, 0o700));
        assert!(open(&dir, None).is_ok(), "and used again");
        assert!(set_up(Some(dir.into_os_string()), None, None, None).is_some());
    }

    #[test]
    fn a_file_in_the_way_turns_the_cache_off() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cache");
        std::fs::write(&file, "not a directory").unwrap();
        let error = open(&file, None).expect_err("a file is not a directory");
        assert!(error.contains("cannot be created"), "{error}");
        let error = open(&file.join("below"), None).expect_err("nor is a directory below a file");
        assert!(error.contains("cannot be created"), "{error}");
        assert!(set_up(Some(file.clone().into_os_string()), None, None, None).is_none());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not a directory", "the file is left alone");
        // nor is a file in the place of the subdirectory
        let dir = root.path().join("with-a-file");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(SUBDIRECTORY), "not a directory").unwrap();
        let error = open(&dir, None).expect_err("a file is not the subdirectory");
        assert!(error.contains("is not a directory"), "{error}");
    }

    #[test]
    fn a_cache_directory_in_the_ledgers_folder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let ledger = root.path().join("ledger");
        std::fs::create_dir(&ledger).unwrap();
        std::fs::write(ledger.join("main.zhang"), "").unwrap();
        for dir in [ledger.clone(), ledger.join("cache"), ledger.join(".cache").join("zhang")] {
            let error = open(&dir, Some(&ledger)).expect_err("in the ledger's folder");
            assert!(error.contains("inside the folder of the ledger"), "{error}");
            assert!(set_up(Some(dir.clone().into_os_string()), None, None, Some(&ledger)).is_none());
        }
        assert_eq!(std::fs::read_dir(&ledger).unwrap().count(), 1, "nothing is created in the ledger's folder");
        // the default directory too, when the home is the ledger's folder
        assert!(set_up(None, None, Some(ledger.clone().into_os_string()), Some(&ledger)).is_none());
        // reached through a link
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&ledger, &link).unwrap();
            let error = open(&link, Some(&ledger)).expect_err("a link into the ledger's folder");
            assert!(error.contains("inside the folder of the ledger"), "{error}");
        }
        // a cache directory beside it is fine, and so is one holding the ledger's folder: wasmtime has its subdirectory
        assert!(open(&root.path().join("cache"), Some(&ledger)).is_ok());
        assert!(open(root.path(), Some(&ledger)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_others_can_write_to_is_refused() {
        let root = tempfile::tempdir().unwrap();
        for writable in [0o770, 0o707, 0o1777] {
            let dir = root.path().join(format!("{writable:o}"));
            std::fs::create_dir(&dir).unwrap();
            set_mode(&dir, writable);
            let error = open(&dir, None).expect_err("a directory others can write to");
            assert!(error.contains("can be written to by its group or by others"), "{error}");
            assert!(!dir.join(SUBDIRECTORY).exists(), "nothing is created in it");
            // and the subdirectory
            set_mode(&dir, 0o700);
            std::fs::create_dir(dir.join(SUBDIRECTORY)).unwrap();
            set_mode(&dir.join(SUBDIRECTORY), writable);
            let error = open(&dir, None).expect_err("a subdirectory others can write to");
            assert!(error.contains(SUBDIRECTORY) && error.contains("by its group or by others"), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_of_another_user_is_refused() {
        // SAFETY: geteuid has no preconditions
        let user = unsafe { libc::geteuid() };
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("theirs");
        std::fs::create_dir(&dir).unwrap();
        let other = if user == 0 { 65534 } else { 0 };
        if user == 0 {
            std::os::unix::fs::chown(&dir, Some(other), None).unwrap();
        }
        // not root: a directory root owns stands for another user's
        let dir = if user == 0 { dir } else { PathBuf::from("/") };
        let error = open(&dir, None).expect_err("another user's directory");
        assert!(error.contains(&format!("belongs to uid {other}")), "{error}");
        // and the subdirectory, which only root can give to another user
        if user == 0 {
            let mine = root.path().join("mine");
            std::fs::create_dir(&mine).unwrap();
            set_mode(&mine, 0o700);
            std::fs::create_dir(mine.join(SUBDIRECTORY)).unwrap();
            std::os::unix::fs::chown(mine.join(SUBDIRECTORY), Some(other), None).unwrap();
            let error = open(&mine, None).expect_err("a subdirectory of another user");
            assert!(error.contains(SUBDIRECTORY) && error.contains(&format!("belongs to uid {other}")), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_subdirectory_that_is_a_link_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache");
        let elsewhere = root.path().join("elsewhere");
        for it in [&dir, &elsewhere] {
            std::fs::create_dir(it).unwrap();
            set_mode(it, 0o700);
        }
        std::os::unix::fs::symlink(&elsewhere, dir.join(SUBDIRECTORY)).unwrap();
        let error = open(&dir, None).expect_err("a link");
        assert!(error.contains("is a symbolic link"), "{error}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0, "nothing is written where it leads");
    }

    /// whether this process can write to a directory whose mode forbids it, as root can: a read-only directory is no
    /// test then
    fn writes_past_permissions(dir: &Path) -> bool {
        let written = std::fs::write(dir.join("privileged"), "").is_ok();
        let _ = std::fs::remove_file(dir.join("privileged"));
        written
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_turns_the_cache_off() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("read-only");
        std::fs::create_dir(&dir).unwrap();
        set_mode(&dir, 0o500);
        if writes_past_permissions(&dir) {
            eprintln!("skipped: this user writes to read-only directories");
            return;
        }
        let error = open(&dir, None).expect_err("a directory zhang cannot create its subdirectory in");
        assert!(error.contains("cannot be created"), "{error}");
        let below = open(&dir.join("below"), None).expect_err("nor one it cannot create");
        assert!(below.contains("cannot be created"), "{below}");
        assert!(set_up(Some(dir.clone().into_os_string()), None, None, None).is_none());
        // a read-only subdirectory
        set_mode(&dir, 0o700);
        std::fs::create_dir(dir.join(SUBDIRECTORY)).unwrap();
        set_mode(&dir.join(SUBDIRECTORY), 0o500);
        let error = open(&dir, None).expect_err("a subdirectory zhang cannot write to");
        assert!(error.contains("cannot be written to"), "{error}");
        set_mode(&dir.join(SUBDIRECTORY), 0o700);
    }
}
