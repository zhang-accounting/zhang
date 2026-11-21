//! What a loaded ledger's plugins provide its readers besides directives and errors: the catalog of the ledger's query
//! tables, and the snapshot each analyzer publishes for it (RFC #698 §2.4).
//!
//! The query engine depends on the core, so the core keeps what it builds without naming its types, as it keeps the
//! query cache ([`crate::derived::Derived`]). Everything here is written once while the ledger loads, before anyone
//! else can read the [`Ledger`](crate::ledger::Ledger), and never changes afterwards: a reload makes a new ledger with
//! a new catalog and new snapshots. A snapshot therefore belongs to the content of exactly one load, and a reader
//! holding a ledger reads that ledger's snapshots only.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, OnceLock};

use crate::plugin_policy::{PluginPolicy, LEVEL_ENV};

/// The official plugins every ledger has, without a `plugin` directive and whatever its `features.plugin` option says:
/// `zhang:budget`, which interprets the budget directives and provides the budget tables, the `budgets` column of
/// `postings` and the budget pages' named queries. Its native provider serves them until RFC #698 moves budgets into
/// its bundled script; the operator's `ZHANG_PLUGINS=off` stops it as it stops every plugin ([`stopped_by`]).
pub const DEFAULT_PROVIDERS: [&str; 1] = [crate::analysis::BUDGET];

/// The official default plugins `policy` stops, each with why: every one of them when the operator turned plugins off.
pub fn stopped_by(policy: &PluginPolicy) -> Vec<Unavailable> {
    if policy.loads_plugins() {
        return vec![];
    }
    DEFAULT_PROVIDERS
        .iter()
        .map(|provider| Unavailable {
            provider: (*provider).to_owned(),
            reason: format!("plugins are off ({LEVEL_ENV}=off)"),
        })
        .collect()
}

/// An official default plugin that does not run for a ledger, and why: what it interprets is not interpreted, its tables
/// and label columns answer that they are unavailable, and the ledger says so once
/// ([`zhang_ast::error::ErrorKind::ExtensionUnavailable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    pub provider: String,
    /// `plugins are off (ZHANG_PLUGINS=off)`
    pub reason: String,
}

/// The extensions of a loaded ledger: its catalog and its analyzers' snapshots.
#[derive(Default)]
pub struct Extensions {
    /// the catalog of the ledger's query tables, the query engine's, when the ledger has tables of its own (its
    /// analyzers provide some); `None` for the tables every ledger has
    pub catalog: Option<Arc<dyn Any + Send + Sync>>,
    /// the analyzers whose tables the catalog has, in the order they analyse the ledger, each with its snapshot once
    /// it published one
    pub analyzers: Vec<AnalyzerSlot>,
    /// whether the analyzers are analysing the ledger, while it loads: its errors are not placed yet, so a query of
    /// them is refused
    pub analysing: bool,
    /// the official default plugins that do not run for the ledger, set before anything of it is interpreted
    pub unavailable: Vec<Unavailable>,
}

impl Extensions {
    /// Why the official default plugin `provider` does not run for the ledger; `None` when it runs.
    pub fn unavailable(&self, provider: &str) -> Option<&str> {
        self.unavailable.iter().find(|it| it.provider == provider).map(|it| it.reason.as_str())
    }

    /// The slot of the analyzer providing the tables of `provider`.
    pub fn analyzer(&self, provider: &str) -> Option<&AnalyzerSlot> {
        self.analyzers.iter().find(|slot| slot.provider == provider)
    }

    /// The analyzer analysing the ledger now: the first one without a snapshot, as they publish in order.
    pub fn analysing_now(&self) -> Option<&str> {
        self.analyzers
            .iter()
            .find(|slot| slot.snapshot.get().is_none())
            .map(|slot| slot.provider.as_str())
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions")
            .field("catalog", &self.catalog.is_some())
            .field("analyzers", &self.analyzers)
            .field("analysing", &self.analysing)
            .field("unavailable", &self.unavailable)
            .finish()
    }
}

/// An analyzer of a ledger, and what it published for it.
#[derive(Debug)]
pub struct AnalyzerSlot {
    /// the provider of its tables, the `provided_by` of the schema
    pub provider: String,
    /// what it published, set once while the ledger loads
    pub snapshot: OnceLock<Snapshot>,
}

impl AnalyzerSlot {
    /// The slot of the analyzer providing the tables of `provider`, before it published.
    pub fn new(provider: impl Into<String>) -> AnalyzerSlot {
        AnalyzerSlot {
            provider: provider.into(),
            snapshot: OnceLock::new(),
        }
    }
}

/// What one analyzer published for one loaded ledger: all of its data, or none of it.
pub struct Snapshot {
    pub status: SnapshotStatus,
    /// the data, the query engine's: `Some` exactly when the status is [`SnapshotStatus::Ok`]
    pub data: Option<Arc<dyn Any + Send + Sync>>,
}

impl Snapshot {
    /// The snapshot of an analysis that published `data`.
    pub fn ok(data: Arc<dyn Any + Send + Sync>) -> Snapshot {
        Snapshot {
            status: SnapshotStatus::Ok,
            data: Some(data),
        }
    }

    /// The snapshot of an analysis that failed for `reason`: no data.
    pub fn failed(reason: impl Into<String>) -> Snapshot {
        Snapshot {
            status: SnapshotStatus::Failed { reason: reason.into() },
            data: None,
        }
    }

    /// The snapshot of an analyzer the ledger gives nothing to read: its tables are empty.
    pub fn skipped() -> Snapshot {
        Snapshot {
            status: SnapshotStatus::Skipped,
            data: None,
        }
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot").field("status", &self.status).finish_non_exhaustive()
    }
}

/// How an analysis ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotStatus {
    /// it published its data
    Ok,
    /// it failed, and published nothing; the ledger has an error saying why. A failed snapshot is never reused: the
    /// next load analyses again
    Failed { reason: String },
    /// the ledger has nothing it reads, so it did not run; its tables are empty
    Skipped,
}
