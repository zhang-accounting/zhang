//! The catalog: the tables a query compiles against, and the handles a compiled query keeps to
//! them.
//!
//! A compiled query holds no `&'static` reference to a table or a column but a handle: a
//! [`TableRef`] for a table, and for one column of it a handle that keeps the table and the column's
//! index. The compiler resolves names through a [`Catalog`]; the optimizer, the projector and
//! the executor only see the handles. A table is a core table, a `static` [`Table`] of
//! [`crate::table`] ([`TableRef::Static`]), or one a catalog builds at run time and shares
//! ([`TableRef::Shared`]), whose names and columns it owns: the tables of the catalog's extensions,
//! and its `postings` table when the extensions add columns to it.
//!
//! [`Catalog::core`] has the core tables only. [`Catalog::standard`] adds the tables of the native
//! providers, the budget tables of `zhang:budget`: it is the catalog of a ledger without tables of
//! its own and the one [`crate::Query::compile`] compiles against. A ledger whose analyzers provide
//! tables has a catalog of its own, built while it loads and kept with it ([`Catalog::of_ledger`]):
//! a query to run on a ledger compiles against that ledger's catalog. A handle holds a table's
//! schema, never its rows, which an execution looks up in the ledger it runs on.

use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, LazyLock};

use zhang_core::ledger::Ledger;

use crate::extension::{self, CatalogDigest, LabelColumn, NamedQuery, Provider, Skipped};
use crate::table::{self, Borrow, ColumnDef, Dataset, Get, RowRef, Rows, Table, POSTINGS};
use crate::value::{DataType, Value};

/// The tables a query can read: the default `postings` table first, then the tables `FROM #name`
/// names, in the order the schema lists them.
#[derive(Debug)]
pub struct Catalog {
    /// never empty: `postings` is the first table
    tables: Vec<TableRef>,
    /// the providers of the extension tables, in order
    providers: Vec<Provider>,
    /// the named queries of the providers, each with its provider, in order
    queries: Vec<(Arc<str>, NamedQuery)>,
    /// the digest of the extensions the tables come from
    digest: CatalogDigest,
}

impl Catalog {
    /// The core tables only.
    pub fn core() -> &'static Catalog {
        static CORE: LazyLock<Catalog> = LazyLock::new(|| Catalog::with_extensions(&[]).expect("the core catalog"));
        &CORE
    }

    /// The core tables and those of the native providers, compiled into zhang: the catalog of a ledger without tables
    /// of its own, which [`crate::Query::compile`] compiles against.
    pub fn standard() -> &'static Arc<Catalog> {
        static STANDARD: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
            // the tests build it from the same manifests, so this cannot fail in a release
            Arc::new(Catalog::with_extensions(extension::native_providers()).unwrap_or_else(|err| panic!("a native provider is invalid: {err}")))
        });
        &STANDARD
    }

    /// The catalog of the tables of `ledger`: the one a query to run on it compiles against. It is the ledger's own,
    /// built while it loaded, when its analyzers provide tables, and [`Catalog::standard`] otherwise.
    pub fn of_ledger(ledger: &Ledger) -> Arc<Catalog> {
        ledger
            .extensions
            .catalog
            .clone()
            .and_then(|catalog| catalog.downcast::<Catalog>().ok())
            .unwrap_or_else(|| Catalog::standard().clone())
    }

    /// The core tables and those of `providers`, whose manifests are checked; an error naming every provider that
    /// does not hold (see [`Catalog::build`]).
    pub(crate) fn with_extensions(providers: &[Provider]) -> Result<Catalog, String> {
        let (catalog, skipped) = Catalog::build(providers);
        match skipped.is_empty() {
            true => Ok(catalog),
            false => Err(skipped.into_iter().map(|skipped| skipped.message).collect::<Vec<_>>().join("; ")),
        }
    }

    /// The core tables and those of `providers`, whose manifests are checked: the core's tables, then the providers'
    /// in order, then `#errors`, which the schema has always listed last. `postings` has the providers' label columns
    /// after its own, and the catalog their named queries. A provider whose manifest does not hold, whose tables and
    /// columns cannot all be had (a name is taken, or a limit is reached), or one of whose named queries does not compile
    /// against the catalog with the parameters it declares, has none of them, and is among the providers skipped, with
    /// why. The named queries of a native provider are checked by the tests, against the standard catalog.
    pub(crate) fn build(providers: &[Provider]) -> (Catalog, Vec<Skipped>) {
        // the positions in `providers` of those the catalog is built from: a provider whose query does not compile
        // against the catalog is left out, and the catalog built again without it, which the queries of the others
        // then compile against
        let mut kept = (0..providers.len()).collect::<Vec<_>>();
        let mut refused = vec![];
        loop {
            let built = kept.iter().map(|at| providers[*at].clone()).collect::<Vec<_>>();
            let (catalog, skipped) = Catalog::build_once(&built);
            let failed = catalog
                .providers
                .iter()
                .filter(|provider| matches!(provider.source, extension::Source::Published))
                .filter_map(|provider| {
                    let error = provider.manifest.queries.iter().find_map(|query| query.check(&catalog).err())?;
                    Some((provider.manifest.provider.clone(), error))
                })
                .collect::<Vec<_>>();
            if failed.is_empty() {
                let mut skipped = skipped
                    .into_iter()
                    .map(|skipped| Skipped {
                        index: kept[skipped.index],
                        ..skipped
                    })
                    .chain(refused)
                    .collect::<Vec<_>>();
                skipped.sort_by_key(|skipped| skipped.index);
                return (catalog, skipped);
            }
            for (provider, error) in failed {
                let at = built.iter().position(|it| it.manifest.provider == provider).expect("a provider of the catalog");
                refused.push(Skipped {
                    index: kept[at],
                    message: format!("{}: {}", provider, error),
                    provider,
                });
            }
            kept.retain(|at| !refused.iter().any(|refused| refused.index == *at));
        }
    }

    /// [`Catalog::build`] but for the check of the providers' named queries.
    fn build_once(providers: &[Provider]) -> (Catalog, Vec<Skipped>) {
        let extensions = extension::build(providers);
        let core = table::tables();
        let (data, trailing) = core.split_at(core.len() - table::TRAILING);
        let postings = if extensions.labels.is_empty() {
            TableRef::Static(&POSTINGS)
        } else {
            let (columns, labels): (Vec<_>, Vec<_>) = extensions.labels.into_iter().unzip();
            TableRef::Shared(Arc::new(Table {
                name: POSTINGS.name.clone(),
                description: POSTINGS.description.clone(),
                columns: Cow::Owned(POSTINGS.columns.iter().cloned().chain(columns).collect()),
                wildcard: POSTINGS.wildcard.clone(),
                rows: Rows::Postings,
                provided_by: None,
                labels,
            }))
        };
        let tables = std::iter::once(postings)
            .chain(data.iter().filter(|table| !table.is_postings()).map(|table| TableRef::Static(table)))
            .chain(extensions.tables)
            .chain(trailing.iter().map(|table| TableRef::Static(table)));
        let mut catalog = Catalog::of(tables, extensions.digest);
        catalog.providers = extensions.providers;
        catalog.queries = extensions.queries;
        (catalog, extensions.skipped)
    }

    /// The catalog of `tables`, the first of which is a `postings` table (the only one).
    fn of(tables: impl IntoIterator<Item = TableRef>, digest: CatalogDigest) -> Catalog {
        let tables = tables.into_iter().collect::<Vec<_>>();
        assert!(tables.first().is_some_and(TableRef::is_postings), "a catalog starts with the postings table");
        assert!(!tables[1..].iter().any(TableRef::is_postings), "a catalog has one postings table");
        Catalog {
            tables,
            providers: vec![],
            queries: vec![],
            digest,
        }
    }

    /// The core tables and `extra` after them: a catalog with test-only tables.
    #[cfg(test)]
    pub(crate) fn core_and(extra: impl IntoIterator<Item = TableRef>) -> Catalog {
        Catalog::of(Catalog::core().tables.iter().cloned().chain(extra), Catalog::core().digest)
    }

    /// The digest of the catalog's extensions: a query that reads them runs only on ledgers whose catalog has it.
    pub fn digest(&self) -> CatalogDigest {
        self.digest
    }

    /// Every table, `postings` first.
    pub fn tables(&self) -> &[TableRef] {
        &self.tables
    }

    /// The named queries of the catalog's providers, each with its provider, by provider and then as each declares them:
    /// the server serves them with its own built-in queries.
    pub fn named_queries(&self) -> impl Iterator<Item = (&str, &NamedQuery)> {
        self.queries.iter().map(|(provider, query)| (&**provider, query))
    }

    /// The table a query without `FROM #name` reads.
    pub(crate) fn postings(&self) -> &TableRef {
        &self.tables[0]
    }

    /// The table `name` (without `#`). Table names are case-sensitive, as in beanquery.
    pub(crate) fn find(&self, name: &str) -> Option<&TableRef> {
        self.tables.iter().find(|table| table.name() == name)
    }

    /// The providers whose snapshots the catalog reads from the ledger ([`extension::Source::Published`]): its analyzers,
    /// in order.
    pub(crate) fn published_providers(&self) -> impl Iterator<Item = &str> {
        self.providers
            .iter()
            .filter(|provider| matches!(provider.source, extension::Source::Published))
            .map(|provider| provider.manifest.provider.as_str())
    }

    /// The provider `name` of extension tables of the catalog, the one whose snapshot it reads from the ledger
    /// ([`extension::Source::Published`]); `None` when the catalog has no such provider.
    pub(crate) fn published_provider(&self, name: &str) -> Option<&Provider> {
        self.providers
            .iter()
            .find(|provider| matches!(provider.source, extension::Source::Published) && provider.manifest.provider == name)
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

    /// The provider of an extension's table; `None` for a core table.
    pub fn provided_by(&self) -> Option<&str> {
        self.table().provided_by.as_deref()
    }

    /// The label columns of a `postings` table, by slot.
    pub(crate) fn labels(&self) -> &[Arc<LabelColumn>] {
        &self.table().labels
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
    pub fn value<'a>(&self, data: &Dataset<'a>, row: RowRef<'_, 'a>) -> Value {
        self.get.value(data, row)
    }
}

/// `#table.name: type`
impl fmt::Debug for ColumnRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}.{:?}", self.table, self.def())
    }
}

#[cfg(test)]
mod tests {
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
            Rows::Extension(_) => unreachable!("a core table"),
        };
        TableRef::Shared(Arc::new(Table {
            name: Cow::Owned(name.to_owned()),
            description: Cow::Owned(format!("{} (a copy)", table.description)),
            columns: Cow::Owned(table.columns.iter().cloned().chain(extra).collect()),
            wildcard: Names::Owned(table.wildcard.iter().map(str::to_owned).collect()),
            rows,
            provided_by: None,
            labels: vec![],
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
        let catalog = Catalog::of(std::iter::once(postings.clone()).chain(others), Catalog::core().digest());
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
