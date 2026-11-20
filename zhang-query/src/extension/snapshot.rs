//! Snapshots: what a provider publishes for one loaded ledger, the rows of its tables and the labels of its label
//! columns.
//!
//! A snapshot lives with its ledger (in the query cache, [`crate::table::LedgerCache`]) and is looked up by the
//! provider's name when a query executes ([`snapshot`]), never through a compiled query, which keeps only the schema.
//! The snapshot of a native provider builds each table and label column from the ledger the first time a query reads
//! it, so a query reads only the parts it needs, as the native tables always did.

use std::sync::{Arc, OnceLock};

use chrono::NaiveDate;
use zhang_ast::Meta;
use zhang_core::ledger::Ledger;

use super::{Manifest, NativeProvider, NATIVE};
use crate::params::Params;
use crate::table::LedgerCache;
use crate::value::Value;

/// One provider's snapshot of a ledger.
// TODO(P1.4): a published snapshot carries the digest of the manifest it was built for, checked when it is published
// (with the publish checks of §3.3: cell types, the number of cells, partitions, periods, labels); today a native
// provider's snapshot is trusted, and a row with too few cells reads NULL for the missing ones
pub(crate) struct Snapshot {
    provider: Arc<str>,
    /// what builds the parts
    native: &'static NativeProvider,
    /// the rows of every table, by name, each built on first use
    tables: Vec<(String, OnceLock<TableData>)>,
    /// the labels of every label column, by name, each built on first use
    labels: Vec<(String, OnceLock<Vec<Label>>)>,
}

impl Snapshot {
    /// The snapshot of the native provider `native`, whose manifest is `manifest`, empty until a part is read.
    fn native(native: &'static NativeProvider, manifest: &Manifest) -> Snapshot {
        Snapshot {
            provider: Arc::from(manifest.provider.as_str()),
            native,
            tables: manifest.tables.iter().map(|table| (table.name.clone(), OnceLock::new())).collect(),
            labels: manifest.labels.iter().map(|label| (label.name.clone(), OnceLock::new())).collect(),
        }
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The rows of the table `name` of `ledger`, the ledger of this snapshot; `None` when the provider has no such
    /// table.
    pub fn table(&self, ledger: &Ledger, name: &str) -> Option<&TableData> {
        let (_, data) = self.tables.iter().find(|(table, _)| table == name)?;
        Some(data.get_or_init(|| (self.native.table)(ledger, name)))
    }

    /// The labels of the label column `name` of `ledger`, the ledger of this snapshot; `None` when the provider has no
    /// such column.
    pub fn labels(&self, ledger: &Ledger, name: &str) -> Option<&[Label]> {
        let (_, labels) = self.labels.iter().find(|(column, _)| column == name)?;
        Some(labels.get_or_init(|| (self.native.labels)(ledger, name)))
    }
}

/// The snapshots of a ledger's providers, for its query cache: today those of the native providers, empty until read.
pub(crate) fn snapshots() -> Vec<Snapshot> {
    NATIVE
        .iter()
        .zip(super::native_manifests())
        .map(|(native, manifest)| Snapshot::native(native, manifest))
        .collect()
}

/// The snapshot `provider` has of `ledger`; `None` when it has none.
pub(crate) fn snapshot<'l>(ledger: &'l Ledger, provider: &str) -> Option<&'l Snapshot> {
    LedgerCache::of(ledger).snapshots().iter().find(|snapshot| snapshot.provider() == provider)
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
