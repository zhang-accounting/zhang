//! The per-ledger cache of the tables: what every query of a loaded ledger would otherwise
//! recompute, computed once and kept with the ledger ([`zhang_core::derived::Derived`]).
//!
//! A loaded ledger does not change. zhang replaces it on reload, and the replacement starts with
//! an empty cache, so nothing here is ever stale. What the cache holds depends on the ledger
//! only, never on a query: each part is built by the first query that needs it, and every
//! later query reads the same part, whichever query built it.
//!
//! - [`Entries`]: the rows of `#entries` in the order zhang processed the ledger, which the
//!   `seq` column numbers, the rows of `#transactions`, and for every transaction its id, and apart
//!   from them the kinds of the errors recorded for it ([`LedgerCache::entry_errors`], the one part
//!   that depends on the ledger's errors). The load decides all of it for each directive
//!   ([`Ledger::outcomes`]), so directives sharing a source position, such as the padding
//!   transactions of a `pad`, one for each commodity it pads, or the copies a plugin emits, each
//!   have their own.
//! - [`Postings`]: the rows of the `postings` table ([`CachedRow`]), one per booked posting of
//!   the ledger's directives (zhang books while it loads, so a sale across several lots is one
//!   posting per lot already), the transactions they belong to, and the rows of every account,
//!   so a query scoped to some accounts ([`super::Scope`]) only visits theirs. A row keeps the
//!   cost of its lot and its price whatever a query projects. A large ledger's are built in
//!   parts, on threads of their own, and kept as the parts that built them ([`Parts`]).
//! - The [`PriceMap`] of the ledger, the ids of the `#entries` rows, and the transactions that
//!   name documents in their metadata (`#documents`).
//!
//! A query then only does the work its own rows need (see [`super::Dataset`]): it keeps the parts
//! of the rows its projection reads. The other tables read the same parts: the directive tables
//! list their rows in the order of [`Entries`], `#balances` and `#budgets` add up the booked rows
//! of their accounts only, and `#documents` reads the transactions that name documents.

use std::collections::{BTreeSet, HashMap};
use std::ops::{Index, Range};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, FixedOffset, NaiveDate};
use rustc_hash::FxBuildHasher;
use uuid::Uuid;
use zhang_ast::{written_groups_into, Date, Directive, Transaction};
use zhang_core::derived::DerivedData;
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_core::utils::id::FromSpan;

use super::lookups::Lookups;
use super::postings;
use crate::extension::NativeSnapshot;
use crate::prices::PriceMap;
use crate::value::Cost;
use crate::Amount;

/// The cache of one loaded ledger; see the module docs.
#[derive(Default)]
pub(crate) struct LedgerCache {
    entries: OnceLock<Entries>,
    postings: OnceLock<Postings>,
    /// shared, so a caller can keep valuing with it after releasing the ledger
    prices: OnceLock<Arc<PriceMap>>,
    /// the `id` of every `#entries` row, by `seq`
    entry_ids: OnceLock<Vec<String>>,
    /// the `seq` of the transactions whose metadata, or the metadata of one of their postings,
    /// names a document, in the order of `#transactions`
    documented: OnceLock<Vec<u32>>,
    /// the account and commodity directives by name
    lookups: OnceLock<Lookups>,
    /// the snapshots of the native providers of the ledger ([`crate::extension`])
    snapshots: OnceLock<Vec<NativeSnapshot>>,
    /// for every `#entries` row, by `seq`, the kinds of the errors recorded for its transaction, when there are any:
    /// the one part of the cache that depends on the ledger's errors ([`DerivedData::errors_changed`])
    entry_errors: OnceLock<Vec<Option<BTreeSet<String>>>>,
}

/// The ledger's analyzers may query it while it loads, before its errors are placed: the load then drops the error
/// kinds of the entries, and keeps everything else, the booked postings first, so the first query after a load does
/// not book the ledger's rows again. The native providers' snapshots are kept too, so a native provider must not read
/// the ledger's errors.
impl DerivedData for LedgerCache {
    fn errors_changed(&mut self) {
        self.entry_errors = OnceLock::new();
    }
}

impl LedgerCache {
    /// The cache kept with `ledger`, made empty on first use.
    pub fn of(ledger: &Ledger) -> &LedgerCache {
        ledger
            .derived
            .get_or_init(LedgerCache::default)
            .expect("the query engine is the only reader keeping data with a ledger")
    }

    pub fn entries(&self, ledger: &Ledger) -> &Entries {
        self.entries.get_or_init(|| Entries::build(ledger))
    }

    pub fn postings(&self, ledger: &Ledger) -> &Postings {
        self.postings.get_or_init(|| Postings::build(ledger, self.entries(ledger)))
    }

    /// The `#entries` rows (their `seq`) of the transactions whose metadata, or the metadata of
    /// one of their postings, has a `document` key, in the order of `#transactions`.
    pub fn documented(&self, ledger: &Ledger) -> &[u32] {
        self.documented.get_or_init(|| {
            let entries = self.entries(ledger);
            let documented = |txn: &Transaction| {
                std::iter::once(&txn.meta)
                    .chain(txn.postings.iter().map(|posting| &posting.meta))
                    .any(|meta| meta.get_one("document").is_some())
            };
            entries
                .transactions
                .iter()
                .copied()
                .filter(|seq| match &ledger.directives[entries.rows[*seq as usize].directive as usize].data {
                    Directive::Transaction(txn) => documented(txn),
                    _ => false,
                })
                .collect()
        })
    }

    /// The kinds of the errors recorded for the transaction of every `#entries` row, by `seq`; `None` for a row
    /// without any, or that is no transaction.
    pub fn entry_errors(&self, ledger: &Ledger) -> &[Option<BTreeSet<String>>] {
        self.entry_errors.get_or_init(|| {
            self.entries(ledger)
                .rows
                .iter()
                .map(|row| match &ledger.outcomes[row.directive as usize].detail {
                    Detail::Transaction { errors, .. } if !errors.is_empty() => {
                        let kinds = errors.iter().map(|error| ledger.errors[*error].error_type.to_string());
                        Some(kinds.collect::<BTreeSet<_>>()).filter(|kinds| !kinds.is_empty())
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// The snapshots of the ledger's native providers, each part built on first use.
    pub fn native_snapshots(&self) -> &[NativeSnapshot] {
        self.snapshots.get_or_init(crate::extension::native_snapshots)
    }

    /// The account and commodity directives of the ledger by name.
    pub fn lookups(&self, ledger: &Ledger) -> &Lookups {
        self.lookups.get_or_init(|| Lookups::build(ledger, self.entries(ledger)))
    }

    pub fn prices(&self, ledger: &Ledger) -> &PriceMap {
        self.shared_prices(ledger)
    }

    /// The price map of [`LedgerCache::prices`], to keep beyond the ledger's lock.
    pub fn shared_prices(&self, ledger: &Ledger) -> &Arc<PriceMap> {
        self.prices.get_or_init(|| Arc::new(PriceMap::for_ledger(ledger)))
    }

    /// The `id` of the `#entries` row `seq`: a transaction has its id (the `id` of its postings),
    /// a checked balance assertion the id of its check (the id `/api/journals` lists it with);
    /// another directive an id derived from its source position, distinct from the ids of the
    /// padding transactions that share the position of a `pad` or a `balance ... with pad`. The
    /// derivation does not depend on the platform's `usize`.
    pub fn entry_id(&self, ledger: &Ledger, entries: &Entries, seq: u32) -> &str {
        let ids = self.entry_ids.get_or_init(|| {
            entries
                .rows
                .iter()
                .map(|entry| match &ledger.outcomes[entry.directive as usize].detail {
                    Detail::Transaction { id, .. } | Detail::Assertion { id, .. } => id.to_string(),
                    _ => {
                        let span = &ledger.directives[entry.directive as usize].span;
                        Uuid::from_txn_posting(&Uuid::from_span(span), u32::MAX as usize).to_string()
                    }
                })
                .collect()
        });
        &ids[seq as usize]
    }
}

/// The rows of `#entries` and `#transactions`.
pub(crate) struct Entries {
    /// the rows of `#entries`, in the order zhang processed the ledger: a row's index is its `seq`
    pub rows: Vec<EntryInfo>,
    /// the rows of `#transactions` (indexes into `rows`), in the order of the ledger's directives, which zhang
    /// processes the transactions in
    pub transactions: Vec<u32>,
}

/// One `#entries` row.
pub(crate) struct EntryInfo {
    /// the index of the directive in [`Ledger::directives`]
    pub directive: u32,
    /// the index of the row in `#entries`, its position in the order zhang processed the ledger:
    /// the `seq` column ([`zhang_core::outcome::Outcome::seq`])
    pub seq: u32,
    /// for a transaction, its id
    pub txn: Option<Uuid>,
}

impl Entries {
    /// The dated directives in the order zhang processed them, the `seq` the load gave each, without the
    /// transactions zhang rejected, which are no entries.
    fn build(ledger: &Ledger) -> Entries {
        let mut rows = vec![];
        let mut transactions = vec![];
        for (idx, directive, outcome) in ledger.entries() {
            let seq = outcome.seq.expect("an entry has its place");
            let txn = match &outcome.detail {
                Detail::Transaction { id, .. } => Some(*id),
                _ => None,
            };
            if matches!(directive.data, Directive::Transaction(_)) {
                transactions.push(seq);
            }
            rows.push(EntryInfo {
                directive: idx as u32,
                seq,
                txn,
            });
        }
        Entries { rows, transactions }
    }
}

/// The booked rows of the `postings` table. Their transactions, the rows and the postings as written are in the parts
/// that built them, the same parts for the three ([`Parts`]).
pub(crate) struct Postings {
    /// the transactions the rows belong to, in ledger order
    pub entries: Parts<CachedEntry>,
    /// the booked rows, in ledger order
    pub rows: Parts<CachedRow>,
    /// the postings as written of every transaction, one after the other in its part ([`CachedEntry::heads`])
    heads: Parts<Head>,
    /// the accounts of the rows
    accounts: Accounts,
    /// the indexes of the rows of every account (by its index in `accounts`), in ledger order
    account_rows: Vec<Vec<u32>>,
}

/// Items built in parts, each on a thread of its own, kept in the parts that built them, one after the other: one
/// list to read, indexed from the first item of the first part. Copied into one vector, they took as much memory again
/// while the copy was made, which the allocator kept: the booked rows of 100,000 transactions, their transactions and
/// heads are about 30 MiB, and a load that books them for an analysis ended 17 MiB higher. A part may be empty.
pub(crate) struct Parts<T> {
    parts: Vec<Vec<T>>,
    /// the index of the first item of each part
    starts: Vec<usize>,
    len: usize,
}

impl<T> Parts<T> {
    /// `parts`, one after the other
    fn new(parts: Vec<Vec<T>>) -> Parts<T> {
        let mut len = 0;
        let starts = parts
            .iter()
            .map(|part| {
                let start = len;
                len += part.len();
                start
            })
            .collect();
        Parts { parts, starts, len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// The items, in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.parts.iter().flatten()
    }

    /// The part of the item `idx` and its index in the part. The last part starting at or before it holds it: an empty
    /// part starts where the next one does.
    fn locate(&self, idx: usize) -> (usize, usize) {
        let part = self.starts.partition_point(|start| *start <= idx).saturating_sub(1);
        (part, idx - self.starts.get(part).copied().unwrap_or_default())
    }

    /// [`Parts::locate`], looking in the part `hint` first: that of the item read before, in a walk of ascending
    /// indexes, which holds most of the next ones.
    fn locate_from(&self, idx: usize, hint: usize) -> (usize, usize) {
        match (self.starts.get(hint), self.parts.get(hint)) {
            (Some(start), Some(part)) if *start <= idx && idx - start < part.len() => (hint, idx - start),
            _ => self.locate(idx),
        }
    }
}

impl<T> Index<usize> for Parts<T> {
    type Output = T;

    fn index(&self, idx: usize) -> &T {
        let (part, at) = self.locate(idx);
        &self.parts[part][at]
    }
}

/// The names of the accounts of the rows, numbered in the order they first appear. Every row looks its account up, so
/// the names are hashed with a fast hash rather than a DoS-resistant one: they come from the ledger, not from a client.
#[derive(Default)]
pub(crate) struct Accounts {
    names: Vec<Box<str>>,
    numbers: HashMap<Box<str>, u32, FxBuildHasher>,
}

impl Accounts {
    /// The number of the account `name`, numbering it when it is new.
    pub fn index(&mut self, name: &str) -> u32 {
        if let Some(number) = self.numbers.get(name) {
            return *number;
        }
        let number = self.names.len() as u32;
        self.names.push(name.into());
        self.numbers.insert(name.into(), number);
        number
    }
}

/// A posting as written of a transaction: the first of the legs booking split it into
/// ([`zhang_ast::written_groups`]).
#[derive(Clone, Copy)]
pub(crate) struct Head {
    /// its index in the transaction's booked postings
    pub leg: u32,
    /// whether it was written without units, which zhang inferred
    pub automatic: bool,
}

/// A transaction of the `postings` table.
pub(crate) struct CachedEntry {
    pub id: Uuid,
    /// its date in the ledger's timezone
    pub date: NaiveDate,
    /// the date and time zhang books it at, in the ledger's timezone
    pub datetime: DateTime<FixedOffset>,
    /// its directive (its index in [`Ledger::directives`]): its metadata, costs and prices are
    /// read from it
    pub parsed: u32,
    /// its row of `#entries` (an index into [`Entries::rows`]): its `seq`, which its errors are found by
    pub entry: Option<u32>,
    /// its postings as written, by `posting_index`: a range of the part of [`Postings::heads`] built with it, read with
    /// [`Postings::heads_of`]
    heads: Range<u32>,
}

/// One booked row of the `postings` table.
pub(crate) struct CachedRow {
    /// the index of its transaction in [`Postings::entries`]
    pub entry: u32,
    /// the index of its posting in the transaction
    pub posting_index: u32,
    /// the number of its account in [`Postings::account_name`]
    pub account: u32,
    /// the units: the posting's, or a part of them when booking split the posting across lots. They are a copy, read
    /// from the rows one after the other, where the units of the directive lie wherever the load put them: a query that
    /// reads the units of every row reads the copies about twice as fast at 100,000 transactions
    pub units: Amount,
    /// the cost and price of the row; most rows have neither
    pub lot: Option<Box<Lot>>,
}

/// The cost and price of a booked row.
pub(crate) struct Lot {
    /// the cost of the lot the row is booked against
    pub cost: Option<Cost>,
    /// the per-unit price annotation of the posting
    pub price: Option<Amount>,
}

/// How many transactions a part of the cache's build has at least: a smaller ledger builds its cache on the thread of
/// its first query only.
const TRANSACTIONS_PER_PART: usize = 16_384;

/// How many threads build the cache of a large ledger at most, the thread of the first query included.
const MAX_PARTS: usize = 4;

/// The rows of a run of the transactions of the `postings` table, built on a thread of its own: the indexes of its
/// transactions count from the ledger's first, those of its heads and accounts from the run's first.
#[derive(Default)]
struct Part {
    entries: Vec<CachedEntry>,
    rows: Vec<CachedRow>,
    heads: Vec<Head>,
    accounts: Accounts,
}

impl Part {
    /// An empty part with room for the rows of `count` transactions of `legs` legs in all: a leg is at most one row, and
    /// a posting as written at least one leg. Its vectors belong to the heap of the thread that calls this, whichever
    /// thread fills them.
    fn with_room(count: usize, legs: usize) -> Part {
        Part {
            entries: Vec::with_capacity(count),
            rows: Vec::with_capacity(legs),
            heads: Vec::with_capacity(legs),
            accounts: Accounts::default(),
        }
    }

    /// `part`, empty ([`Part::with_room`]), with the rows of `transactions`, in their order, the first of them the
    /// transaction `first_entry` of the ledger's.
    fn build<'l>(ledger: &Ledger, first_entry: usize, mut part: Part, transactions: impl Iterator<Item = (&'l EntryInfo, Uuid, &'l Transaction)>) -> Part {
        let timezone = ledger.options.timezone;
        // the date of the transaction before, and when zhang books it: transactions come in date order, so most share
        // the date of the one before, and its conversion to the ledger's timezone
        let mut previous: Option<(&Date, DateTime<FixedOffset>)> = None;
        let mut groups = vec![];
        for (info, id, parsed) in transactions {
            let entry = first_entry + part.entries.len();
            written_groups_into(&parsed.postings, &mut groups);
            let first = part.heads.len() as u32;
            let mut leg = 0;
            part.heads.extend(groups.iter().map(|group| {
                // as written, or as booking left it
                let automatic = group.written.map_or(group.legs[0].units.is_none(), |written| written.units.is_none());
                let head = Head { leg, automatic };
                leg += group.legs.len() as u32;
                head
            }));
            let datetime = match previous {
                Some((date, datetime)) if *date == parsed.date => datetime,
                _ => {
                    let datetime = parsed.date.to_timezone_datetime(&timezone).fixed_offset();
                    previous = Some((&parsed.date, datetime));
                    datetime
                }
            };
            part.entries.push(CachedEntry {
                id,
                date: datetime.date_naive(),
                datetime,
                parsed: info.directive,
                entry: Some(info.seq),
                heads: first..part.heads.len() as u32,
            });
            postings::booked_rows(entry, &groups, &mut part.accounts, &mut part.rows);
        }
        part
    }
}

impl Postings {
    /// The rows of the ledger's transactions, in the order zhang processed them. A large ledger's are built in parts,
    /// each on a thread of its own, and kept in order, so the rows are the same whatever the parts: an account's number
    /// is the order it first appears in over all the rows. The parts' vectors are allocated on this thread, before the
    /// threads fill them: they belong to its heap, which goes on, rather than to those of threads that end, whose
    /// memory the allocator keeps apart once they ended.
    fn build(ledger: &Ledger, entries: &Entries) -> Postings {
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let parts = (entries.transactions.len() / TRANSACTIONS_PER_PART).clamp(1, MAX_PARTS.min(threads));
        Postings::build_in(ledger, entries, parts)
    }

    /// [`Postings::build`] in `parts` parts. One part is built straight from the entries, as a smaller ledger always
    /// is: nothing is listed or renumbered for it.
    fn build_in(ledger: &Ledger, entries: &Entries, parts: usize) -> Postings {
        // the transactions the load accepted, in the order zhang processed them
        let transactions = || {
            entries
                .rows
                .iter()
                .filter_map(|info| match (info.txn, &ledger.directives[info.directive as usize].data) {
                    (Some(id), Directive::Transaction(parsed)) => Some((info, id, parsed)),
                    _ => None,
                })
        };
        if parts <= 1 {
            let (count, legs) = transactions().fold((0, 0), |(count, legs), (_, _, parsed)| (count + 1, legs + parsed.postings.len()));
            return Postings::of(vec![Part::build(ledger, 0, Part::with_room(count, legs), transactions())]);
        }
        // the transactions in runs of `size`, and the legs of each run, counted as they are listed
        let size = entries.transactions.len().div_ceil(parts).max(1);
        let mut legs = vec![];
        let transactions = transactions()
            .enumerate()
            .map(|(at, transaction)| {
                if at % size == 0 {
                    legs.push(0);
                }
                *legs.last_mut().expect("a run") += transaction.2.postings.len();
                transaction
            })
            .collect::<Vec<_>>();
        let room = |run: usize, chunk: &[_]| Part::with_room(chunk.len(), legs[run]);
        if transactions.len() < parts {
            let room = Part::with_room(transactions.len(), legs.iter().sum());
            return Postings::of(vec![Part::build(ledger, 0, room, transactions.iter().copied())]);
        }
        let chunks = transactions
            .chunks(size)
            .enumerate()
            .map(|(run, chunk)| (run, room(run, chunk), chunk))
            .collect::<Vec<_>>();
        let built = std::thread::scope(|scope| {
            // the first part is built on this thread; a part whose thread cannot start is built here too
            let mut chunks = chunks.into_iter();
            let (_, part, chunk) = chunks.next().expect("a part");
            let started = chunks
                .map(|(run, part, chunk)| {
                    let build = move || Part::build(ledger, run * size, part, chunk.iter().copied());
                    // a thread that does not start drops its job, and the room made for it
                    std::thread::Builder::new().spawn_scoped(scope, build).map_err(|_| (run, chunk))
                })
                .collect::<Vec<_>>();
            let mut built = vec![Part::build(ledger, 0, part, chunk.iter().copied())];
            for part in started {
                built.push(match part {
                    Ok(thread) => thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                    Err((run, chunk)) => Part::build(ledger, run * size, room(run, chunk), chunk.iter().copied()),
                });
            }
            built
        });
        Postings::of(built)
    }

    /// The postings of the whole ledger, `parts` one after the other, with the rows of every account. The accounts of
    /// the rows are numbered over all the parts, in the order they first appear, in place.
    fn of(mut parts: Vec<Part>) -> Postings {
        let mut accounts = Accounts::default();
        let mut sizes = vec![];
        for part in &mut parts {
            let numbers = part.accounts.names.iter().map(|name| accounts.index(name)).collect::<Vec<_>>();
            sizes.resize(accounts.names.len(), 0);
            for row in &mut part.rows {
                row.account = numbers[row.account as usize];
                sizes[row.account as usize] += 1;
            }
        }
        let mut account_rows = sizes.into_iter().map(Vec::with_capacity).collect::<Vec<_>>();
        for (idx, row) in parts.iter().flat_map(|part| &part.rows).enumerate() {
            account_rows[row.account as usize].push(idx as u32);
        }
        let (mut entries, mut rows, mut heads) = (vec![], vec![], vec![]);
        for part in parts {
            entries.push(part.entries);
            rows.push(part.rows);
            heads.push(part.heads);
        }
        Postings {
            entries: Parts::new(entries),
            rows: Parts::new(rows),
            heads: Parts::new(heads),
            accounts,
            account_rows,
        }
    }

    /// The rows at `indexes` (into [`Postings::rows`]), each with its transaction and the transaction's postings as
    /// written, by `posting_index`. Indexes that ascend, as those of a scope or an account do, are read part after part.
    pub fn at(&self, indexes: impl Iterator<Item = usize>) -> impl Iterator<Item = (&CachedRow, &CachedEntry, &[Head])> {
        let mut part = 0;
        indexes.map(move |idx| {
            let at;
            (part, at) = self.rows.locate_from(idx, part);
            let row = &self.rows.parts[part][at];
            // a row's transaction is in the part built with it
            let entry = &self.entries.parts[part][row.entry as usize - self.entries.starts[part]];
            (row, entry, &self.heads.parts[part][entry.heads.start as usize..entry.heads.end as usize])
        })
    }

    /// The name of the account with this number.
    pub fn account_name(&self, account: u32) -> &str {
        &self.accounts.names[account as usize]
    }

    /// The names of the accounts that have rows.
    pub fn account_names(&self) -> impl Iterator<Item = &str> {
        self.accounts.names.iter().map(AsRef::as_ref)
    }

    /// The indexes of the rows of `account`, in ledger order.
    pub fn account_rows(&self, account: &str) -> &[u32] {
        match self.accounts.numbers.get(account) {
            Some(number) => &self.account_rows[*number as usize],
            None => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Entries, LedgerCache, Parts, Postings};

    /// Parts read as one list, the empty ones included, wherever they are.
    #[test]
    fn parts_read_as_one_list() {
        let parts = Parts::new(vec![vec![], vec![1, 2], vec![], vec![], vec![3], vec![4, 5, 6], vec![]]);
        assert_eq!(parts.len(), 6);
        assert_eq!(parts.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert_eq!((0..6).map(|idx| parts[idx]).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert!(std::panic::catch_unwind(|| parts[6]).is_err(), "past the last item");
        assert_eq!(Parts::<u32>::new(vec![vec![], vec![]]).len(), 0);
        assert_eq!(Parts::<u32>::new(vec![]).len(), 0);
    }

    /// A description of everything the cache of the booked postings holds, to compare two builds of it.
    fn described(postings: &Postings) -> Vec<String> {
        let mut lines = postings
            .account_names()
            .map(|name| format!("account {name}: {:?}", postings.account_rows(name)))
            .collect::<Vec<_>>();
        let heads_of = |at: usize| {
            let (part, at) = postings.entries.locate(at);
            let heads = &postings.entries.parts[part][at].heads;
            &postings.heads.parts[part][heads.start as usize..heads.end as usize]
        };
        for (at, entry) in postings.entries.iter().enumerate() {
            let heads = heads_of(at).iter().map(|head| (head.leg, head.automatic)).collect::<Vec<_>>();
            lines.push(format!(
                "entry {at}: {} {} {} {} {:?} {heads:?}",
                entry.id, entry.date, entry.datetime, entry.parsed, entry.entry
            ));
        }
        for row in postings.rows.iter() {
            let lot = row
                .lot
                .as_ref()
                .map(|lot| format!("{:?} {:?}", lot.cost.as_ref().map(ToString::to_string), lot.price));
            lines.push(format!(
                "row {} {} {} {} {lot:?}",
                row.entry,
                row.posting_index,
                postings.account_name(row.account),
                row.units
            ));
        }
        lines
    }

    #[test]
    fn a_cache_built_in_parts_is_the_cache_built_whole() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
        let source = zhang_core::data_source::LocalFileSystemDataSource::new(zhang_core::data_type::text::ZhangDataType {});
        let ledger = zhang_core::ledger::Ledger::load_with_data_source(dir, "main.zhang".to_owned(), std::sync::Arc::new(source)).expect("the ledger loads");
        let entries = LedgerCache::of(&ledger).entries(&ledger);
        let whole = described(&Postings::build_in(&ledger, entries, 1));
        assert!(whole.len() > 1000, "{}", whole.len());
        for parts in [2, 3, 7] {
            assert_eq!(described(&Postings::build_in(&ledger, entries, parts)), whole, "{parts} parts");
        }
        // more parts than transactions
        let empty = Entries {
            rows: vec![],
            transactions: vec![],
        };
        assert_eq!(Postings::build_in(&ledger, &empty, 4).rows.len(), 0);
    }
}
