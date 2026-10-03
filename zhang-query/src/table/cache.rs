//! The per-ledger cache of the tables: what every query of a loaded ledger would otherwise
//! recompute, computed once and kept with the ledger ([`zhang_core::derived::Derived`]).
//!
//! A loaded ledger does not change. zhang replaces it on reload, and the replacement starts with
//! an empty cache, so nothing here is ever stale. What the cache holds depends on the ledger
//! only, never on a query: each part is built by the first query that needs it, and every
//! later query reads the same part, whichever query built it.
//!
//! - [`Entries`]: the rows of `#entries` in ledger order (their position is the `seq` column),
//!   the rows of `#transactions`, and for every transaction its stored id and the kinds of the
//!   errors recorded for it. Stored transactions are found by source position, which is how
//!   zhang derives their ids, so no id is hashed.
//! - [`Postings`]: the booked rows of the `postings` table ([`CachedRow`]), the transactions
//!   they belong to, and the rows of every account, so a query scoped to some accounts
//!   ([`super::Scope`]) only visits theirs. Booking runs here, over every posting at cost; a
//!   row keeps the cost of its lot and its price whatever a query projects.
//! - The [`PriceMap`] of the ledger, the ids of the `#entries` rows, and the transactions that
//!   name documents in their metadata (`#documents`).
//!
//! A query then only does the work its own rows need (see [`super::Dataset`]): it finds the
//! stored transactions of the rows it reads, and keeps the parts of the rows its projection
//! reads. The other tables read the same parts: the directive tables list their rows in the
//! order of [`Entries`], `#balances` and `#budgets` add up the booked rows of their accounts
//! only, and `#documents` reads the transactions that name documents.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::OnceLock;

use chrono::NaiveDate;
use uuid::Uuid;
use zhang_ast::{Directive, Flag, SpanInfo, Transaction};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;
use zhang_core::utils::id::FromSpan;

use super::directives::{date_of, day_rank};
use super::postings;
use crate::error::LocatedError;
use crate::prices::PriceMap;
use crate::value::Cost;
use crate::Amount;

/// "none" in the index vectors of the cache
const NONE: u32 = u32::MAX;

/// The cache of one loaded ledger; see the module docs.
pub(crate) struct LedgerCache {
    /// the size of the ledger the cache was made for
    fingerprint: Fingerprint,
    entries: OnceLock<Entries>,
    postings: OnceLock<Postings>,
    prices: OnceLock<PriceMap>,
    /// the `id` of every `#entries` row, by `seq`
    entry_ids: OnceLock<Vec<String>>,
    /// the `seq` of the transactions whose metadata, or the metadata of one of their postings,
    /// names a document, in the order of `#transactions`
    documented: OnceLock<Vec<u32>>,
}

/// How many directives, transactions, postings, prices, errors and metadata a ledger holds: a
/// cheap check that the ledger did not change since its cache was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint([usize; 6]);

impl Fingerprint {
    fn of(ledger: &Ledger, store: &Store) -> Fingerprint {
        Fingerprint([
            ledger.directives.len(),
            store.transactions.len(),
            store.postings.len(),
            store.prices.len(),
            store.errors.len(),
            store.metas.len(),
        ])
    }
}

impl LedgerCache {
    /// An empty cache for the ledger as it is now.
    pub fn new(ledger: &Ledger, store: &Store) -> LedgerCache {
        LedgerCache {
            fingerprint: Fingerprint::of(ledger, store),
            entries: OnceLock::new(),
            postings: OnceLock::new(),
            prices: OnceLock::new(),
            entry_ids: OnceLock::new(),
            documented: OnceLock::new(),
        }
    }

    /// The cache kept with `ledger`, made empty on first use; `store` is the ledger's store.
    pub fn of<'a>(ledger: &'a Ledger, store: &Store) -> &'a LedgerCache {
        ledger
            .derived
            .get_or_init(|| LedgerCache::new(ledger, store))
            .expect("the query engine is the only reader keeping data with a ledger")
    }

    /// An error when the ledger changed since the cache was made. zhang never changes a loaded
    /// ledger (a reload replaces it), so this only catches code that writes to the store of a
    /// ledger it already queried.
    pub fn check(&self, ledger: &Ledger, store: &Store) -> Result<(), LocatedError> {
        if self.fingerprint == Fingerprint::of(ledger, store) {
            Ok(())
        } else {
            Err(LocatedError::eval(
                "the ledger changed after it was first queried; reload it instead of changing it",
                None,
            ))
        }
    }

    pub fn entries(&self, ledger: &Ledger, store: &Store) -> &Entries {
        self.entries.get_or_init(|| Entries::build(ledger, store))
    }

    pub fn postings(&self, ledger: &Ledger, store: &Store) -> &Postings {
        self.postings.get_or_init(|| Postings::build(ledger, store, self.entries(ledger, store)))
    }

    /// The `#entries` rows (their `seq`) of the transactions whose metadata, or the metadata of
    /// one of their postings, has a `document` key, in the order of `#transactions`.
    pub fn documented(&self, ledger: &Ledger, store: &Store) -> &[u32] {
        self.documented.get_or_init(|| {
            let entries = self.entries(ledger, store);
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

    pub fn prices(&self, store: &Store) -> &PriceMap {
        self.prices.get_or_init(|| PriceMap::from_prices(&store.prices))
    }

    /// The `id` of the `#entries` row `seq`: a transaction has its stored id (the `id` of its
    /// postings); another directive an id derived from its source position, distinct from the
    /// id of the padding transaction that shares the position of a `balance ... with pad`.
    /// The derivation does not depend on the platform's `usize`.
    pub fn entry_id(&self, ledger: &Ledger, entries: &Entries, seq: u32) -> &str {
        let ids = self.entry_ids.get_or_init(|| {
            entries
                .rows
                .iter()
                .map(|entry| match entry.txn {
                    Some(id) => id.to_string(),
                    None => {
                        let span = &ledger.directives[entry.directive as usize].span;
                        Uuid::from_txn_posting(&Uuid::from_span(span), u32::MAX as usize).to_string()
                    }
                })
                .collect()
        });
        &ids[seq as usize]
    }
}

/// A source position as two integers (see [`Positions`]).
type Position = (Option<usize>, usize);

/// Source positions as hashable integers: file names are numbered once, so a position hashes as
/// two integers instead of a path. Paths are compared as strings, as zhang derives ids from them
/// (comparing or hashing a `Path` walks its components).
#[derive(Default)]
struct Positions<'a> {
    files: HashMap<&'a OsStr, usize>,
    /// the file last numbered or looked up, as consecutive spans usually share their file
    last: Option<(&'a OsStr, usize)>,
}

impl<'a> Positions<'a> {
    /// The position of `span`, numbering its file when `add` is set; `None` for a file that
    /// was never numbered.
    fn of(&mut self, span: &'a SpanInfo, add: bool) -> Option<Position> {
        let Some(path) = span.filename.as_deref().map(Path::as_os_str) else {
            return Some((None, span.start));
        };
        if let Some((last, number)) = self.last {
            if last == path {
                return Some((Some(number), span.start));
            }
        }
        let number = match self.files.get(path) {
            Some(number) => *number,
            None if add => {
                let number = self.files.len();
                self.files.insert(path, number);
                number
            }
            None => return None,
        };
        self.last = Some((path, number));
        Some((Some(number), span.start))
    }
}

/// The rows of `#entries` and `#transactions`.
pub(crate) struct Entries {
    /// the rows of `#entries`, in ledger order: a row's index is its `seq`
    pub rows: Vec<EntryInfo>,
    /// the rows of `#transactions` (indexes into `rows`), in the order of the ledger's directives
    pub transactions: Vec<u32>,
    /// directive index → index into `rows` ([`NONE`] for a directive that is not an entry)
    of_directive: Vec<u32>,
}

/// One `#entries` row.
pub(crate) struct EntryInfo {
    /// the index of the directive in [`Ledger::directives`]
    pub directive: u32,
    /// the position in `#entries`
    pub seq: u32,
    /// for a transaction, its id in the store
    pub txn: Option<Uuid>,
    /// for a transaction, the kinds of the errors zhang recorded for it, when there are any
    pub errors: Option<BTreeSet<String>>,
}

impl Entries {
    /// The dated directives in beancount's order: by date, then by [`day_rank`], then in the
    /// order of the ledger's directives (which follows the source within a day for directives
    /// without a time), without the correcting transactions of balance assertions and the
    /// transactions zhang rejected, which never reached the store.
    fn build(ledger: &Ledger, store: &Store) -> Entries {
        let directives = &ledger.directives;
        let mut positions = Positions::default();
        for directive in directives {
            positions.of(&directive.span, true);
        }
        // the stored transactions, which zhang identifies by the position of their directive
        let mut stored: HashMap<Position, Uuid> = HashMap::with_capacity(store.transactions.len());
        for txn in store.transactions.values() {
            if let Some(position) = positions.of(&txn.span, false) {
                stored.insert(position, txn.id);
            }
        }
        // what zhang records about a directive, it records at its position
        let mut errors: HashMap<Position, BTreeSet<String>> = HashMap::new();
        for error in &store.errors {
            if let Some(position) = error.span.as_ref().and_then(|span| positions.of(span, false)) {
                errors.entry(position).or_default().insert(error.error_type.to_string());
            }
        }

        // (sort key, index), sorted stably by key
        let mut order = directives
            .iter()
            .enumerate()
            .map(|(idx, directive)| ((date_of(&directive.data), day_rank(&directive.data)), idx))
            .collect::<Vec<_>>();
        order.sort_by_key(|(key, _)| *key);
        let order = order.into_iter().map(|(_, idx)| idx);
        let mut rows = Vec::with_capacity(order.len());
        let mut of_directive = vec![NONE; directives.len()];
        for idx in order {
            let directive = &directives[idx];
            let position = positions.of(&directive.span, false);
            let txn = match &directive.data {
                Directive::Transaction(txn) => match position.and_then(|position| stored.get(&position)) {
                    Some(id) if txn.flag != Some(Flag::BalanceCheck) => Some(*id),
                    _ => continue,
                },
                _ => None,
            };
            let seq = rows.len() as u32;
            of_directive[idx] = seq;
            rows.push(EntryInfo {
                directive: idx as u32,
                seq,
                txn,
                errors: txn.and(position).and_then(|position| errors.remove(&position)),
            });
        }
        let transactions = (0..directives.len())
            .filter(|idx| of_directive[*idx] != NONE && matches!(directives[*idx].data, Directive::Transaction(_)))
            .map(|idx| of_directive[idx])
            .collect();
        Entries {
            rows,
            transactions,
            of_directive,
        }
    }

    /// The `#entries` row of the directive with this index.
    pub fn of_directive(&self, idx: usize) -> Option<&EntryInfo> {
        self.of_directive.get(idx).filter(|it| **it != NONE).map(|it| &self.rows[*it as usize])
    }
}

/// The booked rows of the `postings` table.
pub(crate) struct Postings {
    /// the stored transactions the rows belong to, without the correcting transactions of
    /// balance assertions, in ledger order (by their sequence in the store)
    pub entries: Vec<CachedEntry>,
    /// the sequence of a stored transaction → its index in `entries` ([`NONE`] if not there)
    entry_of_sequence: Vec<u32>,
    /// the booked rows, in ledger order
    pub rows: Vec<CachedRow>,
    /// the accounts of the rows
    accounts: Accounts,
    /// the indexes of the rows of every account (by its index in `accounts`), in ledger order
    account_rows: Vec<Vec<u32>>,
}

/// The names of the accounts of the rows, numbered in the order they first appear.
#[derive(Default)]
pub(crate) struct Accounts {
    names: Vec<Box<str>>,
    numbers: HashMap<Box<str>, u32>,
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

/// A transaction of the `postings` table.
pub(crate) struct CachedEntry {
    pub id: Uuid,
    /// its date in the ledger's timezone
    pub date: NaiveDate,
    /// the parsed directive (its index in [`Ledger::directives`]) when its postings match the
    /// stored ones: its metadata, costs and prices are read from it
    pub parsed: Option<u32>,
    /// its row of `#entries` (an index into [`Entries::rows`]): its `seq` and errors
    pub entry: Option<u32>,
}

/// One booked row of the `postings` table.
pub(crate) struct CachedRow {
    /// the index of its transaction in [`Postings::entries`]
    pub entry: u32,
    /// the index of its posting in the transaction
    pub posting_index: u32,
    /// the number of its account in [`Postings::account_name`]
    pub account: u32,
    /// the units: the posting's, or a part of them when booking split the posting across lots
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

impl Postings {
    fn build(ledger: &Ledger, store: &Store, entries: &Entries) -> Postings {
        // the parsed transaction directives by position; the last directive of a position wins
        let mut positions = Positions::default();
        let mut parsed_at: HashMap<Position, usize> = HashMap::new();
        for (idx, directive) in ledger.directives.iter().enumerate() {
            if let Directive::Transaction(_) = &directive.data {
                if let Some(position) = positions.of(&directive.span, true) {
                    parsed_at.insert(position, idx);
                }
            }
        }

        // in ledger order; the sequence is copied next to the transaction to sort on it
        let mut transactions = store
            .transactions
            .values()
            .filter(|txn| txn.flag != Flag::BalanceCheck)
            .map(|txn| (txn.sequence, txn))
            .collect::<Vec<_>>();
        transactions.sort_unstable_by_key(|(sequence, _)| *sequence);

        let max_sequence = transactions.last().map(|(sequence, _)| *sequence).unwrap_or_default();
        let mut entry_of_sequence = vec![NONE; usize::try_from(max_sequence).unwrap_or_default() + 1];
        let mut cached = Vec::with_capacity(transactions.len());
        let mut accounts = Accounts::default();
        let mut drafts = Vec::with_capacity(store.postings.len());
        for (_, txn) in transactions {
            let directive = positions.of(&txn.span, false).and_then(|position| parsed_at.get(&position)).copied();
            let parsed: Option<&Transaction> = directive
                .and_then(|idx| match &ledger.directives[idx].data {
                    Directive::Transaction(parsed) => Some(parsed),
                    _ => None,
                })
                .filter(|parsed| parsed.postings.len() == txn.postings.len());
            let entry = cached.len();
            if let Ok(sequence) = usize::try_from(txn.sequence) {
                entry_of_sequence[sequence] = entry as u32;
            }
            cached.push(CachedEntry {
                id: txn.id,
                date: txn.datetime.date_naive(),
                parsed: parsed.and(directive).map(|idx| idx as u32),
                entry: directive.and_then(|idx| entries.of_directive(idx)).map(|it| it.seq),
            });
            drafts.extend(postings::drafts(entry, txn, parsed, &mut accounts));
        }

        let rows = postings::book(drafts, ledger, store);
        let mut account_rows = vec![vec![]; accounts.names.len()];
        for (idx, row) in rows.iter().enumerate() {
            account_rows[row.account as usize].push(idx as u32);
        }
        Postings {
            entries: cached,
            entry_of_sequence,
            rows,
            accounts,
            account_rows,
        }
    }

    /// The index in [`Postings::entries`] of the stored transaction with this sequence.
    pub fn entry_of_sequence(&self, sequence: i32) -> Option<usize> {
        let idx = *self.entry_of_sequence.get(usize::try_from(sequence).ok()?)?;
        (idx != NONE).then_some(idx as usize)
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
