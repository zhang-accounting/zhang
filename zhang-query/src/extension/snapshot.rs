//! Snapshots: what a provider publishes for one loaded ledger, the rows of its tables and the labels of its label
//! columns, and how a query finds them.
//!
//! A snapshot is checked against the manifest of its provider before a query can read it ([`check_table`],
//! [`check_labels`], §3.3 of the RFC): every row has a cell per column, of the column's type or NULL, the rows of a
//! record table are in the declared order, the partitions of a series have their keys and are told apart by them,
//! their periods are strictly increasing and aligned to the unit, and every label binds each parameter of its column
//! with a value of its type. A snapshot that does not hold is never read: the query reads an error instead. It also
//! carries the digest of the provider it was built for ([`super::ManifestDigest`]), which the table reading it must
//! have.
//!
//! Where the snapshot is depends on the source the catalog decided on ([`Source`]):
//!
//! - a native provider's ([`NativeSnapshot`]) lives with its ledger in the query cache ([`LedgerCache`]) and builds
//!   each table and label column from the ledger, checked, the first time a query reads it, so a query reads only the
//!   parts it needs, as the native tables always did;
//! - an analyzer's ([`Snapshot`]) is published into the ledger while it loads ([`publish`], `Ledger::extensions`):
//!   whole, or as a failure without any data.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use chrono::{Datelike, NaiveDate};
use zhang_ast::Meta;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::ledger::Ledger;

use super::{ExtensionRows, LabelColumn, LabelDecl, Manifest, ManifestDigest, NativeProvider, PeriodUnit, Provider, RecordOrder, Source, TableDecl, TableKind};
use crate::params::{ParamRef, Params};
use crate::table::LedgerCache;
use crate::value::Value;

/// The snapshot an analyzer published: every table and label column of its manifest, checked.
pub(crate) struct Snapshot {
    /// the digest of the provider it was built for
    pub digest: ManifestDigest,
    /// the rows of every table, by name
    tables: Vec<(String, TableData)>,
    /// the labels of every label column, by name
    labels: Vec<(String, Vec<Label>)>,
}

impl Snapshot {
    /// The rows of the table `name`; `None` when the provider has no such table.
    pub fn table(&self, name: &str) -> Option<&TableData> {
        self.tables.iter().find(|(table, _)| table == name).map(|(_, data)| data)
    }

    /// The labels of the label column `name`; `None` when the provider has no such column.
    pub fn labels(&self, name: &str) -> Option<&[Label]> {
        self.labels.iter().find(|(column, _)| column == name).map(|(_, labels)| labels.as_slice())
    }
}

/// The snapshot of the provider `provider` made of `tables` and `labels`, once each is checked against its
/// declaration: every table and label column of its manifest, each once (one it has no data of is empty), and nothing
/// else. The error names what does not hold.
pub(crate) fn publish(provider: &Provider, mut tables: Vec<(String, TableData)>, mut labels: Vec<(String, Vec<Label>)>) -> Result<Snapshot, String> {
    let manifest = &provider.manifest;
    if let Some((name, _)) = tables.iter().find(|(name, _)| !manifest.tables.iter().any(|decl| decl.name == *name)) {
        return Err(format!("{} has no table named {}", manifest.provider, name));
    }
    if let Some((name, _)) = labels.iter().find(|(name, _)| !manifest.labels.iter().any(|decl| decl.name == *name)) {
        return Err(format!("{} has no label column named {}", manifest.provider, name));
    }
    let mut published = Snapshot {
        digest: provider.digest(),
        tables: Vec::with_capacity(manifest.tables.len()),
        labels: Vec::with_capacity(manifest.labels.len()),
    };
    for decl in &manifest.tables {
        let data = match tables.iter().position(|(name, _)| *name == decl.name) {
            Some(at) => tables.swap_remove(at).1,
            None => match decl.kind {
                TableKind::Records(_) => TableData::Records(vec![]),
                TableKind::Series(_) => TableData::Series(vec![]),
            },
        };
        if tables.iter().any(|(name, _)| *name == decl.name) {
            return Err(format!("table {} is published twice", decl.name));
        }
        check_table(decl, &data).map_err(|err| format!("table {}: {}", decl.name, err))?;
        published.tables.push((decl.name.clone(), data));
    }
    for decl in &manifest.labels {
        let data = match labels.iter().position(|(name, _)| *name == decl.name) {
            Some(at) => labels.swap_remove(at).1,
            None => vec![],
        };
        if labels.iter().any(|(name, _)| *name == decl.name) {
            return Err(format!("label column {} is published twice", decl.name));
        }
        check_labels(decl, &data).map_err(|err| format!("label column {}: {}", decl.name, err))?;
        published.labels.push((decl.name.clone(), data));
    }
    Ok(published)
}

#[cfg(test)]
impl Snapshot {
    /// Everything the snapshot holds, written out with every cell's `Debug` (a decimal with its scale): for tests that
    /// compare two snapshots exactly.
    pub fn describe(&self) -> String {
        use std::fmt::Write;
        let mut out = format!("digest {:?}\n", self.digest);
        let meta = |meta: &Option<Arc<Meta>>| format!("{:?}", meta.as_deref());
        for (name, data) in &self.tables {
            match data {
                TableData::Records(rows) => {
                    for row in rows {
                        writeln!(out, "{name} {:?} {}", row.cells, meta(&row.meta)).unwrap();
                    }
                }
                TableData::Series(partitions) => {
                    for partition in partitions {
                        writeln!(
                            out,
                            "{name} until {} {:?} {:?}",
                            partition.until,
                            partition.until_cause.map(|it| it.seq),
                            partition.first_cause.map(|it| it.seq)
                        )
                        .unwrap();
                        for fact in &partition.facts {
                            writeln!(out, "  {} {:?} {}", fact.period, fact.cells, meta(&fact.meta)).unwrap();
                        }
                    }
                }
            }
        }
        for (name, labels) in &self.labels {
            for label in labels {
                writeln!(out, "{name} {:?} {:?}", label.label, label.params).unwrap();
            }
        }
        out
    }
}

/// The snapshot of a native provider of a ledger: its parts are built from the ledger on first use, and checked. A
/// catalog may serve some of the provider's tables from an analyzer instead ([`super::abi`]); the parts it serves are
/// the same whichever of them it serves.
pub(crate) struct NativeSnapshot {
    native: &'static NativeProvider,
    provider: &'static Provider,
    /// the rows of every table, by name, each built on first use
    tables: Vec<(&'static TableDecl, Part<TableData>)>,
    /// the labels of every label column, by name, each built on first use
    labels: Vec<(&'static LabelDecl, Part<Vec<Label>>)>,
}

/// A part of a native snapshot, built on first use: an error when it does not hold.
type Part<T> = OnceLock<Result<T, String>>;

impl NativeSnapshot {
    /// The snapshot of the native provider `native`, whose declaration is `provider`, empty until a part is read.
    fn new(native: &'static NativeProvider, provider: &'static Provider) -> NativeSnapshot {
        let manifest: &'static Manifest = &provider.manifest;
        NativeSnapshot {
            native,
            provider,
            tables: manifest.tables.iter().map(|table| (table, OnceLock::new())).collect(),
            labels: manifest.labels.iter().map(|label| (label, OnceLock::new())).collect(),
        }
    }

    /// The rows of the table `name` of `ledger`, the ledger of this snapshot; `None` when the provider has no such
    /// table.
    fn table(&self, ledger: &Ledger, name: &str) -> Option<&Result<TableData, String>> {
        let (decl, data) = self.tables.iter().find(|(table, _)| table.name == name)?;
        Some(data.get_or_init(|| {
            let data = (self.native.table)(ledger, name);
            check_table(decl, &data).map(|()| data).map_err(|err| {
                debug_assert!(false, "{}: table {}: {}", self.provider.manifest.provider, name, err);
                err
            })
        }))
    }

    /// The labels of the label column `name` of `ledger`, the ledger of this snapshot; `None` when the provider has no
    /// such column.
    fn labels(&self, ledger: &Ledger, name: &str) -> Option<&Result<Vec<Label>, String>> {
        let (decl, labels) = self.labels.iter().find(|(column, _)| column.name == name)?;
        Some(labels.get_or_init(|| {
            let labels = (self.native.labels)(ledger, name);
            check_labels(decl, &labels).map(|()| labels).map_err(|err| {
                debug_assert!(false, "{}: label column {}: {}", self.provider.manifest.provider, name, err);
                err
            })
        }))
    }
}

/// The snapshots of a ledger's native providers, for its query cache, empty until read: one per native provider of
/// every catalog, the standard one's and those of the providers a ledger's analyzers leave tables of.
pub(crate) fn native_snapshots() -> Vec<NativeSnapshot> {
    super::native_providers()
        .iter()
        .map(|provider| match provider.source {
            Source::Native(native) => NativeSnapshot::new(native, provider),
            Source::Published => unreachable!("a native provider"),
        })
        .collect()
}

/// The rows of the table `table` of `ledger`, from the snapshot of its provider: `Ok(None)` when its provider was
/// skipped, as the ledger gives it nothing to read, so the table is empty; an error when the snapshot cannot be read,
/// saying why.
pub(crate) fn table_data<'l>(ledger: &'l Ledger, rows: &ExtensionRows, table: &str) -> Result<Option<&'l TableData>, String> {
    let unavailable = |why: &str| format!("table #{} is unavailable: {}", table, why);
    let no_data = || unavailable(&format!("{} has no data for this ledger", rows.provider));
    if let (Source::Native(_), Some(why)) = (rows.source, stopped(ledger, &rows.provider)) {
        return Err(unavailable(&why));
    }
    match rows.source {
        Source::Native(native) => match native_snapshot(ledger, native).and_then(|snapshot| snapshot.table(ledger, table)) {
            Some(Ok(data)) => Ok(Some(data)),
            Some(Err(err)) => Err(unavailable(err)),
            None => Err(no_data()),
        },
        Source::Published => match published(ledger, &rows.provider, rows.digest) {
            Published::Data(snapshot) => snapshot.table(table).map(Some).ok_or_else(no_data),
            Published::Skipped => Ok(None),
            Published::NotYet(analysing) => Err(format!("table #{} is not available while {} analyses the ledger", table, analysing)),
            Published::Unavailable(why) => Err(unavailable(&why)),
        },
    }
}

/// The labels of the label column `column` of `ledger`, from the snapshot of its provider: none when its provider was
/// skipped. A query does not read a column whose snapshot cannot be read ([`check_label_column`]); should it, the column
/// has no label.
pub(crate) fn label_data<'l>(ledger: &'l Ledger, column: &LabelColumn) -> &'l [Label] {
    match column.source {
        Source::Native(_) if stopped(ledger, &column.provider).is_some() => &[],
        Source::Native(native) => native_snapshot(ledger, native)
            .and_then(|snapshot| snapshot.labels(ledger, &column.name))
            .and_then(|labels| labels.as_deref().ok())
            .unwrap_or_default(),
        Source::Published => match published(ledger, &column.provider, column.digest) {
            Published::Data(snapshot) => snapshot.labels(&column.name).unwrap_or_default(),
            _ => &[],
        },
    }
}

/// Whether the label column `column` of `ledger` can be read, as for its provider's tables (RFC #698 §2.5): not when
/// its provider failed (`column postings.x is unavailable: plugin P failed (see the ledger errors)`), nor while it has
/// not analysed the ledger yet. A column whose provider was skipped reads empty, as its tables do.
pub(crate) fn check_label_column(ledger: &Ledger, column: &LabelColumn) -> Result<(), String> {
    match column.source {
        Source::Native(_) => match stopped(ledger, &column.provider) {
            Some(why) => Err(format!("column postings.{} is unavailable: {}", column.name, why)),
            None => Ok(()),
        },
        Source::Published => match published(ledger, &column.provider, column.digest) {
            Published::Data(_) | Published::Skipped => Ok(()),
            Published::NotYet(analysing) => Err(format!(
                "column postings.{} is not available while {} analyses the ledger",
                column.name, analysing
            )),
            Published::Unavailable(why) => Err(format!("column postings.{} is unavailable: {}", column.name, why)),
        },
    }
}

/// Why the native provider `provider` serves nothing of `ledger`: the official default plugin it is does not run for
/// the ledger, as the operator turned plugins off ([`zhang_core::extensions::Extensions::unavailable`]).
fn stopped(ledger: &Ledger, provider: &str) -> Option<String> {
    let reason = ledger.extensions.unavailable(provider)?;
    Some(format!("plugin {} does not run: {}", provider, reason))
}

/// The native snapshot of `native` of `ledger`.
fn native_snapshot<'l>(ledger: &'l Ledger, native: &NativeProvider) -> Option<&'l NativeSnapshot> {
    LedgerCache::of(ledger)
        .native_snapshots()
        .iter()
        .find(|snapshot| std::ptr::eq(snapshot.native, native))
}

/// What a provider published into a ledger, as a query reads it.
enum Published<'l> {
    Data(&'l Snapshot),
    /// it was skipped: its tables are empty
    Skipped,
    /// it has not published yet, while the ledger loads: the analyzer analysing it now
    NotYet(String),
    /// it has nothing to read, and why
    Unavailable(String),
}

/// What `provider` published into `ledger` for the provider digest `digest`.
fn published<'l>(ledger: &'l Ledger, provider: &str, digest: ManifestDigest) -> Published<'l> {
    let Some(slot) = ledger.extensions.analyzer(provider) else {
        return Published::Unavailable(format!("{} has no data for this ledger", provider));
    };
    let Some(snapshot) = slot.snapshot.get() else {
        return Published::NotYet(ledger.extensions.analysing_now().unwrap_or(provider).to_owned());
    };
    match &snapshot.status {
        SnapshotStatus::Skipped => Published::Skipped,
        SnapshotStatus::Failed { .. } => Published::Unavailable(format!("plugin {} failed (see the ledger errors)", provider)),
        SnapshotStatus::Ok => match snapshot.data.as_ref().and_then(|data| data.downcast_ref::<Snapshot>()) {
            Some(data) if data.digest == digest => Published::Data(data),
            // a snapshot for other tables than the catalog's, which its publication refuses
            Some(_) => Published::Unavailable(format!("{} published other tables", provider)),
            None => Published::Unavailable(format!("{} has no data for this ledger", provider)),
        },
    }
}

/// Check the rows `data` of the table `decl`: see the module docs.
pub(crate) fn check_table(decl: &TableDecl, data: &TableData) -> Result<(), String> {
    let columns = decl.columns.len();
    let cells = |cells: &[Value], what: &dyn Fn() -> String| -> Result<(), String> {
        if cells.len() != columns {
            return Err(format!("{} has {} cells for {} columns", what(), cells.len(), columns));
        }
        for (cell, column) in cells.iter().zip(&decl.columns) {
            if !cell.is_null() && cell.data_type() != column.ty {
                return Err(format!("{}: column {} is a {}, not a {}", what(), column.name, column.ty, cell.data_type()));
            }
        }
        Ok(())
    };
    match (&decl.kind, data) {
        (TableKind::Records(order), TableData::Records(rows)) => {
            for (at, row) in rows.iter().enumerate() {
                cells(&row.cells, &|| format!("row {}", at + 1))?;
            }
            if let RecordOrder::Columns(names) = order {
                let indexes = names
                    .iter()
                    .filter_map(|name| decl.columns.iter().position(|column| column.name == *name))
                    .collect::<Vec<_>>();
                let compare = |a: &RecordRow, b: &RecordRow| {
                    indexes
                        .iter()
                        .map(|at| a.cells[*at].sort_cmp(&b.cells[*at]))
                        .find(|order| *order != Ordering::Equal)
                        .unwrap_or(Ordering::Equal)
                };
                if let Some(at) = rows.windows(2).position(|pair| compare(&pair[0], &pair[1]) == Ordering::Greater) {
                    return Err(format!(
                        "row {} comes after row {}, but the rows are ordered by {}",
                        at + 1,
                        at + 2,
                        names.join(", ")
                    ));
                }
            }
            Ok(())
        }
        (TableKind::Series(series), TableData::Series(partitions)) => {
            let PeriodUnit::Month = series.unit;
            let aligned = |date: NaiveDate| date.day() == 1;
            let partition_columns = series
                .partition
                .iter()
                .filter_map(|name| decl.columns.iter().position(|column| column.name == *name))
                .collect::<Vec<_>>();
            let mut keys = HashSet::new();
            for (at, partition) in partitions.iter().enumerate() {
                let what = || format!("partition {}", at + 1);
                let (Some(first), Some(last)) = (partition.facts.first(), partition.facts.last()) else {
                    return Err(format!("{} has no row", what()));
                };
                for (n, fact) in partition.facts.iter().enumerate() {
                    cells(&fact.cells, &|| format!("{}, row {}", what(), n + 1))?;
                    if !aligned(fact.period) {
                        return Err(format!("{}, row {}: {} is not the first day of a month", what(), n + 1, fact.period));
                    }
                }
                if let Some(n) = partition.facts.windows(2).position(|pair| pair[0].period >= pair[1].period) {
                    return Err(format!("{}: the period of row {} is not after that of row {}", what(), n + 2, n + 1));
                }
                let key = partition_columns.iter().map(|at| first.cells[*at].clone()).collect::<Vec<_>>();
                if let Some(at) = partition_columns.iter().zip(&key).find(|(_, cell)| cell.is_null()).map(|(at, _)| *at) {
                    return Err(format!("{} has no {}", what(), decl.columns[at].name));
                }
                if partition
                    .facts
                    .iter()
                    .any(|fact| partition_columns.iter().zip(&key).any(|(at, cell)| fact.cells[*at] != *cell))
                {
                    return Err(format!("{}: the rows of a partition have its key", what()));
                }
                if !keys.insert(format!("{:?}", key)) {
                    return Err(format!("{} has the key of an earlier one", what()));
                }
                if !aligned(partition.until) || partition.until < last.period {
                    return Err(format!(
                        "{} runs until {}, which is not the first day of a month on or after its last row",
                        what(),
                        partition.until
                    ));
                }
            }
            Ok(())
        }
        _ => Err("its rows are not of its kind".to_owned()),
    }
}

/// Check the labels `labels` of the label column `decl`: every label once, each binding every parameter of the
/// column, and no other, with a value of its type or NULL.
pub(crate) fn check_labels(decl: &LabelDecl, labels: &[Label]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for label in labels {
        if !seen.insert(label.label.as_str()) {
            return Err(format!("the label {:?} is published twice", label.label));
        }
        if label.params.len() != decl.params.len() {
            return Err(format!(
                "the label {:?} binds {} parameters, not {}",
                label.label,
                label.params.len(),
                decl.params.len()
            ));
        }
        for (name, ty) in &decl.params {
            match label.params.get(&ParamRef::Named(name.clone())) {
                None => return Err(format!("the label {:?} does not bind :{}", label.label, name)),
                Some(value) if !value.is_null() && value.data_type() != *ty => {
                    return Err(format!("the label {:?} binds :{} to a {}, not a {}", label.label, name, value.data_type(), ty))
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// The rows of one table of a snapshot.
pub(crate) enum TableData {
    /// a record table's rows, in the table's order
    Records(Vec<RecordRow>),
    /// a series table's partitions, in the table's order
    Series(Vec<Partition>),
}

/// A row of a record table: a cell per column, and the metadata `meta()` reads.
pub(crate) struct RecordRow {
    pub cells: Box<[Value]>,
    pub meta: Option<Arc<Meta>>,
}

/// One partition of a series table: the periods that have something (its facts), and how far the series runs.
pub(crate) struct Partition {
    /// the facts, by period, which they are the first day of; the first one starts the series
    pub facts: Vec<FactRow>,
    /// the first day of the last period of the series as the provider sees it: the host may extend it to the current
    /// period, and the query's date bound may end it earlier
    pub until: NaiveDate,
    /// the entry whose date sets `until`
    pub until_cause: Option<Cause>,
    /// the entry that starts the series
    pub first_cause: Option<Cause>,
}

/// A fact of a series: the row of a period that has something. A generated period copies its values ([`super::Fill`]).
pub(crate) struct FactRow {
    /// the first day of the period
    pub period: NaiveDate,
    /// a cell per column; those of the period and the columns derived from it are not read
    pub cells: Box<[Value]>,
    /// the metadata `meta()` reads, which the generated periods after it keep; the facts of a partition may share it
    pub meta: Option<Arc<Meta>>,
}

/// An entry of the ledger, by its `seq` (its row of `#entries`): what an error names as the cause of a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cause {
    pub seq: u32,
}

/// A label of a label column, and the parameters its predicate binds (`:label` is the label).
pub(crate) struct Label {
    pub label: String,
    pub params: Params,
}
