//! Extension tables: the tables and columns a provider adds to the catalog, which the host serves from the provider's
//! snapshot of each loaded ledger (RFC #698 §3).
//!
//! A provider declares them in a static [`Manifest`]. A catalog is built from the manifests before any snapshot
//! exists ([`build`]), and a compiled query keeps their schema only, the handles of [`crate::catalog`], never rows.
//! Where the rows of a provider come from is decided when the catalog is built, and kept with each of its tables and
//! columns ([`Source`]): an analyzer's are in the snapshot it published into the ledger while the ledger loaded, and a
//! provider zhang only declares has none. A query compiled once reads the rows of every ledger it runs on, as long as
//! the catalog of that ledger has the digest the query was compiled against ([`CatalogDigest`]); on another one it
//! fails with an error that asks to compile it again, never with a wrong read.
//!
//! There are three kinds:
//!
//! - **Record tables** ([`TableKind::Records`]): the snapshot holds every row, its cells and its metadata, which
//!   `meta()` and `meta_values()` read ([`RecordRow`]); the host lists them in the order the snapshot publishes them,
//!   which the manifest declares ([`RecordOrder`]).
//! - **Series tables** ([`TableKind::Series`]): per partition, the snapshot holds the periods that have something,
//!   its facts, and how far the series runs. The host fills the periods between and after them per query, by copying
//!   values ([`Fill`]), up to the end the provider sets, the current period when [`SeriesDecl::extend_to_today`] says
//!   so, and no later than the query's date bound ([`series`]). Every generated row counts against the query's limits.
//! - **Label columns** ([`LabelDecl`]): a set column of `postings` whose value is the labels whose predicate holds for
//!   the row. The predicate is BQL over the row with parameters, compiled once per catalog; the snapshot holds the
//!   labels and the parameters each binds ([`labels`]).
//!
//! A provider may also declare **named queries** ([`NamedQuery`]): BQL over the catalog with typed parameters, which the
//! server lists and runs with its own built-in queries, `zhang:budget`'s `budgets.*` among them. They are part of the
//! catalog, checked when it is built: each compiles against it with exactly the parameters it declares.
//!
//! The providers are analyzers ([`abi`]): plugins that analyse a ledger while it loads and publish what they found as a
//! snapshot, which the host checks against their manifest before anything of it is published ([`snapshot::publish`]).
//! zhang also declares the tables of the official default plugin `zhang:budget`, its legacy names ([`DECLARED`],
//! [`compat`]), which every catalog has: they answer that they are unavailable, saying why, when the plugin does not
//! run for a ledger, and the plugin's own tables, which it publishes, take their place when it runs.
//!
//! Names follow §3.4 of the RFC: a core table or column always keeps its name, and of two providers declaring one
//! name the first keeps it. A provider that cannot have all of its tables and columns, because a name is taken or
//! a limit is reached, or whose manifest does not hold, has none of them: it is skipped, and the catalog says why
//! ([`Skipped`]), for the load to report it on the plugin's directive. The other providers keep theirs.
//!
//! **Limits.** An extension's table has at most 64 columns ([`crate::table::MAX_CELLS`]), and a catalog at most 16 label
//! columns ([`crate::table::MAX_LABELS`]), shared by all the providers of a ledger: a column is read through a getter
//! per cell position and per label slot, monomorphized, so that reading it costs what reading a core column does. A
//! provider past either limit is skipped.

pub(crate) mod abi;
mod compat;
pub(crate) mod labels;
pub(crate) mod reader;
pub(crate) mod series;
pub(crate) mod snapshot;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, LazyLock};

pub(crate) use self::labels::{LabelColumn, Labels};
pub(crate) use self::series::{FillRead, SeriesRow, SeriesSpec};
pub(crate) use self::snapshot::{Label, RecordRow, TableData};
use crate::catalog::{Catalog, TableRef};
use crate::params::JSON_TYPES;
use crate::table::{self, Borrow, ColumnDef, Get, Names, Rows, Table};
use crate::value::DataType;
use crate::{ParamRef, ParamTypes, Query};

/// What a provider declares: its tables, its label columns and its named queries. It is static, so the catalog is known
/// before any snapshot is built.
#[derive(Debug, Clone, Hash)]
pub(crate) struct Manifest {
    /// the provider, such as `zhang:budget`: the `provided_by` of its tables and columns
    pub provider: String,
    pub tables: Vec<TableDecl>,
    pub labels: Vec<LabelDecl>,
    pub queries: Vec<NamedQuery>,
}

/// A named query: BQL with typed parameters, which the server serves as a built-in query (`GET /api/query/builtins`,
/// `POST /api/query/builtins/{name}`). zhang's own are the server's; a provider of a catalog declares more
/// ([`Catalog::named_queries`](crate::Catalog::named_queries)).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedQuery {
    /// unique, dotted and lower case, such as `budgets.month`
    pub name: String,
    /// one sentence on what the query returns
    pub description: String,
    /// the query, with its parameters written `:name`
    pub bql: String,
    /// every parameter the query uses, with its type, in the order it declares them
    pub params: Vec<(String, DataType)>,
}

impl NamedQuery {
    /// The declared types of the parameters, to compile the query with.
    pub fn param_types(&self) -> ParamTypes {
        self.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(name.as_str(), *ty))
    }

    /// Whether `name` is the name of a named query: dotted lower-case words, `[a-z][a-z0-9_]*` each, at least two.
    pub fn is_name(name: &str) -> bool {
        let mut words = 0;
        let valid = name.split('.').all(|word| {
            words += 1;
            check_name(word).is_ok()
        });
        valid && words >= 2
    }

    /// Check the query against `catalog`: it compiles, with its declared parameters, and uses each of them, by name.
    pub(crate) fn check(&self, catalog: &Catalog) -> Result<(), String> {
        let query = Query::compile_in(catalog, &self.bql, &self.param_types()).map_err(|err| format!("query {}: {}", self.name, err))?;
        let mut used = vec![];
        for (param, _) in query.params() {
            match param {
                ParamRef::Named(name) => used.push(name),
                ParamRef::Positional(_) => return Err(format!("query {}: it uses {}; name the parameters of a named query", self.name, param)),
            }
        }
        if let Some((unused, _)) = self.params.iter().find(|(name, _)| !used.contains(name)) {
            return Err(format!("query {}: it declares the parameter :{}, which it does not use", self.name, unused));
        }
        Ok(())
    }
}

/// A table of a provider.
#[derive(Debug, Clone, Hash)]
pub(crate) struct TableDecl {
    /// the name after `#`: `[a-z][a-z0-9_]*`
    pub name: String,
    /// the name its rows are published under: an analyzer's name for it, which the catalog's starts with the
    /// analyzer's namespace ([`abi`]); the name for a declared provider
    pub published_as: String,
    pub description: String,
    pub kind: TableKind,
    pub columns: Vec<ColumnDecl>,
    /// the columns `SELECT *` expands to, in order
    pub wildcard: Vec<String>,
}

/// How a table has its rows.
#[derive(Debug, Clone, Hash)]
pub(crate) enum TableKind {
    /// every row is in the snapshot, in the table's order
    Records(RecordOrder),
    /// periods per partition, the missing ones generated per query
    Series(SeriesDecl),
}

/// The order of the rows of a record table, which a snapshot keeps: checked when it is published.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RecordOrder {
    /// the order the provider publishes them in
    Published,
    /// sorted by these columns, ascending, as `ORDER BY` sorts
    Columns(Vec<String>),
}

/// The periods of a series table.
#[derive(Debug, Clone, Hash)]
pub(crate) struct SeriesDecl {
    /// the column of the period, a date: the first day of the period
    pub period: String,
    pub unit: PeriodUnit,
    /// the columns of a part of the period's date, such as its year
    pub derive: Vec<(String, DatePart)>,
    /// the columns that tell the partitions apart; they keep their value over a partition (fill `carry`)
    pub partition: Vec<String>,
    /// whether a series that ends before the current period runs up to it
    pub extend_to_today: bool,
}

/// The length of a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PeriodUnit {
    Month,
}

/// A part of a date, for a column derived from the period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DatePart {
    Year,
    Month,
}

/// A column of a provider's table.
#[derive(Debug, Clone, Hash)]
pub(crate) struct ColumnDecl {
    pub name: String,
    pub ty: DataType,
    pub description: String,
    /// how a generated period fills the column, in a series table; `None` in a record table and for the period and
    /// the columns derived from it
    pub fill: Option<Fill>,
}

/// How a generated period of a series fills a column: the host only copies values, it computes none.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Fill {
    /// the previous row's value
    Carry,
    /// zero, in the commodity of the previous row's value for an amount
    Zero,
    /// the previous row's value of another column of the same type, which is filled with `carry`.
    ///
    /// Only a `carry` column, so that every generated period reads the last fact's value of it: the previous row of
    /// the first generated period is the fact itself, and that of each later one a generated period, whose value of a
    /// `carry` column is the fact's again. Of a column filled with `zero` or `previous`, the previous row's value
    /// would depend on how far the period is from the fact, which a copy of the last fact cannot give. Budgets need
    /// `assigned` to be the previous `available`, a `carry` column.
    Previous(String),
}

/// A label column of a provider: a set column the core table `table` gains.
#[derive(Debug, Clone, Hash)]
pub(crate) struct LabelDecl {
    /// the table: `postings`, the only one in this version
    pub table: String,
    pub name: String,
    /// the name its labels are published under, as for a table ([`TableDecl::published_as`])
    pub published_as: String,
    pub description: String,
    /// a BQL predicate over a row of the table, which reads `:label`, the label, and the declared parameters
    pub predicate: String,
    /// the parameters every label binds, with their types
    pub params: Vec<(String, DataType)>,
}

/// The manifests of the providers zhang declares without serving their rows, in the order their tables are listed:
/// the legacy names of `zhang:budget` ([`compat::budget_manifest`]).
pub(crate) static DECLARED: &[fn() -> Manifest] = &[compat::budget_manifest];

/// Where the rows of a provider's tables and columns come from, decided when a catalog is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// a provider zhang declares ([`DECLARED`]) but does not serve: an official default plugin that does not run for
    /// the ledger, whose tables and columns answer that they are unavailable, saying why
    Declared,
    /// an analyzer: its snapshot is the one it published into the ledger while the ledger loaded
    /// ([`zhang_core::extensions::Extensions`])
    Published,
}

/// The kind of source: two catalogs whose tables are alike but served from different sources have different digests,
/// so a query compiled against one never reads the other's rows.
impl Hash for Source {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Source::Declared => 0u8.hash(state),
            Source::Published => 1u8.hash(state),
        }
    }
}

/// A provider of a catalog: its manifest and where its rows come from.
#[derive(Debug, Clone)]
pub(crate) struct Provider {
    pub manifest: Manifest,
    pub source: Source,
}

impl Provider {
    /// The digest of the provider: of its manifest and the kind of its source.
    pub fn digest(&self) -> ManifestDigest {
        let mut hasher = DefaultHasher::new();
        self.manifest.hash(&mut hasher);
        self.source.hash(&mut hasher);
        ManifestDigest(hasher.finish())
    }
}

/// The providers zhang declares, as [`DECLARED`] lists them.
pub(crate) fn declared_providers() -> &'static [Provider] {
    static PROVIDERS: LazyLock<Vec<Provider>> = LazyLock::new(|| {
        DECLARED
            .iter()
            .map(|manifest| Provider {
                manifest: manifest(),
                source: Source::Declared,
            })
            .collect()
    });
    &PROVIDERS
}

/// The digest of one provider of a catalog ([`Provider::digest`]): a snapshot carries the digest of the provider it was
/// built for, which the catalog checks when it is published and the tables when they read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ManifestDigest(u64);

/// The digest of the extension part of a catalog: a hash of every provider it is built from, their manifests (names,
/// types, descriptions and predicates included) and the kinds of their sources. Two catalogs with one digest have the
/// same tables and columns, served alike, so a query compiled against one runs on a ledger of the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CatalogDigest(u64);

impl CatalogDigest {
    fn of(digests: &[ManifestDigest]) -> CatalogDigest {
        // SipHash with fixed keys: the same providers give the same digest in every process of a build
        let mut hasher = DefaultHasher::new();
        digests.hash(&mut hasher);
        CatalogDigest(hasher.finish())
    }
}

/// `016x`
impl fmt::Display for CatalogDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// How the rows of an extension table are found: in the snapshot of its provider, under the table's name, from the
/// source the catalog decided on.
pub(crate) struct ExtensionRows {
    pub provider: Arc<str>,
    /// the digest of the provider, which its snapshot must have
    pub digest: ManifestDigest,
    pub source: Source,
    pub kind: ExtensionKind,
}

/// The kind of an extension table, as the host serves it.
pub(crate) enum ExtensionKind {
    Records,
    Series(SeriesSpec),
}

/// The extension part of a catalog: the tables, the columns `postings` gains, the named queries, the providers that have
/// them, the digest, and the providers skipped.
pub(crate) struct Extensions {
    pub tables: Vec<TableRef>,
    /// the label columns, with what they read
    pub labels: Vec<(ColumnDef, Arc<LabelColumn>)>,
    /// the named queries, each with its provider, by provider then as each declares them
    pub queries: Vec<(Arc<str>, NamedQuery)>,
    /// the providers whose tables and columns the catalog has, in order
    pub providers: Vec<Provider>,
    pub digest: CatalogDigest,
    /// the providers the catalog has none of the tables and columns of, and why
    pub skipped: Vec<Skipped>,
}

/// A provider a catalog skipped: a name it declares is taken, it would pass a limit, or its manifest does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// its position among the providers the catalog was built from
    pub index: usize,
    pub provider: String,
    /// what is wrong, naming the provider
    pub message: String,
}

/// Check the manifests of `providers` (§3.3 of the RFC) and build their tables and label columns, in order, and gather
/// their named queries. A provider whose manifest does not hold, or whose tables, columns or queries cannot all be had,
/// has none of them and is skipped, with a message naming it and what is wrong; the others keep theirs. Whether each
/// named query compiles against the catalog is checked once the catalog is built ([`Catalog::build`]).
pub(crate) fn build(providers: &[Provider]) -> Extensions {
    let mut names = table::tables().iter().map(|table| table.name.to_string()).collect::<HashSet<_>>();
    let mut label_names = table::POSTINGS.columns.iter().map(|column| column.name.to_lowercase()).collect::<HashSet<_>>();
    let mut query_names = HashSet::new();
    let mut extensions = Extensions {
        tables: vec![],
        labels: vec![],
        queries: vec![],
        providers: vec![],
        digest: CatalogDigest(0),
        skipped: vec![],
    };
    for (index, provider) in providers.iter().enumerate() {
        let checked =
            provider_tables(provider, &names, &label_names, extensions.labels.len()).and_then(|parts| provider_queries(provider, &query_names).map(|()| parts));
        match checked {
            Ok((tables, labels)) => {
                names.extend(provider.manifest.tables.iter().map(|decl| decl.name.clone()));
                label_names.extend(provider.manifest.labels.iter().map(|decl| decl.name.to_lowercase()));
                query_names.extend(provider.manifest.queries.iter().map(|query| query.name.clone()));
                extensions.tables.extend(tables);
                extensions.labels.extend(labels);
                let name: Arc<str> = Arc::from(provider.manifest.provider.as_str());
                extensions
                    .queries
                    .extend(provider.manifest.queries.iter().map(|query| (name.clone(), query.clone())));
                extensions.providers.push(provider.clone());
            }
            Err(message) => extensions.skipped.push(Skipped {
                index,
                provider: provider.manifest.provider.clone(),
                message: format!("{}: {}", provider.manifest.provider, message),
            }),
        }
    }
    extensions.digest = CatalogDigest::of(&extensions.providers.iter().map(Provider::digest).collect::<Vec<_>>());
    extensions
}

/// The tables and label columns of `provider`, checked, when the catalog has the names `names` and `label_names`
/// already and `labels` label columns.
#[allow(clippy::type_complexity)]
fn provider_tables(
    provider: &Provider, names: &HashSet<String>, label_names: &HashSet<String>, labels: usize,
) -> Result<(Vec<TableRef>, Vec<(ColumnDef, Arc<LabelColumn>)>), String> {
    let manifest = &provider.manifest;
    let name: Arc<str> = Arc::from(manifest.provider.as_str());
    let digest = provider.digest();
    let mut taken = HashSet::new();
    let mut tables = vec![];
    for decl in &manifest.tables {
        check_name(&decl.name).map_err(|it| format!("table '{}': {}", decl.name, it))?;
        if names.contains(&decl.name) || !taken.insert(decl.name.clone()) {
            return Err(format!("table '{}' is already a table", decl.name));
        }
        let table = extension_table(&name, digest, provider.source, decl).map_err(|it| format!("table #{}: {}", decl.name, it))?;
        tables.push(TableRef::Shared(Arc::new(table)));
    }
    let mut columns = vec![];
    for decl in &manifest.labels {
        let what = format!("column {}.{}", decl.table, decl.name);
        if decl.table != "postings" {
            return Err(format!("{}: label columns are columns of postings", what));
        }
        check_name(&decl.name).map_err(|it| format!("{}: {}", what, it))?;
        if label_names.contains(&decl.name.to_lowercase()) || !taken.insert(format!("postings.{}", decl.name.to_lowercase())) {
            return Err(format!("{}: postings already has a column '{}'", what, decl.name));
        }
        let column = LabelColumn::compile(&name, digest, provider.source, decl).map_err(|it| format!("{}: {}", what, it))?;
        let (getter, holds) =
            table::label_getter(labels + columns.len()).ok_or_else(|| format!("{}: a catalog has at most {} label columns", what, table::MAX_LABELS))?;
        let def = ColumnDef {
            name: Cow::Owned(decl.name.clone()),
            ty: DataType::Set,
            description: Cow::Owned(decl.description.clone()),
            get: Get::Posting(getter),
            borrow: Borrow::Contains(holds),
            provided_by: Some(name.clone()),
        };
        columns.push((def, Arc::new(column)));
    }
    Ok((tables, columns))
}

/// Check the named queries of `provider` but whether they compile, when the catalog has the named queries `taken`
/// already: their names, which no other query of the catalog has, and their parameters, which a JSON value can give.
fn provider_queries(provider: &Provider, taken: &HashSet<String>) -> Result<(), String> {
    let mut own = HashSet::new();
    for query in &provider.manifest.queries {
        let what = format!("query {}", query.name);
        if !NamedQuery::is_name(&query.name) {
            return Err(format!(
                "{}: a query's name is dotted lower-case words, each a lower-case letter, then lower-case letters, digits and '_'",
                what
            ));
        }
        if taken.contains(&query.name) || !own.insert(query.name.as_str()) {
            return Err(format!("{} is already a named query", what));
        }
        let mut params = HashSet::new();
        for (name, ty) in &query.params {
            check_name(name).map_err(|it| format!("{}: parameter :{}: {}", what, name, it))?;
            if !params.insert(name.as_str()) {
                return Err(format!("{}: it declares the parameter :{} twice", what, name));
            }
            if !JSON_TYPES.contains(ty) {
                return Err(format!("{}: parameter :{} is a {}, which a JSON value cannot give", what, name, ty));
            }
        }
    }
    Ok(())
}

/// Whether `name` is a name a provider can give a table or a column: `[a-z][a-z0-9_]*`.
fn check_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_lowercase()) && chars.all(|it| it.is_ascii_lowercase() || it.is_ascii_digit() || it == '_');
    if valid {
        Ok(())
    } else {
        Err("a name is a lower-case letter, then lower-case letters, digits and '_'".to_owned())
    }
}

/// The table `decl` of `provider`, whose digest is `digest` and whose rows come from `source`, its declaration checked.
fn extension_table(provider: &Arc<str>, digest: ManifestDigest, source: Source, decl: &TableDecl) -> Result<Table, String> {
    let index = |name: &str| decl.columns.iter().position(|column| column.name == name);
    if decl.columns.is_empty() {
        return Err("a table has columns".to_owned());
    }
    if decl.columns.len() > table::MAX_CELLS {
        return Err(format!("a table has at most {} columns", table::MAX_CELLS));
    }
    let mut seen = HashSet::new();
    for column in &decl.columns {
        check_name(&column.name).map_err(|it| format!("column '{}': {}", column.name, it))?;
        if !seen.insert(column.name.to_lowercase()) {
            return Err(format!("two columns are named '{}'", column.name));
        }
    }
    for name in &decl.wildcard {
        index(name).ok_or_else(|| format!("its wildcard names '{}', which is no column", name))?;
    }
    let kind = match &decl.kind {
        TableKind::Records(order) => {
            if let Some(column) = decl.columns.iter().find(|column| column.fill.is_some()) {
                return Err(format!("column '{}' has a fill, which only the columns of a series have", column.name));
            }
            if let RecordOrder::Columns(names) = order {
                if let Some(name) = names.iter().find(|name| index(name).is_none()) {
                    return Err(format!("its rows are ordered by '{}', which is no column", name));
                }
            }
            ExtensionKind::Records
        }
        TableKind::Series(series) => {
            let period = index(&series.period).ok_or_else(|| format!("its period is '{}', which is no column", series.period))?;
            let fills = decl
                .columns
                .iter()
                .enumerate()
                .map(|(at, column)| series_read(decl, series, at, column, period))
                .collect::<Result<Vec<_>, _>>()?;
            for name in &series.partition {
                let at = index(name).ok_or_else(|| format!("its partition names '{}', which is no column", name))?;
                if decl.columns[at].fill != Some(Fill::Carry) {
                    return Err(format!("partition column '{}' is filled with carry", name));
                }
            }
            let partition = series.partition.iter().map(|name| index(name).expect("checked above")).collect();
            let spec = SeriesSpec {
                period,
                unit: series.unit,
                partition,
                fills,
                extend_to_today: series.extend_to_today,
            };
            ExtensionKind::Series(spec)
        }
    };
    let columns = decl
        .columns
        .iter()
        .enumerate()
        .map(|(at, column)| ColumnDef {
            name: Cow::Owned(column.name.clone()),
            ty: column.ty,
            description: Cow::Owned(column.description.clone()),
            get: Get::Record(table::cell_getter(at).expect("checked above")),
            borrow: Borrow::No,
            provided_by: Some(provider.clone()),
        })
        .collect::<Vec<_>>();
    Ok(Table {
        name: Cow::Owned(decl.name.clone()),
        description: Cow::Owned(decl.description.clone()),
        columns: Cow::Owned(columns),
        wildcard: Names::Owned(decl.wildcard.clone()),
        rows: Rows::Extension(ExtensionRows {
            provider: provider.clone(),
            digest,
            source,
            kind,
        }),
        provided_by: Some(provider.clone()),
        labels: vec![],
    })
}

/// How a series reads the column `column`, at `at` among the columns, in a generated period; the period is the column
/// at `period`.
fn series_read(decl: &TableDecl, series: &SeriesDecl, at: usize, column: &ColumnDecl, period: usize) -> Result<FillRead, String> {
    let derived = series.derive.iter().find(|(name, _)| *name == column.name).map(|(_, part)| *part);
    let generated = at == period || derived.is_some();
    match (&column.fill, generated) {
        (Some(_), true) => return Err(format!("column '{}' is generated from the period and has no fill", column.name)),
        (None, false) => return Err(format!("column '{}' needs a fill: carry, zero or previous", column.name)),
        _ => {}
    }
    if at == period {
        if column.ty != DataType::Date {
            return Err(format!("the period '{}' is a date", column.name));
        }
        return Ok(FillRead::Period);
    }
    if let Some(part) = derived {
        if column.ty != DataType::Int {
            return Err(format!("column '{}', a part of the period, is an int", column.name));
        }
        return Ok(match part {
            DatePart::Year => FillRead::Year,
            DatePart::Month => FillRead::Month,
        });
    }
    match column.fill.as_ref().expect("checked above") {
        Fill::Carry => Ok(FillRead::Carry),
        Fill::Zero => match column.ty {
            DataType::Amount | DataType::Decimal | DataType::Int => Ok(FillRead::Zero),
            other => Err(format!("column '{}' is a {}, which has no zero", column.name, other)),
        },
        Fill::Previous(name) => {
            let source = decl
                .columns
                .iter()
                .position(|it| it.name == *name)
                .ok_or_else(|| format!("column '{}' is filled from '{}', which is no column", column.name, name))?;
            let other = &decl.columns[source];
            if source == at || other.ty != column.ty || other.fill != Some(Fill::Carry) {
                return Err(format!(
                    "column '{}' is filled from '{}', which must be another column of its type filled with carry",
                    column.name, name
                ));
            }
            Ok(FillRead::Previous(source as u32))
        }
    }
}

#[cfg(test)]
mod tests;
