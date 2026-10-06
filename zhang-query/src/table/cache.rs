//! The per-ledger cache of the tables: what every query of a loaded ledger would otherwise
//! recompute, computed once and kept with the ledger ([`zhang_core::derived::Derived`]).
//!
//! A loaded ledger does not change. zhang replaces it on reload, and the replacement starts with
//! an empty cache, so nothing here is ever stale. What the cache holds depends on the ledger
//! only, never on a query: each part is built by the first query that needs it, and every
//! later query reads the same part, whichever query built it.
//!
//! - [`Entries`]: the rows of `#entries` in the order zhang processed the ledger, which the
//!   `seq` column numbers, the rows of `#transactions`, and for every transaction its id and the
//!   kinds of the errors recorded for it. The load decides all of it for each directive
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
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, FixedOffset, NaiveDate};
use uuid::Uuid;
use zhang_ast::{written_groups, Directive, Transaction};
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_core::utils::id::FromSpan;

use super::lookups::Lookups;
use super::postings;
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
    /// for a transaction, the kinds of the errors zhang recorded for it, when there are any
    pub errors: Option<BTreeSet<String>>,
}

impl Entries {
    /// The dated directives in the order zhang processed them, the `seq` the load gave each, without the
    /// transactions zhang rejected, which are no entries.
    fn build(ledger: &Ledger) -> Entries {
        let mut rows = vec![];
        let mut transactions = vec![];
        for (idx, directive, outcome) in ledger.entries() {
            let seq = outcome.seq.expect("an entry has its place");
            let (txn, errors) = match &outcome.detail {
                Detail::Transaction { id, errors } => {
                    let kinds = errors.iter().map(|error| ledger.errors[*error].error_type.to_string());
                    (Some(*id), Some(kinds.collect::<BTreeSet<_>>()).filter(|kinds| !kinds.is_empty()))
                }
                _ => (None, None),
            };
            if matches!(directive.data, Directive::Transaction(_)) {
                transactions.push(seq);
            }
            rows.push(EntryInfo {
                directive: idx as u32,
                seq,
                txn,
                errors,
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
    /// its row of `#entries` (an index into [`Entries::rows`]): its `seq` and errors
    pub entry: Option<u32>,
    /// its postings as written, by `posting_index`
    pub heads: Box<[Head]>,
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
    fn build(ledger: &Ledger, entries: &Entries) -> Postings {
        let timezone = ledger.options.timezone;
        let mut cached = vec![];
        let mut accounts = Accounts::default();
        let mut rows = Vec::new();
        // the transactions the load accepted, in the order zhang processed them
        for info in &entries.rows {
            let (Some(id), Directive::Transaction(parsed)) = (info.txn, &ledger.directives[info.directive as usize].data) else {
                continue;
            };
            let entry = cached.len();
            let groups = written_groups(&parsed.postings);
            let mut leg = 0;
            let heads = groups
                .iter()
                .map(|group| {
                    // as written, or as booking left it
                    let automatic = group.written.map_or(group.legs[0].units.is_none(), |written| written.units.is_none());
                    let head = Head { leg, automatic };
                    leg += group.legs.len() as u32;
                    head
                })
                .collect();
            let datetime = parsed.date.to_timezone_datetime(&timezone).fixed_offset();
            cached.push(CachedEntry {
                id,
                date: datetime.date_naive(),
                datetime,
                parsed: info.directive,
                entry: Some(info.seq),
                heads,
            });
            rows.extend(postings::booked_rows(entry, &groups, &mut accounts));
        }

        let mut account_rows = vec![vec![]; accounts.names.len()];
        for (idx, row) in rows.iter().enumerate() {
            account_rows[row.account as usize].push(idx as u32);
        }
        Postings {
            entries: cached,
            rows,
            accounts,
            account_rows,
        }
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
