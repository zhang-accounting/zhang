//! Tables: the row sources a query reads, selected with `FROM #name`.
//!
//! A core table is a static [`Table`] in the registry ([`tables`]): a name, a description,
//! typed columns ([`ColumnDef`]), the columns `SELECT *` expands to, and a row source. A catalog
//! adds the tables and the `postings` columns its extensions provide (`crate::extension`). The
//! compiler resolves the query's table in its catalog ([`crate::Catalog`]) first and then its
//! column names against that table only, into handles ([`crate::catalog`]); the projector
//! prunes the table's columns; the executor reads rows through
//! [`RowRef`], so every later stage (filters, grouping, ordering, LIMIT, the result budget
//! and the deadline) works the same way for every table.
//!
//! There are three kinds of row sources:
//!
//! - [`Rows::Postings`]: the `postings` table ([`postings`]), the default when a query names
//!   no table. Its rows are booked postings ([`Row`]), assembled from the rows booked once
//!   per loaded ledger ([`LedgerCache`]), and only those of the accounts the query is scoped
//!   to ([`Scope`]).
//! - [`Rows::Records`]: every other table. A builder walks the ledger once and returns one
//!   [`Record`] per row, which only borrows the ledger; its columns are computed
//!   when an expression reads them. A builder may skip work that only unprojected columns
//!   need (see [`Projection::contains`]).
//! - [`Rows::Extension`]: the tables of a provider (`crate::extension`), whose rows are its
//!   snapshot of the ledger: the rows of a record table, or the periods of a series table, which
//!   are not read from the ledger but generated, such as the months of `#budgets`, so that their
//!   number is not bounded by the size of the ledger. The generation charges every row to the
//!   result budget and checks the deadline while it builds them ([`Limits`]), before any filter
//!   runs.
//!
//! A column named `base.attribute` is an attribute of the structured value `base`, read
//! with attribute access (`open.date` in `#accounts`); `base` itself is a column too.
//!
//! # Adding a table
//!
//! Add a module with a `static` [`Table`] whose columns read a [`Record`] variant (add one
//! when no existing variant fits), register it in [`TABLES`], and describe it in the query
//! language reference. Its columns appear in `GET /api/query/schema` automatically. A table
//! that is no part of the core, such as the budget tables, is an extension's: its provider
//! declares it in a manifest and publishes its rows in a snapshot (`crate::extension`).

mod accounts;
mod cache;
mod directives;
mod entries;
mod errors;
mod lookups;
mod postings;
mod prices;

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use zhang_ast::{Commodity, Directive, Meta, Spanned};
use zhang_core::domains::schemas::AccountStatus;
use zhang_core::ledger::Ledger;

pub(crate) use self::cache::{Head, LedgerCache};
pub(crate) use self::directives::date_of;
pub(crate) use self::entries::entry_type;
pub use self::postings::COLUMNS;
pub(crate) use self::postings::{position, time_value, Entry, MaybeOwned, Row, Scope, ACCOUNT_BALANCE_COLUMN, BALANCE_COLUMN};
#[cfg(test)]
use crate::catalog::ColumnRef;
use crate::catalog::TableRef;
use crate::error::LocatedError;
use crate::executor::{Budget, Deadline};
use crate::extension::{self, ExtensionKind, ExtensionRows, LabelColumn, Labels, RecordRow, SeriesRow, TableData};
use crate::functions::AccountDirectives;
use crate::prices::PriceMap;
use crate::projector::Projection;
use crate::value::{DataType, Value};

/// A table a query can read with `FROM #name`.
///
/// A core table is a `static` whose names and columns are `'static` too; a table a catalog builds at run time (see
/// [`crate::catalog::TableRef::Shared`]) owns them. Either way the handles of [`crate::catalog`] read them through
/// this one type.
pub struct Table {
    /// the name after `#`, lower-case; names are case-sensitive
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub columns: Cow<'static, [ColumnDef]>,
    /// the columns `SELECT *` expands to, in order
    pub(crate) wildcard: Names,
    pub(crate) rows: Rows,
    /// the provider of an extension's table; `None` for a core table
    pub provided_by: Option<Arc<str>>,
    /// the label columns among the columns of a `postings` table, by slot ([`label_getter`])
    pub(crate) labels: Vec<Arc<LabelColumn>>,
}

/// Names a table keeps: `'static` ones of a core table, or owned ones.
#[derive(Debug, Clone)]
pub(crate) enum Names {
    Static(&'static [&'static str]),
    Owned(Vec<String>),
}

impl Names {
    /// The names, in order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let (borrowed, owned) = match self {
            Names::Static(names) => (*names, &[][..]),
            Names::Owned(names) => (&[][..], names.as_slice()),
        };
        borrowed.iter().copied().chain(owned.iter().map(String::as_str))
    }
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.name)
    }
}

impl Table {
    /// The column `name` (case-insensitive).
    pub fn column(&self, name: &str) -> Option<&ColumnDef> {
        self.column_index(name).map(|index| &self.columns[index])
    }

    /// The position of the column `name` (case-insensitive) among the columns.
    pub(crate) fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|it| it.name.eq_ignore_ascii_case(name))
    }

    /// Whether this is a `postings` table, whose rows are the booked postings: the default of a query without
    /// `FROM #name`. It is the core's [`POSTINGS`], or a copy of it with more columns that a catalog builds.
    pub fn is_postings(&self) -> bool {
        matches!(self.rows, Rows::Postings)
    }

    /// The attributes of the structured column `base` (the columns named `base.attribute`),
    /// in column order; empty when `base` has none.
    pub(crate) fn attributes(&self, base: &str) -> Vec<&str> {
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
    /// the rows of a provider's snapshot of the ledger: a record table's, or the periods of a series table generated
    /// within [`Limits`] (see [`crate::extension`])
    Extension(ExtensionRows),
}

/// Builds the rows of a record table, in the table's row order.
pub(crate) type RecordSource = for<'a> fn(&'a Ledger, &Projection) -> Vec<Record<'a>>;

/// What the generation of the periods of a series table needs to know of the execution: `today()` and the last date a
/// row needs to have (see [`crate::optimizer::date_bound`]); rows dated after it are not generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Generation {
    /// `today()` of the execution
    pub today: NaiveDate,
    /// the last date the rows the filter keeps can have; `None` generates every row
    pub until: Option<NaiveDate>,
}

/// The limits of one execution as the generation of a series sees them: every generated row costs
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
    name: Cow::Borrowed("postings"),
    description: Cow::Borrowed("One row per posting of every transaction, booked against the lots of its account (the default table)."),
    columns: Cow::Borrowed(COLUMNS),
    wildcard: Names::Static(&postings::WILDCARD_COLUMNS),
    rows: Rows::Postings,
    provided_by: None,
    labels: Vec::new(),
};

/// Every core table, in the order the schema lists them; a catalog lists the tables of its extensions before the
/// last ones ([`TRAILING`]).
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
    &errors::ERRORS,
];

/// How many of the core tables come after the extensions' tables in a catalog: `#errors`, which the schema has
/// always listed last.
pub(crate) const TRAILING: usize = 1;

/// Every core table a query can read, `postings` first: the tables of [`crate::Catalog::core`].
pub fn tables() -> &'static [&'static Table] {
    TABLES
}

/// The column `name` of the `postings` table of the catalog of every ledger ([`crate::Catalog::standard`]).
#[cfg(test)]
pub(crate) fn column(name: &str) -> Option<ColumnRef> {
    crate::Catalog::standard().postings().column(name)
}

/// A column of a table.
#[derive(Clone)]
pub struct ColumnDef {
    pub name: Cow<'static, str>,
    pub ty: DataType,
    pub description: Cow<'static, str>,
    pub(crate) get: Get,
    /// how predicates can read the column in place, without copying it into a [`Value`]
    /// (postings only)
    pub(crate) borrow: Borrow,
    /// the provider of an extension's column; `None` for a core column
    pub provided_by: Option<Arc<str>>,
}

impl fmt::Debug for ColumnDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.ty)
    }
}

impl ColumnDef {
    /// A column of a record table.
    pub(crate) const fn record(name: &'static str, ty: DataType, description: &'static str, get: RecordGetter) -> ColumnDef {
        ColumnDef {
            name: Cow::Borrowed(name),
            ty,
            description: Cow::Borrowed(description),
            get: Get::Record(get),
            borrow: Borrow::No,
            provided_by: None,
        }
    }

    /// A column of the `postings` table.
    pub(crate) const fn posting(name: &'static str, ty: DataType, description: &'static str, get: PostingGetter) -> ColumnDef {
        ColumnDef {
            name: Cow::Borrowed(name),
            ty,
            description: Cow::Borrowed(description),
            get: Get::Posting(get),
            borrow: Borrow::No,
            provided_by: None,
        }
    }

    /// A column of the `postings` table that predicates read in place through `borrow`.
    pub(crate) const fn posting_borrowing(name: &'static str, ty: DataType, description: &'static str, get: PostingGetter, borrow: Borrow) -> ColumnDef {
        ColumnDef {
            name: Cow::Borrowed(name),
            ty,
            description: Cow::Borrowed(description),
            get: Get::Posting(get),
            borrow,
            provided_by: None,
        }
    }
}

/// How a column reads its value from a row: a getter, as a plain function pointer. The columns of the extensions have
/// getters too, one per position of a cell ([`cell_getter`]) and per slot of a label column ([`label_getter`]), so that
/// reading any column is the one indirect call it has always been.
#[derive(Clone, Copy)]
pub(crate) enum Get {
    /// a column of a `postings` table
    Posting(PostingGetter),
    /// a column of a record table
    Record(RecordGetter),
}

/// The getter of a column of a `postings` table.
pub(crate) type PostingGetter = for<'a> fn(&Dataset<'a>, &Row<'a>) -> Value;

/// The getter of a column of a record table.
pub(crate) type RecordGetter = fn(&Dataset<'_>, &Record<'_>) -> Value;

/// How many columns an extension's table can have: the getters of its cells, by position.
pub(crate) const MAX_CELLS: usize = 64;

/// How many label columns a catalog can have: their getters, by slot.
pub(crate) const MAX_LABELS: usize = 16;

/// The getter of the cell at `index` of the rows of an extension's table; `None` past [`MAX_CELLS`].
pub(crate) fn cell_getter(index: usize) -> Option<RecordGetter> {
    fn cell<const INDEX: usize>(_: &Dataset<'_>, record: &Record<'_>) -> Value {
        record.cell(INDEX)
    }
    macro_rules! cells {
        ($($index:literal)*) => { [$(cell::<$index> as RecordGetter),*] };
    }
    static CELLS: [RecordGetter; MAX_CELLS] = cells!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63);
    CELLS.get(index).copied()
}

/// The membership test of a set column of a `postings` table, read in place ([`Borrow::Contains`]).
pub(crate) type ContainsTest = for<'a> fn(&Dataset<'a>, &Row<'a>, &str) -> bool;

/// The getter and the membership test of the label column at `slot` of a `postings` table ([`Table::labels`]);
/// `None` past [`MAX_LABELS`].
pub(crate) fn label_getter(slot: usize) -> Option<(PostingGetter, ContainsTest)> {
    fn labels<'a, const SLOT: u16>(data: &Dataset<'a>, row: &Row<'a>) -> Value {
        Value::Set(data.labels(SLOT).of_row(data, row))
    }
    fn holds<'a, const SLOT: u16>(data: &Dataset<'a>, row: &Row<'a>, label: &str) -> bool {
        data.labels(SLOT).holds(data, row, label)
    }
    macro_rules! labels {
        ($($slot:literal)*) => { [$((labels::<$slot> as PostingGetter, holds::<$slot> as ContainsTest)),*] };
    }
    static LABELS: [(PostingGetter, ContainsTest); MAX_LABELS] = labels!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
    LABELS.get(slot).copied()
}

impl Get {
    /// The value at `row`. A column only reads rows of its own table (the compiler resolves columns against the
    /// query's table); any other row reads as NULL.
    ///
    /// No `#[inline]`, here and in [`crate::catalog::ColumnRef::value`]: with the hint, the release build (`opt-level =
    /// "s"`) builds the getter's value in a temporary and copies it into the executor's result, a `Value` per column
    /// read (about 22 instructions); left to the compiler, the getter writes into the result, as it did before the
    /// handles kept their getters.
    pub(crate) fn value<'a>(self, data: &Dataset<'a>, row: RowRef<'_, 'a>) -> Value {
        match (self, row) {
            (Get::Posting(get), RowRef::Posting(row)) => get(data, row),
            (Get::Record(get), RowRef::Record(record)) => get(data, record),
            _ => Value::Null,
        }
    }
}

/// In-place access to a column, for predicates that only inspect the value.
#[derive(Clone, Copy)]
pub(crate) enum Borrow {
    /// the column is only available as a [`Value`]
    No,
    /// a string column, read without copying; `None` is NULL. `get` returns the same string.
    Str(for<'r> fn(&'r Dataset<'_>, &'r Row<'_>) -> Option<&'r str>),
    /// a set column, as a membership test that does not build the set; of a label column, one that evaluates the
    /// predicate of the label asked for only ([`label_getter`])
    Contains(ContainsTest),
}

/// One row of a record table. It only borrows the ledger; the table's columns compute their
/// values from it on access.
pub(crate) enum Record<'a> {
    /// a row of `#entries`, or of a table of one kind of directive: a directive with its place in the ledger
    Entry {
        directive: &'a Spanned<Directive>,
        info: &'a cache::EntryInfo,
    },
    /// a document: a `document` directive, or a `document` metadata value
    Document(directives::DocumentRow<'a>),
    /// an account with its `open` and `close` directives (at least one of them)
    Account {
        name: &'a str,
        open: Option<&'a Spanned<Directive>>,
        close: Option<&'a Spanned<Directive>>,
        /// the booking method the account books with, as booking resolves its `open`s; `None`
        /// when it books with the ledger's default
        booking: Option<zhang_core::inventory::BookingMethod>,
    },
    /// a row of an extension's record table
    Extension(&'a RecordRow),
    /// a period of an extension's series table
    Series(SeriesRow<'a>),
    /// a ledger error
    Error(errors::LedgerError<'a>),
}

impl<'a> Record<'a> {
    /// The metadata of the row: what `meta()`, `entry_meta()` and `any_meta()` read on a
    /// record table (an account reads the metadata of its `open`, else of its `close`).
    fn metadata(&self) -> Option<&'a Meta> {
        match self {
            Record::Entry { directive, .. } => directive.data.meta(),
            Record::Account { open, close, .. } => open.or(*close).and_then(|directive| directive.data.meta()),
            Record::Document(document) => Some(document.metadata()),
            Record::Extension(row) => row.meta.as_deref(),
            Record::Series(row) => row.metadata(),
            // an error's details are not directive metadata (see `meta`)
            Record::Error(_) => None,
        }
    }

    /// The cell at `index` of an extension's row, as a series fills it in a generated period; NULL on any other row.
    fn cell(&self, index: usize) -> Value {
        match self {
            // a snapshot is checked before it is read: every row has a cell per column ([`extension::snapshot`])
            Record::Extension(row) => row.cells[index].clone(),
            Record::Series(row) => row.cell(index),
            _ => Value::Null,
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
    meta.cloned().map(Meta::sorted_pairs).unwrap_or_default()
}

/// Metadata as text, the value of the `meta` columns: `key: "value"` pairs sorted by key and
/// separated by `, ` (the values of a repeated key in ledger order); `''` without metadata.
/// Quotes and backslashes in values are escaped with a backslash.
pub(crate) fn render_meta(meta: Option<&Meta>) -> Value {
    render_pairs(meta_pairs(meta).iter().map(|(key, value)| (key.as_str(), value.as_str())))
}

/// Metadata `pairs`, already sorted as [`Meta::sorted_pairs`] sorts them, as text, as [`render_meta`] writes it.
pub(crate) fn render_pairs<'p>(pairs: impl Iterator<Item = (&'p str, &'p str)>) -> Value {
    let text = pairs
        .map(|(key, value)| format!("{}: \"{}\"", key, value.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    Value::Str(text)
}

/// The file `path` of the ledger by its path within the ledger, as the UI's file list names it
/// ([`Ledger::path_in_ledger`]), or the path as it is if it is outside.
pub(crate) fn ledger_file<'a>(ledger: &Ledger, path: &'a Path) -> Cow<'a, Path> {
    match ledger.path_in_ledger(path).filter(|it| !it.as_os_str().is_empty()) {
        Some(within) => Cow::Owned(within),
        None => Cow::Borrowed(path),
    }
}

/// A row of any table, as the executor reads it.
#[derive(Clone, Copy)]
pub(crate) enum RowRef<'r, 'a> {
    Posting(&'r Row<'a>),
    Record(&'r Record<'a>),
}

/// The rows of one query execution plus lazily built lookup structures.
pub(crate) struct Dataset<'a> {
    /// whether the rows are `postings` rows (`rows`) rather than records (`records`): what every row access asks,
    /// kept here so that it reads no table
    postings: bool,
    /// the transactions of the `postings` rows
    pub entries: Vec<Entry<'a>>,
    /// the rows of the `postings` table
    pub rows: Vec<Row<'a>>,
    /// the rows of a record table
    pub records: Vec<Record<'a>>,
    pub today: NaiveDate,
    /// `now()` of the execution: the instant of the ledger's clock in its timezone, whose date is `today`
    pub now: NaiveDateTime,
    /// the columns of the table the execution reads
    pub projection: &'a Projection,
    ledger: &'a Ledger,
    /// what every query of the ledger shares (see [`LedgerCache`])
    cache: &'a LedgerCache,
    /// the label columns of the table, by slot, each resolved on its first read
    labels: Box<[OnceCell<Labels<'a>>]>,
}

impl<'a> Dataset<'a> {
    /// The rows of the projection's table (of the `postings` table, those in `scope`);
    /// generated rows count against `limits`.
    pub fn build(
        ledger: &'a Ledger, now: NaiveDateTime, projection: &'a Projection, scope: &Scope, until: Option<NaiveDate>, limits: &mut Limits<'_>,
    ) -> Result<Self, LocatedError> {
        let cache = LedgerCache::of(ledger);
        let today = now.date();
        let records = match projection.table().rows() {
            Rows::Postings => return Ok(Dataset::postings(ledger, cache, now, projection, scope)),
            Rows::Records(source) => source(ledger, projection),
            Rows::Extension(rows) => {
                let table = projection.table();
                let data = extension::snapshot::table_data(ledger, rows, table.name()).map_err(|message| LocatedError::eval(message, None))?;
                match (&rows.kind, data) {
                    (_, None) => vec![],
                    (ExtensionKind::Records, Some(TableData::Records(records))) => records.iter().map(Record::Extension).collect(),
                    (ExtensionKind::Series(spec), Some(TableData::Series(partitions))) => {
                        extension::series::rows(ledger, table, &rows.provider, spec, partitions, Generation { today, until }, limits)?
                    }
                    _ => {
                        return Err(LocatedError::eval(
                            format!("table #{} is unavailable: {} published other rows", table.name(), rows.provider),
                            None,
                        ))
                    }
                }
            }
        };
        Ok(Dataset {
            postings: false,
            entries: vec![],
            rows: vec![],
            records,
            today,
            now,
            projection,
            ledger,
            cache,
            labels: Box::new([]),
        })
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        if self.postings {
            self.rows.len()
        } else {
            self.records.len()
        }
    }

    /// The row at `idx`.
    pub fn row(&self, idx: usize) -> RowRef<'_, 'a> {
        if self.postings {
            RowRef::Posting(&self.rows[idx])
        } else {
            RowRef::Record(&self.records[idx])
        }
    }

    /// The rows, in table order.
    pub fn iter(&self) -> impl Iterator<Item = RowRef<'_, 'a>> {
        (0..self.len()).map(|idx| self.row(idx))
    }

    /// The ledger's price map, built once per loaded ledger.
    pub fn prices(&self) -> &PriceMap {
        self.cache.prices(self.ledger)
    }

    /// The label column at `slot` of the table, resolved for this execution on its first read: the labels of its
    /// provider's snapshot of the ledger, the predicate bound to each. A provider that was skipped gives no label; a
    /// query does not read the column of one that failed ([`extension::snapshot::check_label_column`]).
    pub(crate) fn labels(&self, slot: u16) -> &Labels<'a> {
        self.labels[slot as usize].get_or_init(|| {
            let column = &self.projection.table().labels()[slot as usize];
            Labels::resolve(column, extension::snapshot::label_data(self.ledger, column))
        })
    }

    /// The label columns of a `postings` table, unresolved, for a new execution.
    pub(crate) fn unresolved_labels(table: &TableRef) -> Box<[OnceCell<Labels<'a>>]> {
        table.labels().iter().map(|_| OnceCell::new()).collect()
    }

    /// The `#entries` / `#transactions` rows of the ledger.
    pub(crate) fn entry_table(&self) -> &'a cache::Entries {
        self.cache.entries(self.ledger)
    }

    /// The kinds of the errors recorded for the transaction of the `#entries` row `seq`, when there are any.
    pub(crate) fn entry_errors(&self, seq: u32) -> Option<&'a BTreeSet<String>> {
        self.cache.entry_errors(self.ledger).get(seq as usize).and_then(Option::as_ref)
    }

    /// The `id` of the `#entries` row `seq`.
    pub(crate) fn entry_id(&self, seq: u32) -> &'a str {
        self.cache.entry_id(self.ledger, self.entry_table(), seq)
    }

    /// The `open` and `close` directives of `account`, for `open_date()`, `open_meta()`, ...
    pub fn account_directives(&self, account: &str) -> Option<AccountDirectives<'a>> {
        self.cache.lookups(self.ledger).account(self.ledger, account)
    }

    /// The budgets a posting of `account` at the wall-clock time `at` counts in, for `account_budgets()`: the ledger's
    /// one rule of budget membership ([`Ledger::account_budgets`]).
    pub fn account_budgets(&self, account: &str, at: NaiveDateTime) -> Option<&'a BTreeSet<String>> {
        self.ledger.account_budgets(account, at)
    }

    /// The status of `account` at `at`, for `account_status()`: the ledger's account lifecycle.
    pub fn account_status(&self, account: &str, at: NaiveDateTime) -> Option<AccountStatus> {
        self.ledger.account_status(account, at)
    }

    /// The `commodity` directive of `currency`, for `commodity_meta()`.
    pub fn commodity_directive(&self, currency: &str) -> Option<&'a Commodity> {
        self.cache.lookups(self.ledger).commodity(self.ledger, currency)
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
            RowRef::Posting(row) => self.posting_meta_values(row, key),
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
