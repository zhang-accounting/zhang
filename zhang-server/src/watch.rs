//! When the served ledger is stale: a filesystem event that touches something the load read, or, for a ledger
//! that depends on the date, the next local midnight. And what the file watcher does while it cannot watch the
//! ledger's folder: it says why, checks the ledger's files every [`POLL_INTERVAL`] ([`changes`]) and tries again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode};
use notify::{Event, EventKind};
use zhang_ast::resolve_local_datetime;
use zhang_core::constants::CACHE_DIR;
use zhang_core::data_source::path_in_ledger;
use zhang_core::inputs::ExtraInput;
use zhang_core::utils::has_path_visited;

/// zhang's own state under the root (the registered passkeys), which the server writes itself, so a change there is
/// never a change to an input either
const STATE_DIR: &str = crate::auth::STATE_DIR;

/// how long the watcher waits for no other change before it looks at the changes it gathered ([`gather`])
pub const QUIET: Duration = Duration::from_millis(500);

/// the longest the watcher gathers changes, from the first, however busy the files are ([`gather`])
pub const MAX_WAIT: Duration = Duration::from_secs(2);

/// the wait before the first attempt to watch the ledger's folder again after the watcher failed, doubled after every
/// attempt up to [`MAX_RETRY`]
pub const FIRST_RETRY: Duration = Duration::from_secs(1);

/// the longest wait between two attempts to watch the ledger's folder
pub const MAX_RETRY: Duration = Duration::from_secs(30);

/// how often the ledger's files are checked for changes while the folder is not watched ([`changes`])
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// whether a report of the watcher may be a change, to gather: not a read (an access), which inotify reports for every
/// file a load opens, and which changes nothing; an error may be anything
pub fn is_change(event: &notify::Result<Event>) -> bool {
    !matches!(event, Ok(event) if event.kind.is_access())
}

/// The reports of the watcher, gathered: the first one, waited for, then every other until none came for `quiet`, or
/// until `max_wait` passed since the first, so that a burst of writes, or an editor saving a file in steps, is looked
/// at once, and a file that never stops changing still is, every `max_wait`. `None` once the watcher is gone.
pub async fn gather<T>(reports: &mut tokio::sync::mpsc::Receiver<T>, quiet: Duration, max_wait: Duration) -> Option<Vec<T>> {
    let mut gathered = vec![reports.recv().await?];
    let longest = tokio::time::Instant::now() + max_wait;
    loop {
        let until = (tokio::time::Instant::now() + quiet).min(longest);
        match tokio::time::timeout_at(until, reports.recv()).await {
            Ok(Some(report)) => gathered.push(report),
            // quiet, or waited long enough; a watcher gone leaves what it reported to look at, and `None` next time
            Ok(None) | Err(_) => return Some(gathered),
        }
    }
}

/// the ledger root as event paths spell it: canonical (macOS reports canonical paths) and absolute (other
/// platforms join the watched path onto the working directory)
pub fn watch_roots(entry: &Path) -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Ok(canonical) = entry.canonicalize() {
        roots.push(canonical);
    }
    let absolute = match std::env::current_dir() {
        Ok(cwd) if entry.is_relative() => cwd.join(entry),
        _ => entry.to_path_buf(),
    };
    if !roots.contains(&absolute) {
        roots.push(absolute);
    }
    roots
}

/// What an event changed of what the load of the served ledger read, the most first
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// one of the ledger's own files, modified: it may hold what the ledger served was loaded from still, as after a
    /// write of the server, which reloaded the ledger itself ([`zhang_core::ledger::Ledger::files_unchanged`])
    LedgerFiles,
    /// something else the load read ([`ExtraInput`]): the ledger is stale
    Inputs,
}

/// what `event` changed ([`Change`]), `None` when it makes the ledger no staler:
/// - a modification of one of the ledger's own files (`visited_files`) is [`Change::LedgerFiles`]
/// - creating, modifying or removing a [`ExtraInput::File`], and any change under an [`ExtraInput::Dir`], including a
///   file created in it, is [`Change::Inputs`]; reading is not a change
///
/// `roots` are the spellings of the ledger root from [`watch_roots`]; extra inputs are relative to it
pub fn change(event: &Event, roots: &[PathBuf], visited_files: &[PathBuf], extra_inputs: &IndexSet<ExtraInput>) -> Option<Change> {
    let changes = event.paths.iter().filter_map(|path| {
        let is_input_changed =
            relative_to_root(path, roots).is_some_and(|relative| extra_inputs.iter().any(|input| is_changed_by(input, &event.kind, &relative)));
        let is_visited_file_modified = event.kind.is_modify() && has_path_visited(visited_files, path);
        match (is_input_changed, is_visited_file_modified) {
            (true, _) => Some(Change::Inputs),
            (false, true) => Some(Change::LedgerFiles),
            (false, false) => None,
        }
    });
    changes.max()
}

/// whether `event` makes the ledger stale, or may: it changed something the load read ([`change`])
pub fn should_reload(event: &Event, roots: &[PathBuf], visited_files: &[PathBuf], extra_inputs: &IndexSet<ExtraInput>) -> bool {
    change(event, roots, visited_files, extra_inputs).is_some()
}

/// `path`, a path on the disk an event names, relative to the ledger root; `None` outside the root and in zhang's own
/// cache and state. Serving from the root puts the cache ([`CACHE_DIR`], relative to the working directory) under it,
/// and every load rewrites the plugin modules there, so a change in it is never a change to an input. A relative
/// `path` is relative to no root
fn relative_to_root(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|_| path.is_absolute())
        .find_map(|root| path_in_ledger(root, path))
        .filter(|relative| !relative.starts_with(CACHE_DIR) && !relative.starts_with(STATE_DIR))
}

/// whether an event of `kind` on `path`, relative to the ledger root, changes `input`
fn is_changed_by(input: &ExtraInput, kind: &EventKind, path: &Path) -> bool {
    match input {
        ExtraInput::File(file) => (kind.is_create() || kind.is_modify() || kind.is_remove()) && path == file,
        ExtraInput::Dir(dir) => !kind.is_access() && path.starts_with(dir),
        ExtraInput::Clock => false,
    }
}

/// the instant the local date in `timezone` next changes after `now`: the next midnight, the earlier one when a DST
/// change repeats it, or the first minute of the day when a DST jump skips midnight
pub fn next_local_midnight(now: DateTime<Utc>, timezone: Tz) -> DateTime<Utc> {
    match now.with_timezone(&timezone).date_naive().succ_opt() {
        // the one reading of a local time every date of the ledger gets: the earlier of a repeated midnight, the end
        // of the gap of a skipped one
        Some(tomorrow) => resolve_local_datetime(&timezone, &tomorrow.and_time(NaiveTime::MIN)).with_timezone(&Utc),
        None => now + TimeDelta::days(1),
    }
}

/// `EMFILE`, "Too many open files", the error of `inotify_init` once the user's inotify instances
/// (`fs.inotify.max_user_instances`) or the process's open files are used up
const TOO_MANY_OPEN_FILES: i32 = 24;

/// Why the watcher cannot watch the ledger's folder, from the `error` it failed with: a limit of inotify names the
/// setting to raise
pub fn why_unwatched(error: &notify::Error) -> String {
    match &error.kind {
        notify::ErrorKind::MaxFilesWatch => format!(
            "{error}: the inotify watches of the user the server runs as are used up, one per folder watched; raise \
             fs.inotify.max_user_watches (`sysctl -w fs.inotify.max_user_watches=524288`)"
        ),
        notify::ErrorKind::Io(io) if cfg!(unix) && io.raw_os_error() == Some(TOO_MANY_OPEN_FILES) => format!(
            "{error}: the inotify instances of the user the server runs as, or the server's open files, are used up; raise \
             fs.inotify.max_user_instances (`sysctl -w fs.inotify.max_user_instances=1024`), or the open-file limit (`ulimit -n`)"
        ),
        notify::ErrorKind::PathNotFound => format!("{error}: the ledger's folder, or a folder in it, is gone"),
        _ => error.to_string(),
    }
}

/// What the server tells, logged once and as `watch_failure` of `/api/info`, while it cannot watch the ledger's folder
/// `root` for `why` ([`why_unwatched`])
pub fn unwatched(root: &Path, why: &str, poll_interval: Duration, max_retry: Duration) -> String {
    format!(
        "cannot watch {} for edits made outside the server: {why}. Until it can, the server checks the ledger's files \
         for changes every {poll_interval:?}, and tries to watch the folder again every {max_retry:?} at most",
        root.display()
    )
}

/// Why the watcher no longer watches the whole ledger, after it reported `events`, `None` while it does: it could not
/// watch a folder created under the root (`MaxFilesWatch`, the inotify watch limit), or the root, one of `roots`, was
/// removed or moved away, after which it watches nothing the ledger is read from
pub fn watch_lost(events: &[notify::Result<Event>], roots: &[PathBuf]) -> Option<String> {
    events.iter().find_map(|event| match event {
        Err(error) if matches!(error.kind, notify::ErrorKind::MaxFilesWatch) => Some(why_unwatched(error)),
        Ok(event)
            if matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)))
                && event.paths.iter().any(|path| roots.contains(path)) =>
        {
            Some("the ledger's folder was removed or moved away".to_owned())
        }
        _ => None,
    })
}

/// What the poll compares of a file ([`changes`]): when it was modified last and its length, `None` while there is no
/// file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp(Option<(Option<SystemTime>, u64)>);

/// The paths the poll checks while the ledger's folder is not watched: the ledger's own files (`visited_files`) and
/// what the load read besides (`extra_inputs`, relative to `root`). A folder among them is checked for files added to
/// it or removed from it, the changes its modification time shows
pub fn polled_paths(root: &Path, visited_files: &[PathBuf], extra_inputs: &IndexSet<ExtraInput>) -> Vec<PathBuf> {
    let inputs = extra_inputs.iter().filter_map(|input| match input {
        ExtraInput::File(path) | ExtraInput::Dir(path) => Some(root.join(path)),
        ExtraInput::Clock => None,
    });
    visited_files.iter().cloned().chain(inputs).collect()
}

/// the stamps of `paths`, read now: a blocking read of their metadata
pub fn stamps(paths: Vec<PathBuf>) -> HashMap<PathBuf, Stamp> {
    paths
        .into_iter()
        .map(|path| {
            let stamp = std::fs::metadata(&path).ok().map(|metadata| (metadata.modified().ok(), metadata.len()));
            (path, Stamp(stamp))
        })
        .collect()
}

/// What changed between two polls of the ledger's files (`before`, `now`), as the watcher reports it: a file created,
/// modified or removed. A path the earlier poll did not check, one the reload in between added, is no change
pub fn changes(before: &HashMap<PathBuf, Stamp>, now: &HashMap<PathBuf, Stamp>) -> Vec<notify::Result<Event>> {
    now.iter()
        .filter_map(|(path, stamp)| {
            let kind = match (before.get(path)?.0, stamp.0) {
                (old, new) if old == new => return None,
                (None, Some(_)) => EventKind::Create(CreateKind::Any),
                (Some(_), None) => EventKind::Remove(RemoveKind::Any),
                _ => EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            };
            Some(Ok(Event::new(kind).add_path(path.clone())))
        })
        .collect()
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::time::Duration;

    use indexmap::IndexSet;
    use notify::event::{AccessKind, CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode};
    use notify::{Event, EventKind};
    use tokio::time::Instant;
    use zhang_core::inputs::ExtraInput;

    use super::{change, changes, gather, is_change, polled_paths, should_reload, stamps, watch_lost, why_unwatched, Change};

    const MODIFY: EventKind = EventKind::Modify(ModifyKind::Data(DataChange::Content));
    const CREATE: EventKind = EventKind::Create(CreateKind::File);
    const REMOVE: EventKind = EventKind::Remove(RemoveKind::File);
    const ACCESS: EventKind = EventKind::Access(AccessKind::Read);

    /// the ledger root, spelled canonically and through a symlink
    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/private/var/ledger"), PathBuf::from("/var/ledger")]
    }

    fn visited_files() -> Vec<PathBuf> {
        vec![PathBuf::from("/var/ledger/main.zhang"), PathBuf::from("/var/ledger/data/2024.zhang")]
    }

    fn inputs() -> IndexSet<ExtraInput> {
        IndexSet::from([
            ExtraInput::File("plugins/fx.wasm".into()),
            ExtraInput::Dir("documents".into()),
            ExtraInput::Clock,
        ])
    }

    fn reloads(kind: EventKind, path: &str) -> bool {
        let event = Event::new(kind).add_path(PathBuf::from(path));
        should_reload(&event, &roots(), &visited_files(), &inputs())
    }

    #[test]
    fn should_reload_when_a_ledger_file_is_modified_only() {
        assert!(reloads(MODIFY, "/var/ledger/main.zhang"));
        assert!(reloads(MODIFY, "/var/ledger/data/2024.zhang"));
        // as before, only a modification counts for the ledger's own files
        assert!(!reloads(CREATE, "/var/ledger/main.zhang"));
        assert!(!reloads(REMOVE, "/var/ledger/main.zhang"));
        assert!(!reloads(MODIFY, "/var/ledger/other.zhang"));
    }

    #[test]
    fn should_reload_when_a_file_input_is_created_modified_or_removed() {
        for kind in [MODIFY, CREATE, REMOVE, EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To))] {
            assert!(reloads(kind, "/var/ledger/plugins/fx.wasm"), "{kind:?}");
            assert!(reloads(kind, "/private/var/ledger/plugins/fx.wasm"), "{kind:?} on the canonical root");
        }
        assert!(!reloads(ACCESS, "/var/ledger/plugins/fx.wasm"));
        assert!(!reloads(EventKind::Other, "/var/ledger/plugins/fx.wasm"));
    }

    #[test]
    fn should_match_a_file_input_exactly() {
        assert!(!reloads(MODIFY, "/var/ledger/plugins/fx.wasm.tmp"));
        assert!(!reloads(MODIFY, "/var/ledger/plugins/other.wasm"));
        assert!(!reloads(MODIFY, "/var/ledger/plugins"));
        assert!(!reloads(MODIFY, "/var/ledger/fx.wasm"));
    }

    #[test]
    fn should_reload_on_any_change_under_a_dir_input() {
        for kind in [MODIFY, CREATE, REMOVE, EventKind::Any, EventKind::Other] {
            assert!(reloads(kind, "/var/ledger/documents/2024/receipt.pdf"), "{kind:?}");
            assert!(reloads(kind, "/var/ledger/documents"), "{kind:?} on the dir itself");
        }
        // reading a file is not a change
        assert!(!reloads(ACCESS, "/var/ledger/documents/receipt.pdf"));
        // prefixes are matched by component
        assert!(!reloads(CREATE, "/var/ledger/documents-private/receipt.pdf"));
        assert!(!reloads(CREATE, "/var/ledger/receipt.pdf"));
    }

    #[test]
    fn should_reload_on_any_change_under_the_root_as_a_dir_input() {
        let event = Event::new(CREATE).add_path(PathBuf::from("/var/ledger/anything/new.txt"));
        let root = IndexSet::from([ExtraInput::Dir(PathBuf::new())]);
        assert!(should_reload(&event, &roots(), &[], &root));
    }

    #[test]
    fn should_ignore_zhangs_own_plugin_cache() {
        let event = Event::new(MODIFY).add_path(PathBuf::from("/var/ledger/.cache/plugins/0123.wasm"));
        let root = IndexSet::from([ExtraInput::Dir(PathBuf::new())]);
        assert!(!should_reload(&event, &roots(), &[], &root));
    }

    #[test]
    fn should_ignore_zhangs_own_state() {
        let root = IndexSet::from([ExtraInput::Dir(PathBuf::new())]);
        for path in [
            "/var/ledger/.zhang/passkeys.json",
            "/private/var/ledger/.zhang/passkeys.json",
            "/var/ledger/.zhang",
        ] {
            for kind in [MODIFY, CREATE, REMOVE] {
                let event = Event::new(kind).add_path(PathBuf::from(path));
                assert!(!should_reload(&event, &roots(), &visited_files(), &root), "{kind:?} {path}");
                assert!(!should_reload(&event, &roots(), &visited_files(), &inputs()), "{kind:?} {path}");
            }
        }
        // only the state dir itself, matched by component
        let event = Event::new(CREATE).add_path(PathBuf::from("/var/ledger/.zhang-notes/todo.txt"));
        assert!(should_reload(&event, &roots(), &visited_files(), &root));
    }

    #[test]
    fn should_not_reload_for_paths_outside_the_root() {
        assert!(!reloads(MODIFY, "/elsewhere/plugins/fx.wasm"));
        assert!(!reloads(CREATE, "/elsewhere/documents/receipt.pdf"));
        assert!(!reloads(CREATE, "/var/ledger-old/documents/receipt.pdf"));
        assert!(!reloads(MODIFY, "plugins/fx.wasm"));
    }

    #[test]
    fn should_reload_when_any_path_of_an_event_matches() {
        // a rename reports both paths
        let event = Event::new(EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)))
            .add_path(PathBuf::from("/var/ledger/plugins/fx.wasm.tmp"))
            .add_path(PathBuf::from("/var/ledger/plugins/fx.wasm"));
        assert!(should_reload(&event, &roots(), &visited_files(), &inputs()));
    }

    /// a read is no change; anything else may be, an error too
    #[test]
    fn a_read_is_no_change() {
        assert!(!is_change(&Ok(Event::new(ACCESS).add_path(PathBuf::from("/var/ledger/main.zhang")))));
        let opened = EventKind::Access(AccessKind::Open(notify::event::AccessMode::Read));
        assert!(!is_change(&Ok(Event::new(opened))));
        let closed = EventKind::Access(AccessKind::Close(notify::event::AccessMode::Read));
        assert!(!is_change(&Ok(Event::new(closed))));
        assert!(is_change(&Ok(Event::new(MODIFY))));
        assert!(is_change(&Ok(Event::new(CREATE))));
        assert!(is_change(&Err(notify::Error::generic("lost"))));
    }

    const QUIET: Duration = Duration::from_millis(500);
    const MAX_WAIT: Duration = Duration::from_secs(2);

    /// changes coming closer together than the quiet period are gathered at once, looked at a quiet period after the
    /// last
    #[tokio::test(start_paused = true)]
    async fn changes_are_gathered_until_a_quiet_moment() {
        let (sender, mut reports) = tokio::sync::mpsc::channel(16);
        let started = Instant::now();
        tokio::spawn(async move {
            for report in 1..=3 {
                sender.send(report).await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            // later than the quiet period: the next gathering
            tokio::time::sleep(Duration::from_millis(500)).await;
            sender.send(4).await.unwrap();
        });
        assert_eq!(gather(&mut reports, QUIET, MAX_WAIT).await, Some(vec![1, 2, 3]));
        // the last at 600 ms, then a quiet period
        assert_eq!(started.elapsed(), Duration::from_millis(1100));
        assert_eq!(gather(&mut reports, QUIET, MAX_WAIT).await, Some(vec![4]));
        assert_eq!(gather(&mut reports, QUIET, MAX_WAIT).await, None, "the watcher is gone");
    }

    /// files that never stop changing, as under writes back to back, are looked at every longest wait
    #[tokio::test(start_paused = true)]
    async fn changes_that_never_stop_are_gathered_for_the_longest_wait() {
        let (sender, mut reports) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            for report in 0.. {
                if sender.send(report).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        let started = Instant::now();
        let first = gather(&mut reports, QUIET, MAX_WAIT).await.unwrap();
        assert_eq!(started.elapsed(), MAX_WAIT);
        // one at once, then one every 100 ms for 2 s, the one at 2 s in this gathering or the next
        assert!((20..=21).contains(&first.len()), "{first:?}");
        let second = gather(&mut reports, QUIET, MAX_WAIT).await.unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= MAX_WAIT * 2 && elapsed <= MAX_WAIT * 2 + Duration::from_millis(100), "{elapsed:?}");
        assert_eq!(second[0], first.last().unwrap() + 1, "nothing is lost between two gatherings");
    }

    /// A modification of the ledger's own files may leave them as loaded; any change to another input does not
    #[test]
    fn a_modification_of_the_ledgers_files_alone_is_a_change_of_the_ledger_files() {
        let modify = |path: &str| Event::new(MODIFY).add_path(PathBuf::from(path));
        let changes = |event: &Event| change(event, &roots(), &visited_files(), &inputs());
        assert_eq!(changes(&modify("/var/ledger/main.zhang")), Some(Change::LedgerFiles));
        assert_eq!(changes(&modify("/var/ledger/data/2024.zhang")), Some(Change::LedgerFiles));
        assert_eq!(changes(&modify("/var/ledger/plugins/fx.wasm")), Some(Change::Inputs));
        assert_eq!(changes(&Event::new(CREATE).add_path(PathBuf::from("/var/ledger/main.zhang"))), None);
        // an event naming both, as a rename does: the input decides
        let both = Event::new(EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)))
            .add_path(PathBuf::from("/var/ledger/main.zhang"))
            .add_path(PathBuf::from("/var/ledger/plugins/fx.wasm"));
        assert_eq!(changes(&both), Some(Change::Inputs));
        // a ledger file under a directory input, the root here, is an input
        let root = IndexSet::from([ExtraInput::Dir(PathBuf::new())]);
        assert_eq!(
            change(&modify("/var/ledger/main.zhang"), &roots(), &visited_files(), &root),
            Some(Change::Inputs)
        );
    }

    #[test]
    fn should_not_reload_without_extra_inputs() {
        let event = Event::new(CREATE).add_path(PathBuf::from("/var/ledger/documents/receipt.pdf"));
        assert!(!should_reload(&event, &roots(), &visited_files(), &IndexSet::new()));
    }

    /// a limit of inotify names the setting to raise
    #[test]
    fn why_a_folder_is_not_watched_names_the_inotify_limit_to_raise() {
        let instances = why_unwatched(&notify::Error::io(std::io::Error::from_raw_os_error(24)));
        assert!(instances.contains("fs.inotify.max_user_instances"), "{instances}");
        let watches = why_unwatched(&notify::Error::new(notify::ErrorKind::MaxFilesWatch));
        assert!(watches.contains("fs.inotify.max_user_watches"), "{watches}");
        let gone = why_unwatched(&notify::Error::path_not_found());
        assert!(gone.contains("gone") && !gone.contains("fs.inotify"), "{gone}");
    }

    /// the watcher watches the whole ledger no more once it could not watch a folder created in it, or once the root
    /// is gone; any other report, an error too, leaves it watching
    #[test]
    fn a_watch_is_lost_on_the_watch_limit_or_without_its_root() {
        let report = |kind: EventKind, path: &str| Ok(Event::new(kind).add_path(PathBuf::from(path)));
        let limit = Err(notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path(PathBuf::from("/var/ledger/new")));
        assert!(watch_lost(&[limit], &roots()).is_some_and(|why| why.contains("fs.inotify.max_user_watches")));
        for kind in [
            REMOVE,
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        ] {
            assert!(
                watch_lost(&[report(MODIFY, "/var/ledger/main.zhang"), report(kind, "/var/ledger")], &roots()).is_some(),
                "{kind:?}"
            );
            assert!(
                watch_lost(&[report(kind, "/private/var/ledger")], &roots()).is_some(),
                "{kind:?} of the canonical root"
            );
            assert!(
                watch_lost(&[report(kind, "/var/ledger/data")], &roots()).is_none(),
                "{kind:?} of a folder in it"
            );
        }
        assert!(watch_lost(&[report(MODIFY, "/var/ledger")], &roots()).is_none());
        assert!(watch_lost(&[Err(notify::Error::generic("lost"))], &roots()).is_none());
    }

    /// The poll reports what changed between two polls, as the watcher would: a file modified, created or removed
    #[test]
    fn the_poll_reports_files_modified_created_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (main, plugin, documents) = (root.join("main.zhang"), root.join("plugins/fx.wasm"), root.join("documents"));
        std::fs::write(&main, "1970-01-01 open Assets:A\n").unwrap();
        let inputs = IndexSet::from([
            ExtraInput::File("plugins/fx.wasm".into()),
            ExtraInput::Dir("documents".into()),
            ExtraInput::Clock,
        ]);
        let paths = polled_paths(&root, std::slice::from_ref(&main), &inputs);
        assert_eq!(paths, vec![main.clone(), plugin.clone(), documents.clone()]);
        let before = stamps(paths.clone());
        assert!(changes(&before, &stamps(paths.clone())).is_empty(), "nothing changed");

        // a write of the same length a moment later: its modification time tells
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(&main, "1970-01-01 open Assets:B\n").unwrap();
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(&plugin, "module").unwrap();
        let now = stamps(paths.clone());
        let mut changed: Vec<_> = changes(&before, &now)
            .into_iter()
            .map(|event| event.unwrap())
            .map(|event| (event.kind, event.paths))
            .collect();
        changed.sort_by_key(|(_, paths)| paths.clone());
        assert_eq!(
            changed,
            vec![
                (EventKind::Modify(ModifyKind::Data(DataChange::Any)), vec![main.clone()]),
                (EventKind::Create(CreateKind::Any), vec![plugin.clone()]),
            ]
        );

        std::fs::remove_file(&plugin).unwrap();
        let removed = changes(&now, &stamps(paths.clone()));
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert_eq!(removed[0].as_ref().unwrap().kind, EventKind::Remove(RemoveKind::Any));
        // a path the reload in between added is no change
        let added = root.join("data.zhang");
        std::fs::write(&added, "").unwrap();
        assert!(changes(&now, &stamps(vec![main, added])).is_empty());
    }

    mod midnight {
        use chrono::{DateTime, Utc};
        use chrono_tz::Tz;

        use crate::watch::next_local_midnight;

        fn next(now: &str, timezone: &str) -> DateTime<Utc> {
            next_local_midnight(now.parse().unwrap(), timezone.parse::<Tz>().unwrap())
        }

        fn utc(text: &str) -> DateTime<Utc> {
            text.parse().unwrap()
        }

        #[test]
        fn should_be_the_next_local_midnight() {
            // 2024-03-10 13:00 in Shanghai (UTC+8)
            assert_eq!(next("2024-03-10T05:00:00Z", "Asia/Shanghai"), utc("2024-03-10T16:00:00Z"));
            // the local date is already 03-11 although the UTC date is 03-10
            assert_eq!(next("2024-03-10T17:00:00Z", "Asia/Shanghai"), utc("2024-03-11T16:00:00Z"));
            assert_eq!(next("2024-03-10T05:00:00Z", "UTC"), utc("2024-03-11T00:00:00Z"));
        }

        #[test]
        fn should_be_strictly_after_now() {
            // exactly midnight: the next one is a day later
            assert_eq!(next("2024-03-10T16:00:00Z", "Asia/Shanghai"), utc("2024-03-11T16:00:00Z"));
            // a second before midnight
            assert_eq!(next("2024-03-10T15:59:59Z", "Asia/Shanghai"), utc("2024-03-10T16:00:00Z"));
        }

        #[test]
        fn should_follow_dst_changes_during_the_day() {
            // New York springs forward at 02:00 on 2024-03-10: that day lasts 23 hours
            assert_eq!(next("2024-03-10T05:00:00Z", "America/New_York"), utc("2024-03-11T04:00:00Z"));
            // and falls back at 02:00 on 2024-11-03: that day lasts 25 hours
            assert_eq!(next("2024-11-03T04:00:00Z", "America/New_York"), utc("2024-11-04T05:00:00Z"));
        }

        #[test]
        fn should_take_the_first_minute_of_a_day_without_midnight() {
            // Santiago springs forward at midnight on 2023-09-03: 00:00 -04 is 01:00 -03
            assert_eq!(next("2023-09-02T12:00:00Z", "America/Santiago"), utc("2023-09-03T04:00:00Z"));
        }

        #[test]
        fn should_take_the_earlier_of_a_repeated_midnight() {
            // Havana falls back from 01:00 CDT to 00:00 CST on 2023-11-05, so midnight happens twice
            assert_eq!(next("2023-11-04T16:00:00Z", "America/Havana"), utc("2023-11-05T04:00:00Z"));
            // from inside the repeated hour the date stays 11-05 until the next midnight
            assert_eq!(next("2023-11-05T05:30:00Z", "America/Havana"), utc("2023-11-06T05:00:00Z"));
        }
    }
}
