//! Extension tables: the tables and columns a provider adds to the catalog, which the host serves from the provider's
//! snapshot of each loaded ledger (RFC #698 §3).
//!
//! A provider declares them in a static [`Manifest`]. A catalog is built from the manifests before any snapshot
//! exists ([`build`]), and a compiled query keeps their schema only, the handles of [`crate::catalog`], never rows.
//! The rows are the provider's [`Snapshot`] of the ledger a query runs on, looked up by the provider's name when it
//! executes: a query compiled once reads the rows of every ledger it runs on, as long as the catalog of that ledger has
//! the digest the query was compiled against ([`CatalogDigest`]); on another one it fails with an error that asks to
//! compile it again, never with a wrong read.
//!
//! There are three kinds:
//!
//! - **Record tables** ([`TableKind::Records`]): the snapshot holds every row, its cells and its metadata, which
//!   `meta()` and `meta_values()` read ([`RecordRow`]); the host lists them in the order the snapshot publishes them.
//! - **Series tables** ([`TableKind::Series`]): per partition, the snapshot holds the periods that have something,
//!   its facts, and how far the series runs. The host fills the periods between and after them per query, by copying
//!   values ([`Fill`]), up to the end the provider sets, the current period when [`SeriesDecl::extend_to_today`] says
//!   so, and no later than the query's date bound ([`series`]). Every generated row counts against the query's limits.
//! - **Label columns** ([`LabelDecl`]): a set column of `postings` whose value is the labels whose predicate holds for
//!   the row. The predicate is BQL over the row with parameters, compiled once per catalog; the snapshot holds the
//!   labels and the parameters each binds ([`labels`]).
//!
//! Today the only providers are native ones, compiled into zhang ([`NATIVE`]): `zhang:budget` serves `#budgets`,
//! `#budget_events`, `#budget_definitions` and the `budgets` column of `postings` with the native budget code
//! ([`budget`]). A native snapshot is built from the ledger on first use and kept with it, like the query cache.
//!
//! Names follow §3.4 of the RFC: a core table or column always keeps its name, and of two providers declaring one
//! name the first keeps it.
//!
//! **Limits.** An extension's table has at most 64 columns ([`crate::table::MAX_CELLS`]), and a catalog at most 16 label
//! columns ([`crate::table::MAX_LABELS`]), shared by all the providers of a ledger: a column is read through a getter
//! per cell position and per label slot, monomorphized, so that reading it costs what reading a core column does. A
//! manifest past either limit fails [`build`], with every other check. The plugin guide states both limits once
//! analyzers exist (P1.4).

pub(crate) mod budget;
mod compat;
pub(crate) mod labels;
pub(crate) mod series;
mod snapshot;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, LazyLock};

use zhang_core::ledger::Ledger;

pub(crate) use self::labels::{LabelColumn, Labels};
pub(crate) use self::series::{FillRead, SeriesRow, SeriesSpec};
pub(crate) use self::snapshot::{snapshot, snapshots, Cause, FactRow, Label, Partition, RecordRow, Snapshot, TableData};
use crate::catalog::TableRef;
use crate::table::{self, Borrow, ColumnDef, Get, Names, Rows, Table};
use crate::value::DataType;

/// What a provider declares: its tables and its label columns. It is static, so the catalog is known before any
/// snapshot is built.
#[derive(Debug, Clone, Hash)]
pub(crate) struct Manifest {
    /// the provider, such as `zhang:budget`: the `provided_by` of its tables and columns
    pub provider: String,
    pub tables: Vec<TableDecl>,
    pub labels: Vec<LabelDecl>,
}

/// A table of a provider.
#[derive(Debug, Clone, Hash)]
pub(crate) struct TableDecl {
    /// the name after `#`: `[a-z][a-z0-9_]*`
    pub name: String,
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
    Records,
    /// periods per partition, the missing ones generated per query
    Series(SeriesDecl),
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
    pub description: String,
    /// a BQL predicate over a row of the table, which reads `:label`, the label, and the declared parameters
    pub predicate: String,
    /// the parameters every label binds, with their types
    pub params: Vec<(String, DataType)>,
}

/// A provider compiled into zhang: its manifest, and the parts of its snapshot of a ledger, which the host builds
/// from the ledger on first use.
pub(crate) struct NativeProvider {
    pub manifest: fn() -> Manifest,
    /// the rows of its table `name` of the ledger
    pub table: fn(&Ledger, &str) -> TableData,
    /// the labels of its label column `name` of the ledger
    pub labels: fn(&Ledger, &str) -> Vec<Label>,
}

/// The native providers, in the order their tables are listed.
pub(crate) static NATIVE: &[&NativeProvider] = &[&budget::PROVIDER];

/// The manifests of the native providers, as [`NATIVE`] lists them.
pub(crate) fn native_manifests() -> &'static [Manifest] {
    static MANIFESTS: LazyLock<Vec<Manifest>> = LazyLock::new(|| NATIVE.iter().map(|provider| (provider.manifest)()).collect());
    &MANIFESTS
}

/// The digest of the extension part of a catalog: a hash of every manifest it is built from, names, types,
/// descriptions and predicates included. Two catalogs with one digest have the same tables and columns, so a query
/// compiled against one runs on a ledger of the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CatalogDigest(u64);

impl CatalogDigest {
    fn of(manifests: &[Manifest]) -> CatalogDigest {
        // SipHash with fixed keys: the same manifests give the same digest in every process of a build
        let mut hasher = DefaultHasher::new();
        manifests.hash(&mut hasher);
        CatalogDigest(hasher.finish())
    }
}

/// `016x`
impl fmt::Display for CatalogDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// How the rows of an extension table are found: in the snapshot of its provider, under the table's name.
// TODO(P1.4): decide at catalog build which source serves the table (a native provider's snapshot in the query cache,
// or an analyzer's in `Ledger::extensions`) and keep it here, rather than looking in both when a query runs
pub(crate) struct ExtensionRows {
    pub provider: Arc<str>,
    pub kind: ExtensionKind,
}

/// The kind of an extension table, as the host serves it.
pub(crate) enum ExtensionKind {
    Records,
    Series(SeriesSpec),
}

/// The extension part of a catalog: the tables, the columns `postings` gains, and the digest.
pub(crate) struct Extensions {
    pub tables: Vec<TableRef>,
    /// the label columns, with what they read
    pub labels: Vec<(ColumnDef, Arc<LabelColumn>)>,
    pub digest: CatalogDigest,
}

/// Check `manifests` (§3.3 of the RFC) and build their tables and label columns. A manifest that does not hold is an
/// error naming the provider and what is wrong.
pub(crate) fn build(manifests: &[Manifest]) -> Result<Extensions, String> {
    let mut names = table::tables().iter().map(|table| table.name.to_string()).collect::<HashSet<_>>();
    let mut label_names = table::POSTINGS.columns.iter().map(|column| column.name.to_lowercase()).collect::<HashSet<_>>();
    let mut tables = vec![];
    let mut labels = vec![];
    for manifest in manifests {
        let provider: Arc<str> = Arc::from(manifest.provider.as_str());
        let error = |message: String| format!("{}: {}", manifest.provider, message);
        for decl in &manifest.tables {
            check_name(&decl.name).map_err(|it| error(format!("table '{}': {}", decl.name, it)))?;
            // TODO(P1.4): a ledger analyzer whose name collides is skipped and reported on its directive (§3.4), the
            // other providers keep their tables; failing the whole catalog only suits the native providers
            if !names.insert(decl.name.clone()) {
                return Err(error(format!("table '{}' is already a table", decl.name)));
            }
            let table = extension_table(&provider, decl).map_err(|it| error(format!("table #{}: {}", decl.name, it)))?;
            tables.push(TableRef::Shared(Arc::new(table)));
        }
        for decl in &manifest.labels {
            let what = format!("column {}.{}", decl.table, decl.name);
            if decl.table != "postings" {
                return Err(error(format!("{}: label columns are columns of postings", what)));
            }
            check_name(&decl.name).map_err(|it| error(format!("{}: {}", what, it)))?;
            // TODO(P1.4): as for a table, a colliding analyzer is skipped and reported instead
            if !label_names.insert(decl.name.to_lowercase()) {
                return Err(error(format!("{}: postings already has a column '{}'", what, decl.name)));
            }
            let column = LabelColumn::compile(&provider, decl).map_err(|it| error(format!("{}: {}", what, it)))?;
            // TODO(P1.4): a ledger analyzer past the limit, shared by all providers, is skipped and reported, as on a
            // name collision, instead of failing the catalog
            let (getter, holds) =
                table::label_getter(labels.len()).ok_or_else(|| error(format!("{}: a catalog has at most {} label columns", what, table::MAX_LABELS)))?;
            let def = ColumnDef {
                name: Cow::Owned(decl.name.clone()),
                ty: DataType::Set,
                description: Cow::Owned(decl.description.clone()),
                get: Get::Posting(getter),
                borrow: Borrow::Contains(holds),
                provided_by: Some(provider.clone()),
            };
            labels.push((def, Arc::new(column)));
        }
    }
    Ok(Extensions {
        tables,
        labels,
        digest: CatalogDigest::of(manifests),
    })
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

/// The table `decl` of `provider`, its declaration checked.
fn extension_table(provider: &Arc<str>, decl: &TableDecl) -> Result<Table, String> {
    let index = |name: &str| decl.columns.iter().position(|column| column.name == name);
    if decl.columns.is_empty() {
        return Err("a table has columns".to_owned());
    }
    // TODO(P1.4): as for the label columns, a ledger analyzer past the limit is skipped and reported
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
        TableKind::Records => {
            if let Some(column) = decl.columns.iter().find(|column| column.fill.is_some()) {
                return Err(format!("column '{}' has a fill, which only the columns of a series have", column.name));
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
