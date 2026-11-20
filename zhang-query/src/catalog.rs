//! The catalog: the tables a query compiles against, and the handles a compiled query keeps to
//! them.
//!
//! A compiled query holds no `&'static` reference to a table or a column but a handle: a
//! [`TableRef`] for a table, a [`ColumnRef`] for one column of it (the table and the column's
//! index). The compiler resolves names through a [`Catalog`]; the optimizer, the projector and
//! the executor only see the handles. Today every table is a core table, a `static` [`Table`]
//! of [`crate::table`], and [`Catalog::core`] is the catalog every query compiles against
//! ([`crate::Query::compile_in`]). The handles leave room for tables that are not `static`,
//! such as those a ledger's plugins would provide, served by a catalog of that ledger.

use std::fmt;
use std::sync::LazyLock;

use crate::table::{self, Borrow, ColumnDef, Dataset, RowRef, Rows, Table};
use crate::value::{DataType, Value};

/// The tables a query can read: the default `postings` table first, then the tables `FROM #name`
/// names, in the order the schema lists them.
#[derive(Debug)]
pub struct Catalog {
    /// never empty: `postings` is the first table
    tables: Vec<TableRef>,
}

impl Catalog {
    /// The core tables: the catalog [`crate::Query::compile`] compiles against.
    pub fn core() -> &'static Catalog {
        static CORE: LazyLock<Catalog> = LazyLock::new(|| Catalog::of(table::tables().iter().map(|table| TableRef::Static(table))));
        &CORE
    }

    /// The catalog of `tables`, the first of which is the `postings` table.
    fn of(tables: impl IntoIterator<Item = TableRef>) -> Catalog {
        let tables = tables.into_iter().collect::<Vec<_>>();
        assert!(tables.first().is_some_and(TableRef::is_postings), "a catalog starts with the postings table");
        Catalog { tables }
    }

    /// The core tables and `extra` after them: a catalog with test-only tables.
    #[cfg(test)]
    pub(crate) fn core_and(extra: impl IntoIterator<Item = TableRef>) -> Catalog {
        Catalog::of(Catalog::core().tables.iter().cloned().chain(extra))
    }

    /// Every table, `postings` first.
    pub fn tables(&self) -> &[TableRef] {
        &self.tables
    }

    /// The table a query without `FROM #name` reads.
    pub(crate) fn postings(&self) -> &TableRef {
        &self.tables[0]
    }

    /// The table `name` (without `#`). Table names are case-sensitive, as in beanquery.
    pub(crate) fn find(&self, name: &str) -> Option<&TableRef> {
        self.tables.iter().find(|table| table.name() == name)
    }
}

/// A handle to a table of a [`Catalog`]. Two handles are equal when they are handles to the same
/// table.
#[derive(Clone)]
#[non_exhaustive]
pub enum TableRef {
    /// a core table
    Static(&'static Table),
}

impl TableRef {
    #[inline]
    fn table(&self) -> &Table {
        match self {
            TableRef::Static(table) => table,
        }
    }

    /// The name after `#`.
    pub fn name(&self) -> &str {
        self.table().name
    }

    pub fn description(&self) -> &str {
        self.table().description
    }

    /// Whether this is the `postings` table, the default of a query without `FROM #name`.
    pub fn is_postings(&self) -> bool {
        self.table().is_postings()
    }

    /// The columns, in schema order; a [`ColumnRef`]'s index is its position here.
    #[inline]
    pub(crate) fn columns(&self) -> &[ColumnDef] {
        self.table().columns
    }

    /// The column `name` (case-insensitive).
    pub(crate) fn column(&self, name: &str) -> Option<ColumnRef> {
        self.table().column_index(name).map(|index| self.column_at(index))
    }

    /// The column at `index` of [`TableRef::columns`].
    pub(crate) fn column_at(&self, index: usize) -> ColumnRef {
        match self {
            TableRef::Static(table) => ColumnRef {
                table: self.clone(),
                index,
                def: &table.columns[index],
            },
        }
    }

    /// The columns `SELECT *` expands to, in order.
    pub(crate) fn wildcard(&self) -> &[&str] {
        self.table().wildcard
    }

    /// The attributes of the structured column `base`; see [`Table::attributes`].
    pub(crate) fn attributes(&self, base: &str) -> Vec<&str> {
        self.table().attributes(base)
    }

    /// How the table produces its rows.
    #[inline]
    pub(crate) fn rows(&self) -> &Rows {
        &self.table().rows
    }
}

impl PartialEq for TableRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.table(), other.table())
    }
}

impl Eq for TableRef {}

/// `#name`
impl fmt::Debug for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.name())
    }
}

/// A handle to a column of a table: the table and the column's position among its columns.
#[derive(Clone)]
pub(crate) struct ColumnRef {
    table: TableRef,
    index: usize,
    /// the column of a core table, resolved when the handle is made: the executor reads a
    /// column for every row, and this keeps that read one load away, as it was with a
    /// `&'static ColumnDef` (`table.columns()[index]` would be a chain of three)
    def: &'static ColumnDef,
}

impl ColumnRef {
    /// The table of the column.
    pub fn table(&self) -> &TableRef {
        &self.table
    }

    /// The position of the column among the columns of its table.
    pub fn index(&self) -> usize {
        self.index
    }

    #[inline]
    fn def(&self) -> &ColumnDef {
        self.def
    }

    pub fn name(&self) -> &str {
        self.def().name
    }

    pub fn ty(&self) -> DataType {
        self.def().ty
    }

    /// How predicates can read the column in place.
    #[inline]
    pub fn borrow(&self) -> Borrow {
        self.def().borrow
    }

    /// The value of the column at `row`; see [`ColumnDef::value`].
    #[inline]
    pub fn value(&self, data: &Dataset<'_>, row: RowRef<'_, '_>) -> Value {
        self.def().value(data, row)
    }
}

/// `#table.name: type`
impl fmt::Debug for ColumnRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}.{:?}", self.table, self.def())
    }
}

impl Table {
    /// A handle to the column `name` (case-insensitive) of this core table.
    pub(crate) fn column_ref(&'static self, name: &str) -> Option<ColumnRef> {
        TableRef::Static(self).column(name)
    }
}
