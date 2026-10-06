//! The `postings` table: one row per posting, with the columns of its transaction.
//!
//! Rows are read from the booked directives of the ledger (units, lots, price annotations,
//! transaction metadata) and from its in-memory [`Store`] (the transaction they belong to and the
//! posting metadata). The store records the directive of every transaction it keeps.
//!
//! Which entries produce rows follows beancount: transactions and padding transactions
//! (flag `P`) do; balance assertions, which book nothing, do not.
//!
//! The rows are the booked postings of the ledger ([`booked_rows`], kept in the [`LedgerCache`]):
//! zhang books every transaction while it loads (the booking stage and the store fold, see
//! `zhang_core::booking`), so a sale across several lots is already one posting per lot, each
//! naming its lot, and an implicit posting has its units. The table books nothing itself. A query
//! then assembles its rows from the cached ones: only those of the accounts it is scoped to
//! ([`Scope`]), each keeping only the parts its projected columns read (see [`crate::projector`]).
//! Columns of the transaction are read from the store on access, never copied up front.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::BTreeSet;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use zhang_ast::amount::Amount;
use zhang_ast::{booked_group_units, Directive, Meta, Posting, PostingCost, SingleTotalPrice, WrittenGroup};
use zhang_core::ledger::Ledger;
use zhang_core::store::{PostingMetaDomain, Store, TransactionDomain};

use super::cache::{Accounts, CachedRow, LedgerCache, Lot, Postings};
use super::{render_pairs, Borrow, ColumnDef, Dataset, POSTINGS};
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
    /// the cost of the booked lot
    pub cost: Option<MaybeOwned<'a, Cost>>,
    /// per-unit price annotation
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
                            meta: match &ledger.directives[cached_entry.parsed as usize].data {
                                Directive::Transaction(parsed) => Some(&parsed.meta),
                                _ => None,
                            },
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
                cost: lot.and_then(|lot| lot.cost.as_ref()).map(MaybeOwned::Borrowed),
                price: lot.and_then(|lot| lot.price.as_ref()).map(MaybeOwned::Borrowed),
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
            budgets: OnceCell::new(),
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
        self.entry(row).meta?.get_one(key).map(|value| value.as_str().to_owned())
    }

    /// Every value of transaction metadata `key` of the row, in written order.
    pub fn entry_meta_values(&self, row: &Row<'_>, key: &str) -> Vec<String> {
        self.entry(row)
            .meta
            .map(|meta| meta.get_all(key).into_iter().map(|value| value.as_str().to_owned()).collect())
            .unwrap_or_default()
    }

    /// The transaction metadata of the row as `(key, value)` pairs: the `entry_metas` column.
    pub fn entry_metas(&self, row: &Row<'_>) -> Vec<(String, String)> {
        super::meta_pairs(self.entry(row).meta)
    }
}

/// The rows of a stored transaction, the entry `entry` of the table, from the booked postings of
/// its directive, grouped by the posting they were written as (`groups`, [`zhang_ast::written_groups`]):
/// one row per booked leg, as beanquery lists a sale across several lots, with the lot the leg
/// names and its per-unit price. The rows of a posting as written share its index, which is the
/// store's row.
pub(super) fn booked_rows(entry: usize, groups: &[WrittenGroup<'_>], accounts: &mut Accounts) -> Vec<CachedRow> {
    let mut rows = Vec::with_capacity(groups.len());
    for (posting_index, group) in groups.iter().enumerate() {
        let legs = group.legs;
        for leg in legs {
            // every leg of a stored transaction is booked, so it has units
            let Some(units) = &leg.units else { continue };
            let cost = leg.cost.as_ref().and_then(lot_cost);
            let price = leg.price.as_ref().and_then(|price| per_unit_price(leg, price, legs));
            rows.push(CachedRow {
                entry: entry as u32,
                posting_index: posting_index as u32,
                account: accounts.index(leg.account.name()),
                units: units.clone(),
                lot: (cost.is_some() || price.is_some()).then(|| Box::new(Lot { cost, price })),
            });
        }
    }
    rows
}

/// The units of the posting `leg` was written as, which a total price (`@@`) or a total cost
/// (`{{1000 USD}}`) is spread over: those the leg carries when booking changed its posting (kept
/// by a leg of a split a stage broke apart from the others), else the legs of its group `legs`
/// summed. `None` when a leg has no units.
fn written_units(leg: &Posting, legs: &[Posting]) -> Option<Amount> {
    match leg.written.as_ref().and_then(|written| written.units.as_ref()) {
        Some(units) => Some(units.clone()),
        None => booked_group_units(legs),
    }
}

/// The lot a booked leg names, as its row shows it: the cost zhang-core booked it at, which for a
/// cost written as a total (`{{1000 USD}}`) or inferred from the other postings is the per-unit
/// cost divided in the 28-digit decimal context like beanquery's ([`decimal::per_unit`]), the cost
/// the lot keeps. A cost without a number (the part of a `{}` reduction no lot covered) is no lot.
fn lot_cost(cost: &PostingCost) -> Option<Cost> {
    let base = cost.base.as_ref()?;
    Some(Cost {
        number: base.number.clone(),
        currency: base.commodity.clone(),
        date: cost.date.as_ref().map(|it| it.naive_date()),
        label: cost.label.clone(),
    })
}

/// the per-unit price of the leg `leg` of `legs`: a total price (`@@`) spread over the units as
/// written ([`written_units`], [`decimal::per_unit`]), the same for every leg of a split
fn per_unit_price(leg: &Posting, price: &SingleTotalPrice, legs: &[Posting]) -> Option<Amount> {
    match price {
        SingleTotalPrice::Single(price) => Some(price.clone()),
        SingleTotalPrice::Total(total) => {
            let units = written_units(leg, legs)?;
            decimal::per_unit(&total.number, &units.number).map(|number| Amount::new(number, total.commodity.clone()))
        }
    }
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

/// The weight of a row: its units at its cost, else at its price, in the decimal context like
/// beanquery's (a per-unit cost or price may be a 28-digit quotient), else its units.
fn weight(row: &Row<'_>) -> Amount {
    match (&row.cost, &row.price) {
        (Some(cost), _) => Amount::new(decimal::mul_in_context(&row.units.number, &cost.number), cost.currency.clone()),
        (None, Some(price)) => Amount::new(decimal::mul_in_context(&row.units.number, &price.number), price.commodity.clone()),
        (None, None) => row.units.as_ref().clone(),
    }
}

// The columns of a stored transaction, read the same way by `postings`, `#entries` and `#transactions`: from what
// zhang stored, at the date and time it books the transaction at.

/// The `date` of a stored transaction: its date in the ledger's timezone, as zhang books it.
pub(super) fn txn_date(txn: &TransactionDomain) -> NaiveDate {
    txn.datetime.date_naive()
}

/// The `flag` of a stored transaction: `*` when it was written without one.
pub(super) fn txn_flag(txn: &TransactionDomain) -> Value {
    Value::Str(txn.flag.to_string())
}

/// The `payee` of a stored transaction.
pub(super) fn txn_payee(txn: &TransactionDomain) -> Option<&str> {
    txn.payee.as_deref()
}

/// The `narration` of a stored transaction: '' when absent, as in beancount.
pub(super) fn txn_narration(txn: &TransactionDomain) -> &str {
    txn.narration.as_deref().unwrap_or_default()
}

/// The `description` of a stored transaction: its payee and narration joined with ' | ', whichever are present.
pub(super) fn txn_description(txn: &TransactionDomain) -> Value {
    let parts = [txn.payee.as_deref(), txn.narration.as_deref()]
        .into_iter()
        .flatten()
        .filter(|it| !it.is_empty())
        .collect::<Vec<_>>();
    Value::Str(parts.join(" | "))
}

/// The `time` of a stored transaction: its time of day in the ledger's timezone, as zhang books it.
pub(super) fn txn_time(txn: &TransactionDomain) -> Value {
    time_value(txn.datetime.time())
}

/// The `timestamp` of a stored transaction: the Unix time zhang books it at.
pub(super) fn txn_timestamp(txn: &TransactionDomain) -> Value {
    Value::Int(txn.datetime.timestamp())
}

fn payee<'r>(data: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    txn_payee(&data.entry(row).txn)
}

fn narration<'r>(data: &'r Dataset<'_>, row: &'r Row<'_>) -> Option<&'r str> {
    Some(txn_narration(&data.entry(row).txn))
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
    ColumnDef::posting("date", DataType::Date, "Date of the transaction.", |data, row| {
        Value::Date(data.entry(row).date)
    }),
    ColumnDef::posting("year", DataType::Int, "Year of the transaction date.", |data, row| {
        Value::Int(data.entry(row).date.year() as i64)
    }),
    ColumnDef::posting("month", DataType::Int, "Month (1-12) of the transaction date.", |data, row| {
        Value::Int(data.entry(row).date.month() as i64)
    }),
    ColumnDef::posting("day", DataType::Int, "Day of month of the transaction date.", |data, row| {
        Value::Int(data.entry(row).date.day() as i64)
    }),
    ColumnDef::posting("flag", DataType::Str, "Flag of the transaction: '*', '!', or 'P' for padding.", |data, row| {
        txn_flag(&data.entry(row).txn)
    }),
    ColumnDef::posting("payee", DataType::Str, "Payee of the transaction.", |data, row| opt_str(payee(data, row))).borrowing(Borrow::Str(payee)),
    ColumnDef::posting(
        "narration",
        DataType::Str,
        "Narration of the transaction; '' when absent (as in beancount).",
        |data, row| opt_str(narration(data, row)),
    )
    .borrowing(Borrow::Str(narration)),
    ColumnDef::posting(
        "description",
        DataType::Str,
        "Payee and narration joined with ' | ' (whichever are present).",
        |data, row| txn_description(&data.entry(row).txn),
    ),
    ColumnDef::posting("tags", DataType::Set, "Tags of the transaction.", |data, row| set_of(&data.entry(row).txn.tags))
        .borrowing(Borrow::Contains(|data, row, tag| data.entry(row).txn.tags.iter().any(|it| it == tag))),
    ColumnDef::posting("links", DataType::Set, "Links of the transaction.", |data, row| {
        set_of(&data.entry(row).txn.links)
    })
    .borrowing(Borrow::Contains(|data, row, link| data.entry(row).txn.links.iter().any(|it| it == link))),
    ColumnDef::posting("id", DataType::Str, "Unique id of the transaction.", |data, row| {
        Value::Str(data.entry(row).txn.id.to_string())
    }),
    ColumnDef::posting(
        "posting_flag",
        DataType::Str,
        "Flag of the posting itself, such as '!'; NULL when the posting has none (as in beanquery).",
        |data, row| {
            let posting = data.entry(row).txn.postings.get(row.posting_index);
            posting.and_then(|it| it.flag.as_ref()).map_or(Value::Null, |flag| Value::Str(flag.to_string()))
        },
    ),
    ColumnDef::posting("account", DataType::Str, "Account of the posting.", |data, row| opt_str(account(data, row))).borrowing(Borrow::Str(account)),
    ColumnDef::posting("number", DataType::Decimal, "Number of units of the posting.", |_, row| {
        Value::Decimal(row.units.number.clone())
    }),
    ColumnDef::posting("currency", DataType::Str, "Currency of the units of the posting.", |data, row| {
        opt_str(currency(data, row))
    })
    .borrowing(Borrow::Str(currency)),
    ColumnDef::posting("position", DataType::Position, "Units and cost of the posting.", |_, row| {
        Value::Position(position(row))
    }),
    ColumnDef::posting("cost_number", DataType::Decimal, "Per-unit cost number of the posting's lot.", |_, row| {
        row.cost.as_ref().map(|cost| Value::Decimal(cost.number.clone())).unwrap_or(Value::Null)
    }),
    ColumnDef::posting("cost_currency", DataType::Str, "Cost currency of the posting's lot.", |data, row| {
        opt_str(cost_currency(data, row))
    })
    .borrowing(Borrow::Str(cost_currency)),
    ColumnDef::posting("cost_date", DataType::Date, "Acquisition date of the posting's lot.", |_, row| {
        row.cost.as_ref().and_then(|cost| cost.date).map(Value::Date).unwrap_or(Value::Null)
    }),
    ColumnDef::posting(
        "cost_label",
        DataType::Str,
        "Label of the posting's lot; '' when the posting has no cost (as in beanquery).",
        |data, row| opt_str(cost_label(data, row)),
    )
    .borrowing(Borrow::Str(cost_label)),
    ColumnDef::posting("price", DataType::Amount, "Per-unit price annotation (@ or @@) of the posting.", |_, row| {
        row.price.as_ref().map(|price| Value::Amount(price.as_ref().clone())).unwrap_or(Value::Null)
    }),
    ColumnDef::posting(
        "weight",
        DataType::Amount,
        "Amount the posting contributes to the transaction balance: units × cost, else units × price, else units.",
        |_, row| Value::Amount(weight(row)),
    ),
    ColumnDef::posting(
        "other_accounts",
        DataType::Set,
        "Accounts of the other postings of the transaction.",
        |data, row| Value::Set(other_accounts(data, row).map(str::to_owned).collect()),
    )
    .borrowing(Borrow::Contains(|data, row, account| other_accounts(data, row).any(|it| it == account))),
    ColumnDef::posting("meta", DataType::Str, "Metadata of the posting, as `key: \"value\"` pairs.", |data, row| {
        render_pairs(data.posting_metas(row).iter().map(|meta| (meta.key.as_str(), meta.value.as_str())))
    }),
    ColumnDef::posting(
        "metas",
        DataType::Metas,
        "Metadata of the posting as (key, value) pairs: sorted by key, every value of a repeated key in written order. A zhang extension.",
        |data, row| Value::Metas(data.posting_metas(row).iter().map(|meta| (meta.key.clone(), meta.value.clone())).collect()),
    ),
    ColumnDef::posting(
        "entry_metas",
        DataType::Metas,
        "Metadata of the transaction as (key, value) pairs: sorted by key, every value of a repeated key in written order. A \
        zhang extension.",
        |data, row| Value::Metas(data.entry_metas(row)),
    ),
    ColumnDef::posting(
        BALANCE_COLUMN,
        DataType::Inventory,
        "Running balance: the sum of the positions of the rows produced so far, in ledger order after FROM and WHERE, \
        including this one; not allowed in FROM or WHERE.",
        |_, row| Value::Inventory(Inventory::from_iter([position(row)])),
    ),
    ColumnDef::posting(
        "time",
        DataType::Str,
        "Time of day of the transaction in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, \
        moved past the gap on a day daylight saving skips it, as zhang stores it. A zhang extension.",
        |data, row| txn_time(&data.entry(row).txn),
    ),
    ColumnDef::posting(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the transaction's date and time. A zhang extension.",
        |data, row| txn_timestamp(&data.entry(row).txn),
    ),
    ColumnDef::posting(
        "seq",
        DataType::Int,
        "Position of the transaction, from 0, in the order zhang processes the ledger, as in #entries, shared by all \
        its postings: ORDER BY seq DESC lists the newest first. NULL for the synthetic transactions of OPEN, CLOSE \
        and CLEAR. A zhang extension.",
        |data, row| data.entry(row).seq.map_or(Value::Null, |seq| Value::Int(seq.into())),
    ),
    ColumnDef::posting(
        "posting_index",
        DataType::Int,
        "Index of the posting in its transaction as written, from 0; the rows of a posting that booking splits across \
        lots share it. A zhang extension.",
        |_, row| Value::Int(row.posting_index as i64),
    ),
    ColumnDef::posting(
        ACCOUNT_BALANCE_COLUMN,
        DataType::Inventory,
        "Balance of the posting's account right after this posting: the sum of every posting of the account in ledger \
        order, whatever FROM, WHERE and LIMIT select (unlike balance); with OPEN, CLOSE or CLEAR, of the rows they \
        produce. A zhang extension.",
        |_, row| Value::Inventory(Inventory::from_iter([position(row)])),
    ),
    ColumnDef::posting(
        "balanced",
        DataType::Bool,
        "FALSE when zhang recorded that the transaction does not balance (an UnbalancedTransaction error), else TRUE. \
        A zhang extension.",
        |data, row| balanced(data.entry(row).errors),
    ),
    ColumnDef::posting(
        "errors",
        DataType::Set,
        "Kinds of the errors zhang recorded for the transaction, as #errors names them in kind; empty when there are \
        none. A zhang extension.",
        |data, row| error_kinds(data.entry(row).errors),
    )
    .borrowing(Borrow::Contains(|data, row, kind| {
        data.entry(row).errors.is_some_and(|errors| errors.contains(kind))
    })),
    ColumnDef::posting(
        "automatic",
        DataType::Bool,
        "TRUE when the posting was written without an amount and zhang inferred its units to balance the transaction \
        (beancount's automatic postings); FALSE when its amount is written. A zhang extension.",
        |data, row| Value::Bool(automatic(data, row)),
    ),
    ColumnDef::posting(
        "budgets",
        DataType::Set,
        "The budgets the posting counts toward, as the budget pages count it: those the budget metadata of its \
        account's open in effect at its date and time names, defined before it and not closed by then, into \
        whose commodity a price converts it. The postings of a budget in a month add up to its activity in \
        #budgets. A zhang extension.",
        |data, row| Value::Set(super::budgets::posting_budgets(data, row)),
    ),
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
