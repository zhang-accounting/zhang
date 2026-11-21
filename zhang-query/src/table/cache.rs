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
//!   cost of its lot and its price whatever a query projects.
//! - The [`PriceMap`] of the ledger, the ids of the `#entries` rows, and the transactions that
//!   name documents in their metadata (`#documents`).
//!
//! A query then only does the work its own rows need (see [`super::Dataset`]): it keeps the parts
//! of the rows its projection reads. The other tables read the same parts: the directive tables
//! list their rows in the order of [`Entries`], `#balances` and `#budgets` add up the booked rows
//! of their accounts only, and `#documents` reads the transactions that name documents.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
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

/// The booked rows of the `postings` table.
pub(crate) struct Postings {
    /// the transactions the rows belong to, in ledger order
    pub entries: Vec<CachedEntry>,
    /// the booked rows, in ledger order
    pub rows: Vec<CachedRow>,
    /// the postings as written of every transaction, one after the other ([`CachedEntry::heads`])
    heads: Vec<Head>,
    /// the accounts of the rows
    accounts: Accounts,
    /// the indexes of the rows of every account (by its index in `accounts`), in ledger order
    account_rows: Vec<Vec<u32>>,
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
    /// its postings as written, by `posting_index`: a range of [`Postings::heads`], read with [`Postings::heads_of`]
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

impl Postings {
    fn build(ledger: &Ledger, entries: &Entries) -> Postings {
        let timezone = ledger.options.timezone;
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
        // a leg is at most one row, and a posting as written at least one leg
        let legs = transactions().map(|(_, _, parsed)| parsed.postings.len()).sum();
        let mut cached = Vec::with_capacity(entries.transactions.len());
        let mut rows = Vec::with_capacity(legs);
        let mut heads = Vec::with_capacity(legs);
        let mut accounts = Accounts::default();
        // the date of the transaction before, and when zhang books it: transactions come in date order, so most share
        // the date of the one before, and its conversion to the ledger's timezone
        let mut previous: Option<(&Date, DateTime<FixedOffset>)> = None;
        let mut groups = vec![];
        for (info, id, parsed) in transactions() {
            let entry = cached.len();
            written_groups_into(&parsed.postings, &mut groups);
            let first = heads.len() as u32;
            let mut leg = 0;
            heads.extend(groups.iter().map(|group| {
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
            cached.push(CachedEntry {
                id,
                date: datetime.date_naive(),
                datetime,
                parsed: info.directive,
                entry: Some(info.seq),
                heads: first..heads.len() as u32,
            });
            postings::booked_rows(entry, &groups, &mut accounts, &mut rows);
        }

        let mut sizes = vec![0; accounts.names.len()];
        for row in &rows {
            sizes[row.account as usize] += 1;
        }
        let mut account_rows = sizes.into_iter().map(Vec::with_capacity).collect::<Vec<_>>();
        for (idx, row) in rows.iter().enumerate() {
            account_rows[row.account as usize].push(idx as u32);
        }
        Postings {
            entries: cached,
            rows,
            heads,
            accounts,
            account_rows,
        }
    }

    /// The postings as written of the transaction `entry`, by `posting_index`.
    pub fn heads_of(&self, entry: &CachedEntry) -> &[Head] {
        &self.heads[entry.heads.start as usize..entry.heads.end as usize]
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
