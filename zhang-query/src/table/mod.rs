//! Tables: the row sources a query reads, selected with `FROM #name`.
//!
//! Every table is a static [`Table`] in the registry ([`tables`]): a name, a description,
//! typed columns ([`ColumnDef`]), the columns `SELECT *` expands to, and a row source. The
//! compiler resolves the query's table first and then its column names against that table
//! only; the projector prunes the table's columns; the executor reads rows through
//! [`RowRef`], so every later stage (filters, grouping, ordering, LIMIT, the result budget
//! and the deadline) works the same way for every table.
//!
//! There are three kinds of row sources:
//!
//! - [`Rows::Postings`]: the `postings` table ([`postings`]), the default when a query names
//!   no table. Its rows are booked postings ([`Row`]), assembled for the query's projection
//!   from the rows booked once per loaded ledger ([`LedgerCache`]), and only those of the
//!   accounts the query is scoped to ([`Scope`]).
//! - [`Rows::Records`]: every other table. A builder walks the ledger once and returns one
//!   [`Record`] per row, which only borrows the ledger and the store; its columns are computed
//!   when an expression reads them. A builder may skip work that only unprojected columns
//!   need (see [`Projection::contains`]).
//! - [`Rows::Generated`]: record tables whose rows are not read from the ledger but generated,
//!   such as the months of `#budgets`, so that their number is not bounded by the size of the
//!   ledger. Their builder charges every row to the result budget and checks the deadline
//!   while it builds them ([`Limits`]), before any filter runs.
//!
//! A column named `base.attribute` is an attribute of the structured value `base`, read
//! with attribute access (`open.date` in `#accounts`); `base` itself is a column too.
//!
//! # Adding a table
//!
//! Add a module with a `static` [`Table`] whose columns read a [`Record`] variant (add one
//! when no existing variant fits), register it in [`TABLES`], and describe it in the query
//! language reference. Its columns appear in `GET /api/query/schema` automatically.

mod accounts;
mod budgets;
mod cache;
mod directives;
mod entries;
mod errors;
mod lookups;
mod postings;
mod prices;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::{Commodity, Directive, Meta, Spanned};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

pub(crate) use self::cache::LedgerCache;
pub use self::postings::COLUMNS;
pub(crate) use self::postings::{position, Entry, MaybeOwned, Row, Scope, ACCOUNT_BALANCE_COLUMN, BALANCE_COLUMN};
use crate::error::LocatedError;
use crate::executor::{Budget, Deadline};
use crate::functions::AccountDirectives;
use crate::prices::PriceMap;
use crate::projector::Projection;
use crate::value::{DataType, Value};

/// A table a query can read with `FROM #name`.
pub struct Table {
    /// the name after `#`, lower-case; names are case-sensitive
    pub name: &'static str,
    pub description: &'static str,
    pub columns: &'static [ColumnDef],
    /// the columns `SELECT *` expands to, in order
    pub(crate) wildcard: &'static [&'static str],
    pub(crate) rows: Rows,
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.name)
    }
}

impl Table {
    /// The column `name` (case-insensitive).
    pub fn column(&self, name: &str) -> Option<&'static ColumnDef> {
        self.columns.iter().find(|it| it.name.eq_ignore_ascii_case(name))
    }

    /// Whether this is the `postings` table, the default of a query without `FROM #name`.
    pub fn is_postings(&self) -> bool {
        std::ptr::eq(self, &POSTINGS)
    }

    /// The attributes of the structured column `base` (the columns named `base.attribute`),
    /// in column order; empty when `base` has none.
    pub(crate) fn attributes(&self, base: &str) -> Vec<&'static str> {
        self.columns
            .iter()
            .filter_map(|column| column.name.split_once('.'))
            .filter(|(prefix, _)| prefix.eq_ignore_ascii_case(base))
            .map(|(_, attribute)| attribute)
            .collect()
    }
}

/// How a table produces its rows.
pub(crate) enum Rows {
    /// booked postings (see [`Dataset::postings`])
    Postings,
    /// one [`Record`] per row, built for a projection
    Records(RecordSource),
    /// one generated [`Record`] per row, built for a projection within [`Limits`]
    Generated(GeneratedSource),
}

/// Builds the rows of a record table, in the table's row order.
pub(crate) type RecordSource = for<'a> fn(&'a Ledger, &'a Store, Projection) -> Vec<Record<'a>>;

/// Builds the generated rows of a record table, in the table's row order, calling
/// [`Limits::row`] for every row before it builds it.
pub(crate) type GeneratedSource = for<'a> fn(&'a Ledger, &'a Store, Projection, &mut Limits<'_>) -> Result<Vec<Record<'a>>, LocatedError>;

/// The limits of one execution as a [`GeneratedSource`] sees them: every generated row costs
/// one value of the result budget, and the deadline is checked as rows are generated, so a
/// table that would generate too many rows stops with `TooLarge` or `Timeout` instead of
/// holding them all.
pub(crate) struct Limits<'e> {
    deadline: Option<&'e Deadline>,
    budget: &'e mut Budget,
    rows: usize,
}

impl<'e> Limits<'e> {
    pub(crate) fn new(deadline: Option<&'e Deadline>, budget: &'e mut Budget) -> Self {
        Limits { deadline, budget, rows: 0 }
    }

    /// Account for one more generated row.
    pub(crate) fn row(&mut self) -> Result<(), LocatedError> {
        self.rows += 1;
        Deadline::check(self.deadline, self.rows)?;
        self.budget.charge(1)
    }
}

/// The `postings` table.
pub static POSTINGS: Table = Table {
    name: "postings",
    description: "One row per posting of every transaction, booked against the lots of its account (the default table).",
    columns: COLUMNS,
    wildcard: &postings::WILDCARD_COLUMNS,
    rows: Rows::Postings,
};

/// Every table, in the order the schema lists them.
static TABLES: &[&Table] = &[
    &POSTINGS,
    &entries::ENTRIES,
    &entries::TRANSACTIONS,
    &prices::PRICES,
    &directives::BALANCES,
    &directives::NOTES,
    &directives::EVENTS,
    &directives::DOCUMENTS,
    &accounts::ACCOUNTS,
    &directives::COMMODITIES,
    &budgets::BUDGETS,
    &budgets::BUDGET_EVENTS,
    &errors::ERRORS,
];

/// Every table a query can read, `postings` first.
pub fn tables() -> &'static [&'static Table] {
    TABLES
}

/// The table `name` (without `#`). Table names are case-sensitive, as in beanquery.
pub(crate) fn find(name: &str) -> Option<&'static Table> {
    TABLES.iter().copied().find(|table| table.name == name)
}

/// The column `name` of the `postings` table.
#[cfg(test)]
pub(crate) fn column(name: &str) -> Option<&'static ColumnDef> {
    POSTINGS.column(name)
}

/// A column of a table.
pub struct ColumnDef {
    pub name: &'static str,
    pub ty: DataType,
    pub description: &'static str,
    pub(crate) get: Get,
    /// the parts of a booked posting row the column reads besides its posting and
    /// transaction, so the projector builds them only for plans that use the column
    /// (postings only)
    pub(crate) reads: Reads,
    /// how predicates can read the column in place, without copying it into a [`Value`]
    /// (postings only)
    pub(crate) borrow: Borrow,
}

impl fmt::Debug for ColumnDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.ty)
    }
}

impl ColumnDef {
    /// A column of a record table.
    pub(crate) const fn record(name: &'static str, ty: DataType, description: &'static str, get: fn(&Dataset<'_>, &Record<'_>) -> Value) -> ColumnDef {
        ColumnDef {
            name,
            ty,
            description,
            get: Get::Record(get),
            reads: Reads::POSTING,
            borrow: Borrow::No,
        }
    }

    /// The value of the column at `row`. A column only reads rows of its own table (the
    /// compiler resolves columns against the query's table); any other row reads as NULL.
    pub(crate) fn value(&self, data: &Dataset<'_>, row: RowRef<'_, '_>) -> Value {
        match (&self.get, row) {
            (Get::Posting(get), RowRef::Posting(row)) => get(data, row),
            (Get::Record(get), RowRef::Record(record)) => get(data, record),
            _ => Value::Null,
        }
    }
}

/// How a column reads its value from a row.
pub(crate) enum Get {
    /// a column of the `postings` table
    Posting(fn(&Dataset<'_>, &Row<'_>) -> Value),
    /// a column of a record table
    Record(fn(&Dataset<'_>, &Record<'_>) -> Value),
}

/// The parts of a booked row that only some columns read (see [`Row::cost`], [`Row::price`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    /// the cost of the posting's lot
    pub cost: bool,
    /// the price annotation of the posting
    pub price: bool,
}

impl Reads {
    /// only the posting and its transaction (and every column of a record table)
    pub(crate) const POSTING: Reads = Reads { cost: false, price: false };
    pub(crate) const COST: Reads = Reads { cost: true, price: false };
    pub(crate) const PRICE: Reads = Reads { cost: false, price: true };
    pub(crate) const COST_AND_PRICE: Reads = Reads { cost: true, price: true };
}

/// In-place access to a column, for predicates that only inspect the value.
#[derive(Clone, Copy)]
pub(crate) enum Borrow {
    /// the column is only available as a [`Value`]
    No,
    /// a string column, read without copying; `None` is NULL. `get` returns the same string.
    Str(for<'r> fn(&'r Dataset<'_>, &'r Row<'_>) -> Option<&'r str>),
    /// a set column, as a membership test that does not build the set
    Contains(fn(&Dataset<'_>, &Row<'_>, &str) -> bool),
}

/// One row of a record table. It only borrows the ledger and the store; the table's columns
/// compute their values from it on access.
pub(crate) enum Record<'a> {
    /// a dated directive of the processed ledger
    Directive(&'a Spanned<Directive>),
    /// a row of `#entries` or `#transactions`: a directive with its place in the ledger
    Entry {
        directive: &'a Spanned<Directive>,
        info: &'a cache::EntryInfo,
    },
    /// a balance assertion (`balance`, or `balance ... with pad`) and, when the projection
    /// reads it, the true balance of its account at the assertion
    Balance {
        directive: &'a Spanned<Directive>,
        actual: Option<Amount>,
    },
    /// a document: a `document` directive, or a `document` metadata value
    Document(directives::DocumentRow<'a>),
    /// an account with its `open` and `close` directives (at least one of them)
    Account {
        name: &'a str,
        open: Option<&'a Spanned<Directive>>,
        close: Option<&'a Spanned<Directive>>,
    },
    /// one month of a budget
    Budget(budgets::BudgetMonth<'a>),
    /// one effect of a budget directive
    BudgetEvent(budgets::BudgetEvent<'a>),
    /// a ledger error
    Error(errors::LedgerError<'a>),
}

impl<'a> Record<'a> {
    /// The metadata of the row: what `meta()`, `entry_meta()` and `any_meta()` read on a
    /// record table (an account reads the metadata of its `open`, else of its `close`).
    fn metadata(&self) -> Option<&'a Meta> {
        match self {
            Record::Directive(directive) | Record::Balance { directive, .. } | Record::Entry { directive, .. } => directive_meta(&directive.data),
            Record::Account { open, close, .. } => open.or(*close).and_then(|directive| directive_meta(&directive.data)),
            Record::Document(document) => Some(document.metadata()),
            Record::Budget(month) => month.metadata(),
            Record::BudgetEvent(event) => directive_meta(&event.directive.data),
            // an error's details are not directive metadata (see `meta`)
            Record::Error(_) => None,
        }
    }

    /// Metadata `key` of the row, as a string; for an error, what zhang records about it.
    fn meta(&self, key: &str) -> Option<String> {
        match self {
            Record::Error(error) => error.meta(key),
            _ => self.metadata().and_then(|meta| meta.get_one(key)).map(|value| value.as_str().to_owned()),
        }
    }

    /// Every value of metadata `key` of the row, in written order.
    fn meta_values(&self, key: &str) -> Vec<String> {
        match self {
            Record::Error(error) => error.meta(key).into_iter().collect(),
            _ => self
                .metadata()
                .map(|meta| meta.get_all(key).into_iter().map(|value| value.as_str().to_owned()).collect())
                .unwrap_or_default(),
        }
    }
}

/// Metadata as `(key, value)` pairs, the value of the `metas` columns: sorted by key, the
/// values of a repeated key in written order (zhang keeps no order between different keys).
pub(crate) fn meta_pairs(meta: Option<&Meta>) -> Vec<(String, String)> {
    let mut pairs = meta
        .cloned()
        .map(|meta| meta.get_flatten())
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key, value.to_plain_string()))
        .collect::<Vec<_>>();
    // a stable sort keeps the values of a key in order
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

/// Metadata as text, the value of the `meta` columns: `key: "value"` pairs sorted by key and
/// separated by `, ` (the values of a repeated key in ledger order); `''` without metadata.
/// Quotes and backslashes in values are escaped with a backslash.
pub(crate) fn render_meta(meta: Option<&Meta>) -> Value {
    let pairs = meta.cloned().map(|meta| meta.get_flatten()).unwrap_or_default();
    render_pairs(pairs.iter().map(|(key, value)| (key.as_str(), value.as_str())))
}

/// Metadata `pairs` as text, as [`render_meta`] writes it.
pub(crate) fn render_pairs<'p>(pairs: impl Iterator<Item = (&'p str, &'p str)>) -> Value {
    let mut pairs = pairs.collect::<Vec<_>>();
    pairs.sort_by_key(|(key, _)| *key);
    let text = pairs
        .iter()
        .map(|(key, value)| format!("{}: \"{}\"", key, value.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    Value::Str(text)
}

/// The file `path` of the ledger, relative to the ledger's directory as the UI's file list
/// names it (also trying the directory of the entry file, for a ledger whose directory was
/// not given in canonical form), or the full path if it is outside.
pub(crate) fn ledger_file<'a>(ledger: &'a Ledger, path: &'a Path) -> &'a Path {
    [Some(ledger.entry.0.as_path()), ledger.visited_files.first().and_then(|it| it.parent())]
        .into_iter()
        .flatten()
        .find_map(|root| path.strip_prefix(root).ok())
        .filter(|it| !it.as_os_str().is_empty())
        .unwrap_or(path)
}

/// The metadata of a directive.
pub(crate) fn directive_meta(directive: &Directive) -> Option<&Meta> {
    Some(match directive {
        Directive::Open(it) => &it.meta,
        Directive::Close(it) => &it.meta,
        Directive::Commodity(it) => &it.meta,
        Directive::Transaction(it) => &it.meta,
        Directive::BalancePad(it) => &it.meta,
        Directive::BalanceCheck(it) => &it.meta,
        Directive::Note(it) => &it.meta,
        Directive::Document(it) => &it.meta,
        Directive::Price(it) => &it.meta,
        Directive::Event(it) => &it.meta,
        Directive::Custom(it) => &it.meta,
        Directive::Query(it) => &it.meta,
        Directive::Plugin(it) => &it.meta,
        Directive::Budget(it) => &it.meta,
        Directive::BudgetAdd(it) => &it.meta,
        Directive::BudgetTransfer(it) => &it.meta,
        Directive::BudgetClose(it) => &it.meta,
        Directive::Option(_) | Directive::Include(_) | Directive::Comment(_) => return None,
    })
}

/// A row of any table, as the executor reads it.
#[derive(Clone, Copy)]
pub(crate) enum RowRef<'r, 'a> {
    Posting(&'r Row<'a>),
    Record(&'r Record<'a>),
}

/// The rows of one query execution plus lazily built lookup structures.
///
/// The rows only carry what [`Dataset::projection`] reads: evaluating a column outside of
/// it would see a pruned (empty) cost or price.
pub(crate) struct Dataset<'a> {
    /// the table the rows belong to
    pub table: &'static Table,
    /// the transactions of the `postings` rows
    pub entries: Vec<Entry<'a>>,
    /// the rows of the `postings` table
    pub rows: Vec<Row<'a>>,
    /// the rows of a record table
    pub records: Vec<Record<'a>>,
    pub today: NaiveDate,
    pub projection: Projection,
    ledger: &'a Ledger,
    store: &'a Store,
    /// what every query of the ledger shares (see [`LedgerCache`])
    cache: &'a LedgerCache,
    store_meta: OnceCell<HashMap<&'a str, Vec<(&'a str, &'a str)>>>,
}

impl<'a> Dataset<'a> {
    /// The rows of the projection's table (of the `postings` table, those in `scope`);
    /// generated rows count against `limits`.
    pub fn build(
        ledger: &'a Ledger, store: &'a Store, today: NaiveDate, projection: Projection, scope: &Scope, limits: &mut Limits<'_>,
    ) -> Result<Self, LocatedError> {
        let cache = LedgerCache::of(ledger, store);
        let records = match projection.table().rows {
            Rows::Postings => return Ok(Dataset::postings(ledger, store, cache, today, projection, scope)),
            Rows::Records(source) => source(ledger, store, projection),
            Rows::Generated(source) => source(ledger, store, projection, limits)?,
        };
        Ok(Dataset {
            table: projection.table(),
            entries: vec![],
            rows: vec![],
            records,
            today,
            projection,
            ledger,
            store,
            cache,
            store_meta: OnceCell::new(),
        })
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        match self.table.rows {
            Rows::Postings => self.rows.len(),
            Rows::Records(_) | Rows::Generated(_) => self.records.len(),
        }
    }

    /// The row at `idx`.
    pub fn row(&self, idx: usize) -> RowRef<'_, 'a> {
        match self.table.rows {
            Rows::Postings => RowRef::Posting(&self.rows[idx]),
            Rows::Records(_) | Rows::Generated(_) => RowRef::Record(&self.records[idx]),
        }
    }

    /// The rows, in table order.
    pub fn iter(&self) -> impl Iterator<Item = RowRef<'_, 'a>> {
        (0..self.len()).map(|idx| self.row(idx))
    }

    /// The ledger's price map, built once per loaded ledger.
    pub fn prices(&self) -> &PriceMap {
        self.cache.prices(self.store)
    }

    /// The `#entries` / `#transactions` rows of the ledger.
    pub(crate) fn entry_table(&self) -> &'a cache::Entries {
        self.cache.entries(self.ledger, self.store)
    }

    /// The `id` of the `#entries` row `seq`.
    pub(crate) fn entry_id(&self, seq: u32) -> &'a str {
        self.cache.entry_id(self.ledger, self.entry_table(), seq)
    }

    /// The `open` and `close` directives of `account`, for `open_date()`, `open_meta()`, ...
    pub fn account_directives(&self, account: &str) -> Option<AccountDirectives<'a>> {
        self.cache.lookups(self.ledger, self.store).account(self.ledger, account)
    }

    /// The `commodity` directive of `currency`, for `commodity_meta()`.
    pub fn commodity_directive(&self, currency: &str) -> Option<&'a Commodity> {
        self.cache.lookups(self.ledger, self.store).commodity(self.ledger, currency)
    }

    /// What `meta(key)` reads at `row`: the posting's metadata, or a record's own metadata.
    pub fn row_meta(&self, row: RowRef<'_, '_>, key: &str) -> Option<String> {
        match row {
            RowRef::Posting(row) => self.posting_meta(row, key),
            RowRef::Record(record) => record.meta(key),
        }
    }

    /// What `entry_meta(key)` reads at `row`: the metadata of the posting's transaction, or a
    /// record's own metadata (a record is its own entry).
    pub fn row_entry_meta(&self, row: RowRef<'_, '_>, key: &str) -> Option<String> {
        match row {
            RowRef::Posting(row) => self.entry_meta(row, key),
            RowRef::Record(record) => record.meta(key),
        }
    }

    /// What `meta_values(key)` reads at `row`: every value of the key, as `meta(key)` reads
    /// the first.
    pub fn row_meta_values(&self, row: RowRef<'_, '_>, key: &str) -> Vec<String> {
        match row {
            RowRef::Posting(row) => self
                .posting_metas(row)
                .iter()
                .filter(|meta| meta.key == key)
                .map(|meta| meta.value.clone())
                .collect(),
            RowRef::Record(record) => record.meta_values(key),
        }
    }

    /// What `entry_meta_values(key)` reads at `row`: every value of the key, as
    /// `entry_meta(key)` reads the first.
    pub fn row_entry_meta_values(&self, row: RowRef<'_, '_>, key: &str) -> Vec<String> {
        match row {
            RowRef::Posting(row) => self.entry_meta_values(row, key),
            RowRef::Record(record) => record.meta_values(key),
        }
    }
}
