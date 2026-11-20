//! The catalog: the tables a query compiles against, and the handles a compiled query keeps to
//! them.
//!
//! A compiled query holds no `&'static` reference to a table or a column but a handle: a
//! [`TableRef`] for a table, and for one column of it a handle that keeps the table and the column's
//! index. The compiler resolves names through a [`Catalog`]; the optimizer, the projector and
//! the executor only see the handles. A table is a core table, a `static` [`Table`] of
//! [`crate::table`] ([`TableRef::Static`]), or one a catalog builds at run time and shares
//! ([`TableRef::Shared`]), whose names and columns it owns. [`Catalog::core`] is the catalog every
//! query compiles against ([`crate::Query::compile_in`]).

use std::fmt;
use std::sync::{Arc, LazyLock};

use crate::table::{self, Borrow, ColumnDef, Dataset, Get, RowRef, Rows, Table};
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

    /// The catalog of `tables`, the first of which is a `postings` table (the only one).
    fn of(tables: impl IntoIterator<Item = TableRef>) -> Catalog {
        let tables = tables.into_iter().collect::<Vec<_>>();
        assert!(tables.first().is_some_and(TableRef::is_postings), "a catalog starts with the postings table");
        assert!(!tables[1..].iter().any(TableRef::is_postings), "a catalog has one postings table");
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
    /// a table a catalog built, which owns its names and columns: every handle to it shares it
    Shared(Arc<Table>),
}

impl TableRef {
    #[inline]
    fn table(&self) -> &Table {
        match self {
            TableRef::Static(table) => table,
            TableRef::Shared(table) => table,
        }
    }

    /// The name after `#`.
    pub fn name(&self) -> &str {
        &self.table().name
    }

    pub fn description(&self) -> &str {
        &self.table().description
    }

    /// Whether this is the `postings` table, the default of a query without `FROM #name`.
    pub fn is_postings(&self) -> bool {
        self.table().is_postings()
    }

    /// The columns, in schema order; a column handle's index is its position here.
    #[inline]
    pub(crate) fn columns(&self) -> &[ColumnDef] {
        &self.table().columns
    }

    /// The column `name` (case-insensitive).
    pub(crate) fn column(&self, name: &str) -> Option<ColumnRef> {
        self.table().column_index(name).map(|index| self.column_at(index))
    }

    /// The column at `index` of [`TableRef::columns`].
    pub(crate) fn column_at(&self, index: usize) -> ColumnRef {
        let def = &self.columns()[index];
        ColumnRef {
            table: self.clone(),
            index,
            ty: def.ty,
            get: def.get,
            borrow: def.borrow,
        }
    }

    /// The columns `SELECT *` expands to, in order.
    pub(crate) fn wildcard(&self) -> impl Iterator<Item = &str> {
        self.table().wildcard.iter()
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
///
/// It also keeps, by value, what reading the column needs: its type, its getter and its in-place
/// reader, plain function pointers copied from the column when the handle is made. The executor
/// reads a column for every row, and this keeps that read one load away whatever the table is,
/// with no branch on the kind of table.
#[derive(Clone)]
pub(crate) struct ColumnRef {
    table: TableRef,
    index: usize,
    ty: DataType,
    get: Get,
    borrow: Borrow,
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

    /// The column's definition in its table: its name and description.
    fn def(&self) -> &ColumnDef {
        &self.table.columns()[self.index]
    }

    pub fn name(&self) -> &str {
        &self.def().name
    }

    pub fn ty(&self) -> DataType {
        self.ty
    }

    /// How predicates can read the column in place.
    #[inline]
    pub fn borrow(&self) -> Borrow {
        self.borrow
    }

    /// The value of the column at `row`; see [`Get::value`].
    #[inline]
    pub fn value(&self, data: &Dataset<'_>, row: RowRef<'_, '_>) -> Value {
        self.get.value(data, row)
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
    #[cfg(test)]
    pub(crate) fn column_ref(&'static self, name: &str) -> Option<ColumnRef> {
        TableRef::Static(self).column(name)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;

    use chrono::NaiveDate;

    use super::*;
    use crate::table::{Names, POSTINGS};
    use crate::{ParamTypes, Params, Query};

    fn ledger() -> zhang_core::ledger::Ledger {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        let text = "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n\n\
                    2024-01-02 * \"Cafe\" \"lunch\"\n  Expenses:Food 12.50 USD\n  Assets:Bank -12.50 USD\n";
        std::fs::write(dir.join("main.zhang"), text).expect("write ledger");
        let source = zhang_core::data_source::LocalFileSystemDataSource::new(zhang_core::data_type::text::ZhangDataType {});
        zhang_core::ledger::Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger")
    }

    fn run(catalog: &Catalog, sql: &str) -> Vec<Vec<Value>> {
        let query = Query::compile_in(catalog, sql, &ParamTypes::default()).unwrap_or_else(|err| panic!("{sql}: {err}"));
        query
            .execute_at(&ledger(), &Params::new(), NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
            .unwrap()
            .rows
    }

    /// A copy of a core table that owns its name, description, columns and wildcard, shared by its handles.
    fn owned_copy(table: &Table, name: &str, extra: Option<ColumnDef>) -> TableRef {
        let rows = match &table.rows {
            Rows::Postings => Rows::Postings,
            Rows::Records(source) => Rows::Records(*source),
            Rows::Generated(source) => Rows::Generated(*source),
        };
        TableRef::Shared(Arc::new(Table {
            name: Cow::Owned(name.to_owned()),
            description: Cow::Owned(format!("{} (a copy)", table.description)),
            columns: Cow::Owned(table.columns.iter().cloned().chain(extra).collect()),
            wildcard: Names::Owned(table.wildcard.iter().map(str::to_owned).collect()),
            rows,
        }))
    }

    #[test]
    fn a_table_that_owns_its_names_reads_as_its_core_table() {
        let accounts = Catalog::core().find("accounts").unwrap().clone();
        let TableRef::Static(core) = &accounts else { panic!("a core table") };
        let copy = owned_copy(core, "my_accounts", None);
        let catalog = Catalog::core_and([copy.clone()]);
        assert_eq!(catalog.find("my_accounts"), Some(&copy));
        assert_ne!(copy, accounts);
        for (copied, core) in [
            ("SELECT * FROM #my_accounts", "SELECT * FROM #accounts"),
            (
                "SELECT account, open.date FROM #my_accounts WHERE account ~ 'Food'",
                "SELECT account, open.date FROM #accounts WHERE account ~ 'Food'",
            ),
        ] {
            assert_eq!(run(&catalog, copied), run(&catalog, core), "{copied}");
        }
        // the copy's handles, attributes and wildcard read its own names
        let column = copy.column("open.date").unwrap();
        assert_eq!(
            (column.name(), column.ty(), column.index()),
            ("open.date", DataType::Date, core.column_index("open.date").unwrap())
        );
        assert_eq!(copy.wildcard().collect::<Vec<_>>(), ["account", "open", "close"]);
        assert!(copy.attributes("open").contains(&"date"));
        let unknown = Query::compile_in(&catalog, "SELECT open.nope FROM #my_accounts", &ParamTypes::default())
            .err()
            .unwrap();
        assert!(
            unknown.message.starts_with("unknown attribute 'nope' of open; its attributes are date, "),
            "{}",
            unknown.message
        );
    }

    #[test]
    fn a_catalog_reads_its_own_postings_table() {
        let extra = ColumnDef::posting("account_upper", DataType::Str, "The account, upper case.", |_, row| {
            Value::Str(row.account.to_uppercase())
        });
        let postings = owned_copy(&POSTINGS, "postings", Some(extra));
        let others = Catalog::core().tables().iter().filter(|table| !table.is_postings()).cloned();
        let catalog = Catalog::of(std::iter::once(postings.clone()).chain(others));
        assert_eq!(catalog.postings(), &postings);
        assert!(postings.is_postings());
        assert_eq!(
            run(&catalog, "SELECT account_upper, account WHERE account_upper ~ 'FOOD'"),
            [vec![Value::Str("EXPENSES:FOOD".to_owned()), Value::Str("Expenses:Food".to_owned())]]
        );
        assert_eq!(run(&catalog, "SELECT count(*)"), run(Catalog::core(), "SELECT count(*)"));
        // a bare name after FROM that is a column of the catalog's postings table is a filter, not a table
        let core = Query::compile_in(Catalog::core(), "SELECT count(*) FROM account_upper", &ParamTypes::default())
            .err()
            .unwrap();
        assert!(core.message.starts_with("unknown column or table 'account_upper'"), "{}", core.message);
        let own = Query::compile_in(&catalog, "SELECT count(*) FROM account_upper", &ParamTypes::default())
            .err()
            .unwrap();
        assert!(own.message.starts_with("FROM expects a boolean expression, got str"), "{}", own.message);
    }

    #[test]
    #[should_panic(expected = "a catalog has one postings table")]
    fn a_catalog_has_one_postings_table() {
        let copy = owned_copy(&POSTINGS, "more_postings", None);
        Catalog::core_and([copy]);
    }
}
