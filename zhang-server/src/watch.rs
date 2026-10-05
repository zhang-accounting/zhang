//! When the served ledger is stale: a filesystem event that touches something the load read, or, for a ledger
//! that depends on the date, the next local midnight.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;
use indexmap::IndexSet;
use notify::{Event, EventKind};
use zhang_ast::resolve_local_datetime;
use zhang_core::constants::CACHE_DIR;
use zhang_core::data_source::path_in_ledger;
use zhang_core::inputs::ExtraInput;
use zhang_core::utils::has_path_visited;

/// zhang's own state under the root (the registered passkeys), which the server writes itself, so a change there is
/// never a change to an input either
const STATE_DIR: &str = crate::auth::STATE_DIR;

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

/// whether `event` makes the ledger stale:
/// - a modification of one of the ledger's own files (`visited_files`)
/// - creating, modifying or removing a [`ExtraInput::File`]
/// - any change under an [`ExtraInput::Dir`], including a file created in it; reading is not a change
///
/// `roots` are the spellings of the ledger root from [`watch_roots`]; extra inputs are relative to it
pub fn should_reload(event: &Event, roots: &[PathBuf], visited_files: &[PathBuf], extra_inputs: &IndexSet<ExtraInput>) -> bool {
    event.paths.iter().any(|path| {
        let is_visited_file_modified = event.kind.is_modify() && has_path_visited(visited_files, path);
        is_visited_file_modified
            || relative_to_root(path, roots).is_some_and(|relative| extra_inputs.iter().any(|input| is_changed_by(input, &event.kind, &relative)))
    })
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

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use indexmap::IndexSet;
    use notify::event::{AccessKind, CreateKind, DataChange, ModifyKind, RemoveKind};
    use notify::{Event, EventKind};
    use zhang_core::inputs::ExtraInput;

    use super::should_reload;

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

    #[test]
    fn should_not_reload_without_extra_inputs() {
        let event = Event::new(CREATE).add_path(PathBuf::from("/var/ledger/documents/receipt.pdf"));
        assert!(!should_reload(&event, &roots(), &visited_files(), &IndexSet::new()));
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
