//! The transactions the server deleted or replaced lately, by their id and their version, so that a write naming one
//! of them is refused instead of writing another transaction that took its place.
//!
//! A version is the hash of a transaction's text, and an id is derived from its place: when a transaction is deleted,
//! the next one in its file can take its id, and when that one is written alike (two identical entries, back to
//! back), its version too. A delete sent again with the id and the version of the first, as a client retries one whose
//! answer it lost, named the twin, and deleted it; an update sent at once with a delete edited it. A tombstone tells
//! the request apart: the id and the version it names are those of a transaction the server removed lately, and the
//! request is refused with a 412.
//!
//! They are kept in memory, for a day and at most a thousand of them: after a restart of the server, twins are told
//! apart no more. A transaction the server writes itself, whose id and version a create or an update answers, is no
//! tombstone, even when it is written alike at a place one was removed from.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// how many tombstones are kept, the oldest dropped first
const MAX_TOMBSTONES: usize = 1000;

/// how long a tombstone is kept
const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// The `(id, version)` of the transactions the server deleted or replaced lately, each with when.
#[derive(Debug, Default)]
pub struct Tombstones {
    entries: VecDeque<(Uuid, String, Instant)>,
}

impl Tombstones {
    /// remember that the transaction `id` of the version `version` was deleted or replaced now
    pub fn add(&mut self, id: Uuid, version: String) {
        self.remove(id, &version);
        self.entries.push_back((id, version, Instant::now()));
        while self.entries.len() > MAX_TOMBSTONES {
            self.entries.pop_front();
        }
    }

    /// forget the tombstone of `(id, version)`: the server wrote a transaction it answers under it
    pub fn remove(&mut self, id: Uuid, version: &str) {
        self.entries.retain(|(it, its, _)| !(*it == id && its == version));
    }

    /// whether the transaction `id` of the version `version` was deleted or replaced lately
    pub fn contains(&mut self, id: Uuid, version: &str) -> bool {
        let now = Instant::now();
        while self.entries.front().is_some_and(|(_, _, at)| now.duration_since(*at) > LIFETIME) {
            self.entries.pop_front();
        }
        self.entries.iter().any(|(it, its, _)| *it == id && its == version)
    }
}

#[cfg(test)]
mod test {
    use uuid::Uuid;

    use super::{Tombstones, MAX_TOMBSTONES};

    #[test]
    fn a_tombstone_is_kept_until_it_is_removed_or_a_thousand_others_come() {
        let mut tombstones = Tombstones::default();
        let id = Uuid::new_v4();
        tombstones.add(id, "v1".to_owned());
        assert!(tombstones.contains(id, "v1"));
        assert!(!tombstones.contains(id, "v2") && !tombstones.contains(Uuid::new_v4(), "v1"));
        tombstones.remove(id, "v1");
        assert!(!tombstones.contains(id, "v1"));

        tombstones.add(id, "v1".to_owned());
        for _ in 0..MAX_TOMBSTONES {
            tombstones.add(Uuid::new_v4(), "v".to_owned());
        }
        assert!(!tombstones.contains(id, "v1"), "the oldest is dropped");
    }
}
