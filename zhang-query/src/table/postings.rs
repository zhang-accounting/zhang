//! The `postings` table: one row per posting, with the columns of its transaction.
//!
//! Rows are read directly from the ledger's in-memory [`Store`] (computed units, pads)
//! and enriched from the parsed transaction directives where the store does not keep the
//! information (price annotations, lot date and label, transaction metadata).
//!
//! Which entries produce rows follows beancount: transactions and padding transactions
//! (flag `P`) do; balance assertions, which book nothing, do not.
//!
//! Lot booking runs once per loaded ledger, over every posting ([`book`], kept in the
//! [`LedgerCache`]). A query then assembles its rows from the booked ones: only those of the
//! accounts it is scoped to ([`Scope`]), each keeping only the parts its projected columns read
//! (see [`crate::projector`]). Columns of the transaction are read from the store on access,
//! never copied up front.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};

use bigdecimal::{BigDecimal, Signed, Zero};
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Meta, Posting, PostingCost, SingleTotalPrice};
use zhang_core::domains::schemas::MetaType;
use zhang_core::inventory::BookingMethod;
use zhang_core::ledger::Ledger;
use zhang_core::store::{PostingMetaDomain, Store, TransactionDomain};

use super::cache::{Accounts, CachedRow, LedgerCache, Lot, Postings};
use super::{render_pairs, Borrow, ColumnDef, Dataset, Get, Reads, POSTINGS};
use crate::decimal;
use crate::functions::is_under;
use crate::projector::Projection;
use crate::value::{Cost, DataType, Inventory, Position, Value};

/// A part of a row that it borrows from the ledger or its cache, or owns: like a `Cow`, but an
/// owned value is boxed, which keeps rows small. Only the synthetic rows of the period
/// modifiers own theirs.
pub(crate) enum MaybeOwned<'a, T> {
    Borrowed(&'a T),
    Owned(Box<T>),
}

impl<T> std::ops::Deref for MaybeOwned<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        match self {
            MaybeOwned::Borrowed(it) => it,
            MaybeOwned::Owned(it) => it,
        }
    }
}

impl<T> AsRef<T> for MaybeOwned<'_, T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> MaybeOwned<'_, T> {
    pub fn owned(value: T) -> Self {
        MaybeOwned::Owned(Box::new(value))
    }
}

/// The transaction-level part of a row.
pub(crate) struct Entry<'a> {
    /// the stored transaction; its id, flag, payee, narration, tags, links and posting
    /// accounts are read from it when a column needs them. It is owned only for the synthetic
    /// entries of the period modifiers (see [`crate::period`]).
    pub txn: MaybeOwned<'a, TransactionDomain>,
    pub date: NaiveDate,
    /// metadata from the parsed directive, when it could be matched
    pub meta: Option<&'a Meta>,
    /// the position of the transaction in `#entries`; `None` for a synthetic entry
    pub seq: Option<u32>,
    /// the kinds of the errors zhang recorded for the transaction, when there are any
    pub errors: Option<&'a BTreeSet<String>>,
}

/// One posting.
pub(crate) struct Row<'a> {
    pub entry: usize,
    pub posting_index: usize,
    pub account: &'a str,
    pub units: MaybeOwned<'a, Amount>,
    /// the cost of the booked lot; only kept when the projection reads it
    /// ([`Projection::keeps_cost`]), otherwise always `None`
    pub cost: Option<MaybeOwned<'a, Cost>>,
    /// per-unit price annotation; only kept when the projection reads it
    /// ([`Projection::keeps_price`]), otherwise always `None`
    pub price: Option<MaybeOwned<'a, Amount>>,
}

/// The rows of the `postings` table an execution needs: every row, or only the rows of some
/// accounts, when the query's filter can only hold for them (see
/// [`crate::optimizer::account_scope`]). A scope keeps every row of its accounts, in ledger
/// order, so the running balances of the rows it keeps are the same.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Scope {
    #[default]
    All,
    Accounts {
        /// these accounts
        exact: BTreeSet<String>,
        /// and these accounts with their sub-accounts
        subtrees: Vec<String>,
    },
}

impl Scope {
    /// The indexes of the booked rows of the scope, in ledger order; `None` for every row.
    fn rows<'c>(&self, postings: &'c Postings) -> Option<Cow<'c, [u32]>> {
        let Scope::Accounts { exact, subtrees } = self else {
            return None;
        };
        let mut lists = exact.iter().map(|account| postings.account_rows(account)).collect::<Vec<_>>();
        if !subtrees.is_empty() {
            let under = postings
                .account_names()
                .filter(|account| !exact.contains(*account) && subtrees.iter().any(|ancestor| is_under(account, ancestor)));
            lists.extend(under.map(|account| postings.account_rows(account)));
        }
        lists.retain(|rows| !rows.is_empty());
        Some(match lists.as_slice() {
            [] => Cow::Borrowed(&[]),
            [rows] => Cow::Borrowed(rows),
            _ => {
                // the accounts' rows are disjoint
                let mut rows = lists.concat();
                rows.sort_unstable();
                Cow::Owned(rows)
            }
        })
    }
}

/// The cost specification (`{...}`) of a posting before lot booking.
///
/// As in beancount, what a spec means depends on the side of the posting (see [`book`]): on
/// a reduction its given fields are criteria matched against the open lots and the missing
/// ones are wildcards; on an augmentation it describes the new lot.
pub(super) struct CostSpec {
    /// per-unit cost; `None` for `{}` and for specs with only a date or a label
    per_unit: Option<Amount>,
    date: Option<NaiveDate>,
    label: Option<String>,
}

impl CostSpec {
    /// Whether the open lot `lot` satisfies this spec as a reduction criterion.
    fn matches(&self, lot: &Cost) -> bool {
        self.per_unit
            .as_ref()
            .is_none_or(|cost| cost.number == lot.number && cost.commodity == lot.currency)
            && self.date.is_none_or(|date| lot.date == Some(date))
            && self.label.as_ref().is_none_or(|label| lot.label.as_ref() == Some(label))
    }
}

/// A posting before booking.
pub(super) struct Draft<'a> {
    entry: u32,
    posting_index: u32,
    /// date of the transaction
    date: NaiveDate,
    account: &'a str,
    /// the account's index in the [`Accounts`] of the cache
    account_index: u32,
    units: &'a Amount,
    cost: Option<CostSpec>,
    /// the per-unit price annotation
    price: Option<Amount>,
}

/// The postings of the stored transaction `txn`, the entry `entry` of the table, before
/// booking; `parsed` is its directive's postings as written, when it could be matched.
pub(super) fn drafts<'a, 'c>(
    entry: usize, txn: &'a TransactionDomain, parsed: Option<Vec<Posting>>, accounts: &'c mut Accounts,
) -> impl Iterator<Item = Draft<'a>> + use<'a, 'c> {
    let date = txn.datetime.date_naive();
    txn.postings.iter().enumerate().map(move |(posting_index, posting)| {
        let units = &posting.inferred_amount;
        let parsed_posting = parsed.as_ref().and_then(|it| it.get(posting_index));
        let cost = match parsed_posting {
            Some(parsed_posting) => parsed_posting.cost.as_ref().map(|cost| cost_spec(cost, units)),
            // without the parsed directive only the cost number kept by the store is known
            None => posting.cost.as_ref().map(|cost| CostSpec {
                per_unit: Some(cost.clone()),
                date: None,
                label: None,
            }),
        };
        Draft {
            entry: entry as u32,
            posting_index: posting_index as u32,
            date,
            account: posting.account.name(),
            account_index: accounts.index(posting.account.name()),
            units,
            cost,
            price: parsed_posting.and_then(|it| it.price.as_ref()).and_then(|price| per_unit_price(price, units)),
        }
    })
}

impl<'a> Dataset<'a> {
    /// Every row of the `postings` table, for `projection`.
    #[cfg(test)]
    pub fn new(ledger: &'a Ledger, store: &'a Store, today: NaiveDate, projection: Projection) -> Self {
        Dataset::postings(ledger, store, LedgerCache::of(ledger, store), today, projection, &Scope::All)
    }

    /// The rows of the `postings` table in `scope`, for `projection`, assembled from the booked
    /// rows of `cache` (the cache of `ledger`).
    pub fn postings(ledger: &'a Ledger, store: &'a Store, cache: &'a LedgerCache, today: NaiveDate, projection: Projection, scope: &Scope) -> Self {
        let postings = cache.postings(ledger, store);
        let table = cache.entries(ledger, store);
        let selected = scope.rows(postings);

        // the stored transactions of the rows: all of them, found in one scan of the store, or
        // the few of a scope, looked up by id
        let scanned = selected.is_none().then(|| {
            let mut transactions: Vec<Option<&'a TransactionDomain>> = vec![None; postings.entries.len()];
            for txn in store.transactions.values() {
                if let Some(idx) = postings.entry_of_sequence(txn.sequence).filter(|idx| postings.entries[*idx].id == txn.id) {
                    transactions[idx] = Some(txn);
                }
            }
            transactions
        });
        let transaction = |idx: usize| match &scanned {
            Some(transactions) => transactions[idx],
            None => store.transactions.get(&postings.entries[idx].id),
        };

        let (keep_cost, keep_price) = (projection.keeps_cost(), projection.keeps_price());
        let count = selected.as_ref().map_or(postings.rows.len(), |rows| rows.len());
        let mut entries: Vec<Entry<'a>> = Vec::with_capacity(if selected.is_none() { postings.entries.len() } else { 0 });
        let mut rows = Vec::with_capacity(count);
        // (cached entry, its stored transaction) of the last row
        let mut current: Option<(u32, Option<&'a TransactionDomain>)> = None;
        let selection = (0..count).map(|idx| selected.as_ref().map_or(idx, |rows| rows[idx] as usize));
        for idx in selection {
            let cached: &'a CachedRow = &postings.rows[idx];
            let txn = match current {
                Some((entry, txn)) if entry == cached.entry => txn,
                _ => {
                    let cached_entry = &postings.entries[cached.entry as usize];
                    let txn = transaction(cached.entry as usize);
                    // the cache was made from this store, so every transaction is there
                    debug_assert!(txn.is_some(), "a cached transaction is not in the store");
                    if let Some(txn) = txn {
                        entries.push(Entry {
                            txn: MaybeOwned::Borrowed(txn),
                            date: cached_entry.date,
                            meta: cached_entry.parsed.and_then(|idx| match &ledger.directives[idx as usize].data {
                                Directive::Transaction(parsed) => Some(&parsed.meta),
                                _ => None,
                            }),
                            seq: cached_entry.entry,
                            errors: cached_entry.entry.and_then(|seq| table.rows[seq as usize].errors.as_ref()),
                        });
                    }
                    current = Some((cached.entry, txn));
                    txn
                }
            };
            if txn.is_none() {
                continue;
            }
            let lot: Option<&'a Lot> = cached.lot.as_deref();
            rows.push(Row {
                entry: entries.len() - 1,
                posting_index: cached.posting_index as usize,
                account: postings.account_name(cached.account),
                units: MaybeOwned::Borrowed(&cached.units),
                cost: lot.and_then(|lot| lot.cost.as_ref()).filter(|_| keep_cost).map(MaybeOwned::Borrowed),
                price: lot.and_then(|lot| lot.price.as_ref()).filter(|_| keep_price).map(MaybeOwned::Borrowed),
            });
        }

        Dataset {
            table: &POSTINGS,
            entries,
            rows,
            records: vec![],
            today,
            projection,
            ledger,
            store,
            cache,
            store_meta: OnceCell::new(),
        }
    }

    pub fn entry(&self, row: &Row<'_>) -> &Entry<'a> {
        &self.entries[row.entry]
    }

    /// The metadata of the row's posting, as the store keeps it (sorted by key).
    pub fn posting_metas(&self, row: &Row<'_>) -> &[PostingMetaDomain] {
        self.entry(row)
            .txn
            .postings
            .get(row.posting_index)
            .map_or(&[], |posting| posting.metas.as_slice())
    }

    /// Posting-level metadata `key` of the row, as a string: the first value when the
    /// key is repeated, as for [`Dataset::entry_meta`].
    pub fn posting_meta(&self, row: &Row<'_>, key: &str) -> Option<String> {
        self.posting_metas(row).iter().find(|meta| meta.key == key).map(|meta| meta.value.clone())
    }

    /// Transaction metadata `key` of the row, as a string.
    pub fn entry_meta(&self, row: &Row<'_>, key: &str) -> Option<String> {
        let entry = self.entry(row);
        if let Some(meta) = entry.meta {
            return meta.get_one(key).map(|value| value.as_str().to_owned());
        }
        self.stored_entry_metas(row)
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| (*value).to_owned())
    }

    /// Every value of transaction metadata `key` of the row, in written order.
    pub fn entry_meta_values(&self, row: &Row<'_>, key: &str) -> Vec<String> {
        match self.entry(row).meta {
            Some(meta) => meta.get_all(key).into_iter().map(|value| value.as_str().to_owned()).collect(),
            None => self
                .stored_entry_metas(row)
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, value)| (*value).to_owned())
                .collect(),
        }
    }

    /// The transaction metadata of the row as `(key, value)` pairs: the `entry_metas` column.
    pub fn entry_metas(&self, row: &Row<'_>) -> Vec<(String, String)> {
        match self.entry(row).meta {
            Some(meta) => super::meta_pairs(Some(meta)),
            None => {
                let mut pairs = self
                    .stored_entry_metas(row)
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect::<Vec<_>>();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                pairs
            }
        }
    }

    /// The transaction metadata the store keeps for the row's transaction (one value per
    /// key), for a transaction whose directive could not be matched.
    fn stored_entry_metas(&self, row: &Row<'_>) -> &[(&'a str, &'a str)] {
        let entry = self.entry(row);
        let index = self.store_meta.get_or_init(|| {
            let mut index: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
            for meta in &self.store.metas {
                if meta.meta_type == MetaType::TransactionMeta.as_ref() {
                    index
                        .entry(meta.type_identifier.as_str())
                        .or_default()
                        .push((meta.key.as_str(), meta.value.as_str()));
                }
            }
            index
        });
        index.get(entry.txn.id.to_string().as_str()).map_or(&[], Vec::as_slice)
    }
}

fn cost_spec(cost: &PostingCost, units: &Amount) -> CostSpec {
    let per_unit = cost.base.as_ref().map(|base| {
        let number = if cost.total {
            decimal::div(&base.number, &units.number.abs()).unwrap_or_else(|| base.number.clone())
        } else {
            base.number.clone()
        };
        Amount::new(number, base.commodity.clone())
    });
    CostSpec {
        per_unit,
        date: cost.date.as_ref().map(|it| it.naive_date()),
        label: cost.label.clone(),
    }
}

fn per_unit_price(price: &SingleTotalPrice, units: &Amount) -> Option<Amount> {
    match price {
        SingleTotalPrice::Single(price) => Some(price.clone()),
        SingleTotalPrice::Total(total) => decimal::div(&total.number, &units.number.abs()).map(|number| Amount::new(number, total.commodity.clone())),
    }
}

/// A booked row: its units (the posting's unless booking split it), the cost of its lot and
/// its price.
fn booked_row(draft: &Draft<'_>, units: Option<Amount>, cost: Option<Cost>, price: Option<Amount>) -> CachedRow {
    CachedRow {
        entry: draft.entry,
        posting_index: draft.posting_index,
        account: draft.account_index,
        units: units.unwrap_or_else(|| draft.units.clone()),
        lot: (cost.is_some() || price.is_some()).then(|| Box::new(Lot { cost, price })),
    }
}

/// Book the postings held at cost against the lots opened by earlier postings of the same
/// account and currency, as beancount's booking does.
///
/// - A posting at cost whose sign is opposite to an open lot is a *reduction*. Its cost spec
///   is matched against the open lots: the given fields (cost number and currency, date,
///   label) are criteria and the missing ones are wildcards, so `{100 USD}` reduces lots
///   bought at 100 USD on any date and `{}` reduces any lot. Matching lots are consumed FIFO,
///   oldest acquisition date first, or LIFO, newest first, when the account (or the ledger
///   default) uses the LIFO booking method; lots of the same date go in the order they were
///   opened, reversed for LIFO. A reduction that spans several lots is split into one row per
///   lot, each carrying the lot's cost.
/// - Any other posting at cost, and the part of a reduction no lot covers, is an
///   *augmentation*: it opens (or adds to) the lot of its cost, dated by its transaction
///   when the spec has no date. A spec without a cost number cannot open a lot, so that
///   part keeps no cost.
///
/// zhang-core books STRICT like FIFO (reporting ambiguous matches as ledger errors) and books the
/// unsupported methods (NONE, AVERAGE, AVERAGE_ONLY) with the ledger's default method, so every
/// method other than LIFO books FIFO here. Booking errors are reported by zhang-core, not by
/// queries.
///
/// Lots are keyed by account and currency, so the rows of an account only depend on the earlier
/// postings of that account. Every row keeps the cost of its lot and its price; a query keeps
/// them only when its projection reads them.
pub(super) fn book(drafts: Vec<Draft<'_>>, ledger: &Ledger, store: &Store) -> Vec<CachedRow> {
    let mut account_methods: HashMap<&str, BookingMethod> = HashMap::new();
    for meta in &store.metas {
        if meta.meta_type == MetaType::AccountMeta.as_ref() && meta.key == "booking_method" {
            if let Ok(method) = meta.value.parse::<BookingMethod>() {
                account_methods.insert(meta.type_identifier.as_str(), method);
            }
        }
    }
    let default_method = ledger.options.default_booking_method;

    let mut lots: HashMap<(&str, &str), Vec<(Cost, BigDecimal)>> = HashMap::new();
    let mut rows = Vec::with_capacity(drafts.len());
    for mut draft in drafts {
        let Some(spec) = draft.cost.take() else {
            let price = draft.price.take();
            rows.push(booked_row(&draft, None, None, price));
            continue;
        };
        let account_lots = lots.entry((draft.account, draft.units.commodity.as_str())).or_default();
        let mut remaining = draft.units.number.clone();

        // a reduction, like beancount's `Inventory.is_reduced_by`
        let reducing = !remaining.is_zero() && account_lots.iter().any(|(_, number)| number.is_positive() != remaining.is_positive());
        if reducing {
            let lifo = matches!(account_methods.get(draft.account).copied().unwrap_or(default_method), BookingMethod::Lifo);
            // oldest acquisition date first, lots of the same date in insertion order (a stable
            // sort); LIFO is the exact reverse, like zhang-core's booking
            let mut order = (0..account_lots.len()).collect::<Vec<_>>();
            order.sort_by_key(|&idx| account_lots[idx].0.date);
            if lifo {
                order.reverse();
            }
            for idx in order {
                if remaining.is_zero() {
                    break;
                }
                let (lot, number) = &mut account_lots[idx];
                if number.is_positive() == remaining.is_positive() || !spec.matches(lot) {
                    continue;
                }
                let take = if remaining.abs() >= number.abs() {
                    -number.clone()
                } else {
                    remaining.clone()
                };
                *number += &take;
                remaining -= &take;
                let units = Amount::new(take, draft.units.commodity.clone());
                rows.push(booked_row(&draft, Some(units), Some(lot.clone()), draft.price.clone()));
            }
            account_lots.retain(|(_, number)| !number.is_zero());
        }
        if remaining.is_zero() {
            continue;
        }

        // an augmentation (or the rest of a reduction no lot covers)
        let cost = spec.per_unit.map(|per_unit| {
            let cost = Cost {
                number: per_unit.number,
                currency: per_unit.commodity,
                date: Some(spec.date.unwrap_or(draft.date)),
                label: spec.label,
            };
            match account_lots.iter_mut().find(|(lot, _)| *lot == cost) {
                Some((_, number)) => *number += &remaining,
                None => account_lots.push((cost.clone(), remaining.clone())),
            }
            account_lots.retain(|(_, number)| !number.is_zero());
            cost
        });
        let units = (remaining != draft.units.number).then(|| Amount::new(remaining, draft.units.commodity.clone()));
        let price = draft.price.take();
        rows.push(booked_row(&draft, units, cost, price));
    }
    rows
}

/// The `balance` column. The executor evaluates it as a running sum (see
/// [`crate::compiler::CExpr::Running`]); its [`ColumnDef::get`] is the row's own
/// contribution.
pub(crate) const BALANCE_COLUMN: &str = "balance";

/// The `account_balance` column, a running sum per account the executor evaluates like
/// [`BALANCE_COLUMN`] (see [`crate::running`]); its [`ColumnDef::get`] is the row's own
/// contribution.
pub(crate) const ACCOUNT_BALANCE_COLUMN: &str = "account_balance";

/// The units and cost of a row, as the `position` column reads them.
pub(crate) fn position(row: &Row<'_>) -> Position {
    Position::new(row.units.as_ref().clone(), row.cost.as_deref().cloned())
}

/// A time of day as `HH:MM:SS`, the value of the `time` columns.
pub(crate) fn time_value(time: NaiveTime) -> Value {
    Value::Str(format!("{:02}:{:02}:{:02}", time.hour(), time.minute(), time.second()))
}

/// The kind of the error zhang records for a transaction that does not balance.
pub(crate) const UNBALANCED: &str = "UnbalancedTransaction";

/// The `balanced` column: whether no error says the transaction does not balance.
pub(crate) fn balanced(errors: Option<&BTreeSet<String>>) -> Value {
    Value::Bool(!errors.is_some_and(|errors| errors.contains(UNBALANCED)))
}

/// The `errors` column: the kinds of the errors of a transaction.
pub(crate) fn error_kinds(errors: Option<&BTreeSet<String>>) -> Value {
    Value::Set(errors.cloned().unwrap_or_default())
}

/// The columns produced by `SELECT *`.
pub(crate) const WILDCARD_COLUMNS: [&str; 6] = ["date", "flag", "payee", "narration", "account", "position"];

fn set_of(items: &[String]) -> Value {
    Value::Set(items.iter().cloned().collect::<BTreeSet<_>>())
}

fn opt_str(value: Option<&str>) -> Value {
    value.map(|it| Value::Str(it.to_owned())).unwrap_or(Value::Null)
}

fn weight(row: &Row<'_>) -> Amount {
    match (&row.cost, &row.price) {
        (Some(cost), _) => Amount::new(decimal::mul(&row.units.number, &cost.number), cost.currency.clone()),
        (None, Some(price)) => Amount::new(decimal::mul(&row.units.number, &price.number), price.commodity.clone()),
        (None, None) => row.units.as_ref().clone(),
    }
}

fn payee<'r>(data: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    data.entry(row).txn.payee.as_deref()
}

fn narration<'r>(data: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    Some(data.entry(row).txn.narration.as_deref().unwrap_or_default())
}

fn account<'r>(_: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    Some(row.account)
}

fn currency<'r>(_: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    Some(&row.units.commodity)
}

fn cost_currency<'r>(_: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    row.cost.as_ref().map(|cost| cost.currency.as_str())
}

fn cost_label<'r>(_: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    match &row.cost {
        None => Some(""),
        Some(cost) => cost.label.as_deref(),
    }
}

/// The accounts of the other postings of the row's transaction.
fn other_accounts<'r>(data: &'r Dataset<'_>, row: &'r Row<'_>) -> impl Iterator<Item = &'r str> {
    let posting_index = row.posting_index;
    data.entry(row)
        .txn
        .postings
        .iter()
        .enumerate()
        .filter(move |(idx, _)| *idx != posting_index)
        .map(|(_, posting)| posting.account.name())
}

/// The `postings` table columns.
pub static COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        name: "date",
        ty: DataType::Date,
        description: "Date of the transaction.",
        get: Get::Posting(|data, row| Value::Date(data.entry(row).date)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "year",
        ty: DataType::Int,
        description: "Year of the transaction date.",
        get: Get::Posting(|data, row| Value::Int(data.entry(row).date.year() as i64)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "month",
        ty: DataType::Int,
        description: "Month (1-12) of the transaction date.",
        get: Get::Posting(|data, row| Value::Int(data.entry(row).date.month() as i64)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "day",
        ty: DataType::Int,
        description: "Day of month of the transaction date.",
        get: Get::Posting(|data, row| Value::Int(data.entry(row).date.day() as i64)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "flag",
        ty: DataType::Str,
        description: "Flag of the transaction: '*', '!', or 'P' for padding.",
        get: Get::Posting(|data, row| Value::Str(data.entry(row).txn.flag.to_string())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "payee",
        ty: DataType::Str,
        description: "Payee of the transaction.",
        get: Get::Posting(|data, row| opt_str(payee(data, row))),
        reads: Reads::POSTING,
        borrow: Borrow::Str(payee),
    },
    ColumnDef {
        name: "narration",
        ty: DataType::Str,
        description: "Narration of the transaction; '' when absent (as in beancount).",
        get: Get::Posting(|data, row| opt_str(narration(data, row))),
        reads: Reads::POSTING,
        borrow: Borrow::Str(narration),
    },
    ColumnDef {
        name: "description",
        ty: DataType::Str,
        description: "Payee and narration joined with ' | ' (whichever are present).",
        get: Get::Posting(|data, row| {
            let txn = &data.entry(row).txn;
            let parts = [txn.payee.as_deref(), txn.narration.as_deref()]
                .into_iter()
                .flatten()
                .filter(|it| !it.is_empty())
                .collect::<Vec<_>>();
            Value::Str(parts.join(" | "))
        }),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "tags",
        ty: DataType::Set,
        description: "Tags of the transaction.",
        get: Get::Posting(|data, row| set_of(&data.entry(row).txn.tags)),
        reads: Reads::POSTING,
        borrow: Borrow::Contains(|data, row, tag| data.entry(row).txn.tags.iter().any(|it| it == tag)),
    },
    ColumnDef {
        name: "links",
        ty: DataType::Set,
        description: "Links of the transaction.",
        get: Get::Posting(|data, row| set_of(&data.entry(row).txn.links)),
        reads: Reads::POSTING,
        borrow: Borrow::Contains(|data, row, link| data.entry(row).txn.links.iter().any(|it| it == link)),
    },
    ColumnDef {
        name: "id",
        ty: DataType::Str,
        description: "Unique id of the transaction.",
        get: Get::Posting(|data, row| Value::Str(data.entry(row).txn.id.to_string())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "posting_flag",
        ty: DataType::Str,
        description: "Flag of the posting itself, such as '!'; NULL when the posting has none (as in beanquery).",
        get: Get::Posting(|data, row| {
            let posting = data.entry(row).txn.postings.get(row.posting_index);
            posting.and_then(|it| it.flag.as_ref()).map_or(Value::Null, |flag| Value::Str(flag.to_string()))
        }),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "account",
        ty: DataType::Str,
        description: "Account of the posting.",
        get: Get::Posting(|data, row| opt_str(account(data, row))),
        reads: Reads::POSTING,
        borrow: Borrow::Str(account),
    },
    ColumnDef {
        name: "number",
        ty: DataType::Decimal,
        description: "Number of units of the posting.",
        get: Get::Posting(|_, row| Value::Decimal(row.units.number.clone())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "currency",
        ty: DataType::Str,
        description: "Currency of the units of the posting.",
        get: Get::Posting(|data, row| opt_str(currency(data, row))),
        reads: Reads::POSTING,
        borrow: Borrow::Str(currency),
    },
    ColumnDef {
        name: "position",
        ty: DataType::Position,
        description: "Units and cost of the posting.",
        get: Get::Posting(|_, row| Value::Position(position(row))),
        reads: Reads::COST,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "cost_number",
        ty: DataType::Decimal,
        description: "Per-unit cost number of the posting's lot.",
        get: Get::Posting(|_, row| row.cost.as_ref().map(|cost| Value::Decimal(cost.number.clone())).unwrap_or(Value::Null)),
        reads: Reads::COST,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "cost_currency",
        ty: DataType::Str,
        description: "Cost currency of the posting's lot.",
        get: Get::Posting(|data, row| opt_str(cost_currency(data, row))),
        reads: Reads::COST,
        borrow: Borrow::Str(cost_currency),
    },
    ColumnDef {
        name: "cost_date",
        ty: DataType::Date,
        description: "Acquisition date of the posting's lot.",
        get: Get::Posting(|_, row| row.cost.as_ref().and_then(|cost| cost.date).map(Value::Date).unwrap_or(Value::Null)),
        reads: Reads::COST,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "cost_label",
        ty: DataType::Str,
        description: "Label of the posting's lot; '' when the posting has no cost (as in beanquery).",
        get: Get::Posting(|data, row| opt_str(cost_label(data, row))),
        reads: Reads::COST,
        borrow: Borrow::Str(cost_label),
    },
    ColumnDef {
        name: "price",
        ty: DataType::Amount,
        description: "Per-unit price annotation (@ or @@) of the posting.",
        get: Get::Posting(|_, row| row.price.as_ref().map(|price| Value::Amount(price.as_ref().clone())).unwrap_or(Value::Null)),
        reads: Reads::PRICE,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "weight",
        ty: DataType::Amount,
        description: "Amount the posting contributes to the transaction balance: units × cost, else units × price, else units.",
        get: Get::Posting(|_, row| Value::Amount(weight(row))),
        reads: Reads::COST_AND_PRICE,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "other_accounts",
        ty: DataType::Set,
        description: "Accounts of the other postings of the transaction.",
        get: Get::Posting(|data, row| Value::Set(other_accounts(data, row).map(str::to_owned).collect())),
        reads: Reads::POSTING,
        borrow: Borrow::Contains(|data, row, account| other_accounts(data, row).any(|it| it == account)),
    },
    ColumnDef {
        name: "meta",
        ty: DataType::Str,
        description: "Metadata of the posting, as `key: \"value\"` pairs.",
        get: Get::Posting(|data, row| render_pairs(data.posting_metas(row).iter().map(|meta| (meta.key.as_str(), meta.value.as_str())))),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "metas",
        ty: DataType::Metas,
        description: "Metadata of the posting as (key, value) pairs: sorted by key, every value of a repeated key in written order. A zhang extension.",
        get: Get::Posting(|data, row| Value::Metas(data.posting_metas(row).iter().map(|meta| (meta.key.clone(), meta.value.clone())).collect())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "entry_metas",
        ty: DataType::Metas,
        description: "Metadata of the transaction as (key, value) pairs: sorted by key, every value of a repeated key in written order. A \
                      zhang extension.",
        get: Get::Posting(|data, row| Value::Metas(data.entry_metas(row))),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: BALANCE_COLUMN,
        ty: DataType::Inventory,
        description: "Running balance: the sum of the positions of the rows produced so far, in ledger order after FROM and WHERE, \
                      including this one; not allowed in FROM or WHERE.",
        get: Get::Posting(|_, row| Value::Inventory(Inventory::from_iter([position(row)]))),
        reads: Reads::COST,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "time",
        ty: DataType::Str,
        description: "Time of day of the transaction in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, moved past the gap on a day daylight saving skips it, as zhang stores it. \
                      A zhang extension.",
        get: Get::Posting(|data, row| time_value(data.entry(row).txn.datetime.time())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "timestamp",
        ty: DataType::Int,
        description: "Unix time, in seconds, of the transaction's date and time. A zhang extension.",
        get: Get::Posting(|data, row| Value::Int(data.entry(row).txn.datetime.timestamp())),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "seq",
        ty: DataType::Int,
        description: "Position of the transaction, from 0, in the order zhang processes the ledger, as in #entries, shared by all \
                      its postings: ORDER BY seq DESC lists the newest first. NULL for the synthetic transactions of OPEN, CLOSE \
                      and CLEAR. A zhang extension.",
        get: Get::Posting(|data, row| data.entry(row).seq.map_or(Value::Null, |seq| Value::Int(data.entry_order(seq).into()))),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "posting_index",
        ty: DataType::Int,
        description: "Index of the posting in its transaction as written, from 0; the rows of a posting that booking splits across \
                      lots share it. A zhang extension.",
        get: Get::Posting(|_, row| Value::Int(row.posting_index as i64)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: ACCOUNT_BALANCE_COLUMN,
        ty: DataType::Inventory,
        description: "Balance of the posting's account right after this posting: the sum of every posting of the account in ledger \
                      order, whatever FROM, WHERE and LIMIT select (unlike balance); with OPEN, CLOSE or CLEAR, of the rows they \
                      produce. A zhang extension.",
        get: Get::Posting(|_, row| Value::Inventory(Inventory::from_iter([position(row)]))),
        reads: Reads::COST,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "balanced",
        ty: DataType::Bool,
        description: "FALSE when zhang recorded that the transaction does not balance (an UnbalancedTransaction error), else TRUE. \
                      A zhang extension.",
        get: Get::Posting(|data, row| balanced(data.entry(row).errors)),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
    ColumnDef {
        name: "errors",
        ty: DataType::Set,
        description: "Kinds of the errors zhang recorded for the transaction, as #errors names them in kind; empty when there are \
                      none. A zhang extension.",
        get: Get::Posting(|data, row| error_kinds(data.entry(row).errors)),
        reads: Reads::POSTING,
        borrow: Borrow::Contains(|data, row, kind| data.entry(row).errors.is_some_and(|errors| errors.contains(kind))),
    },
    ColumnDef {
        name: "automatic",
        ty: DataType::Bool,
        description: "TRUE when the posting was written without an amount and zhang inferred its units to balance the transaction \
                      (beancount's automatic postings); FALSE when its amount is written. A zhang extension.",
        get: Get::Posting(|data, row| Value::Bool(automatic(data, row))),
        reads: Reads::POSTING,
        borrow: Borrow::No,
    },
];

/// Whether the row's posting was written without an amount, which zhang inferred. The leg of
/// the padding account of a padding transaction is such a posting too.
fn automatic(data: &Dataset<'_>, row: &Row<'_>) -> bool {
    data.entry(row)
        .txn
        .postings
        .get(row.posting_index)
        .is_some_and(|posting| posting.unit.is_none())
}

#[cfg(test)]
mod tests {
    use zhang_ast::error::ErrorKind;

    use super::{is_under, UNBALANCED};

    #[test]
    fn unbalanced_is_the_kind_of_unbalanced_transactions() {
        assert_eq!(ErrorKind::UnbalancedTransaction.to_string(), UNBALANCED);
    }

    #[test]
    fn sub_accounts_are_under_their_ancestors() {
        assert!(is_under("Assets:Bank", "Assets:Bank"));
        assert!(is_under("Assets:Bank:Checking", "Assets:Bank"));
        assert!(is_under("Assets:Bank:Checking", "Assets"));
        assert!(!is_under("Assets:Banking", "Assets:Bank"));
        assert!(!is_under("Assets", "Assets:Bank"));
        assert!(!is_under("Assets:Bank", ""));
    }
}
