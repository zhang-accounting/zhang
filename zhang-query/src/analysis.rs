//! What the host of an analysis needs of the query engine (RFC #698 §2.4, §3): the catalog of a ledger whose analyzers
//! provide tables, and the snapshots they publish into it.
//!
//! While a ledger loads, its analyzers declare their tables in manifests (`zhang_shared::plugin_abi::analysis`). The
//! host builds the ledger's catalog from them before any analysis runs ([`ledger_catalog`]) and keeps it with the
//! ledger (`Ledger::extensions`), where [`Catalog::of_ledger`] finds it: every query to run on the ledger compiles
//! against it. An analyzer whose tables the catalog cannot have is skipped, and the catalog says why, for the load to
//! report it on the plugin's directive. Each analysis then publishes its output ([`publish`]): checked against the
//! analyzer's manifest in the ledger's catalog, whole, or not at all.

use std::any::Any;
use std::sync::Arc;

pub use zhang_core::analysis::AnalyzerTables;
use zhang_core::ledger::Ledger;
use zhang_shared::plugin_abi::analysis as abi;

use crate::catalog::Catalog;
use crate::extension::{self, Provider, Source};
use crate::table::LedgerCache;

/// The catalog of a ledger, and the analyzers it has no table of.
#[derive(Debug)]
pub struct LedgerCatalog {
    pub catalog: Arc<Catalog>,
    /// the analyzers skipped, by their position among those the catalog was built for, and why, naming them
    pub skipped: Vec<(usize, String)>,
}

impl LedgerCatalog {
    /// The providers whose snapshots the catalog reads from the ledger, in order: the analyzers it has tables of.
    pub fn analyzers(&self) -> impl Iterator<Item = &str> {
        self.catalog.published_providers()
    }
}

/// The catalog of a ledger whose analyzers declare `analyzers`: the core tables, those zhang declares (the legacy names
/// of `zhang:budget`), and those of the analyzers, in order. An analyzer whose manifest does not hold, or whose tables,
/// columns and named queries cannot all be had (a name taken by the core, a declared provider or an earlier analyzer, a
/// limit reached, or a query that does not compile), has none of them and is skipped. The tables, columns and queries
/// of `zhang:budget` keep their legacy names and replace the declared ones: the catalog serves them from its snapshot.
/// Without analyzers it is [`Catalog::standard`].
pub fn ledger_catalog(analyzers: &[AnalyzerTables<'_>]) -> LedgerCatalog {
    if analyzers.is_empty() {
        return LedgerCatalog {
            catalog: Catalog::standard().clone(),
            skipped: vec![],
        };
    }
    let declared = extension::declared_providers();
    let mut skipped = vec![];
    let mut accepted: Vec<(usize, Provider, bool)> = vec![];
    for (index, analyzer) in analyzers.iter().enumerate() {
        let name_taken = declared.iter().any(|it| it.manifest.provider == analyzer.provider) && !analyzer.legacy
            || accepted.iter().any(|(_, provider, _)| provider.manifest.provider == analyzer.provider);
        if name_taken {
            skipped.push((
                index,
                format!("{}: a provider of tables is named {} already", analyzer.provider, analyzer.provider),
            ));
            continue;
        }
        match extension::abi::manifest(analyzer.provider, analyzer.manifest, analyzer.legacy) {
            Ok(manifest) => accepted.push((
                index,
                Provider {
                    manifest,
                    source: Source::Published,
                },
                analyzer.legacy,
            )),
            Err(err) => skipped.push((index, format!("{}: {}", analyzer.provider, err))),
        }
    }
    loop {
        // the declared providers, without the tables and columns a legacy analyzer serves instead
        let replaced = |name: &str| {
            accepted
                .iter()
                .any(|(_, provider, legacy)| *legacy && provider.manifest.tables.iter().any(|it| it.name == name))
        };
        let relabelled = |name: &str| {
            accepted
                .iter()
                .any(|(_, provider, legacy)| *legacy && provider.manifest.labels.iter().any(|it| it.name == name))
        };
        let queried_instead = |name: &str| {
            accepted
                .iter()
                .any(|(_, provider, legacy)| *legacy && provider.manifest.queries.iter().any(|it| it.name == name))
        };
        let mut providers = declared
            .iter()
            .map(|provider| {
                let mut provider = provider.clone();
                provider.manifest.tables.retain(|table| !replaced(&table.name));
                provider.manifest.labels.retain(|label| !relabelled(&label.name));
                provider.manifest.queries.retain(|query| !queried_instead(&query.name));
                provider
            })
            .collect::<Vec<_>>();
        providers.extend(accepted.iter().map(|(_, provider, _)| provider.clone()));
        let (catalog, refused) = Catalog::build(&providers);
        let refused = refused
            .into_iter()
            .map(|refused| {
                let at = refused.index.checked_sub(declared.len()).expect("the declared providers fit every catalog");
                (at, refused.message)
            })
            .collect::<Vec<_>>();
        // a legacy analyzer refused gives the declared provider its tables back: build again without it
        let legacy_refused = refused.iter().any(|(at, _)| accepted[*at].2);
        if !legacy_refused {
            skipped.extend(refused.iter().map(|(at, message)| (accepted[*at].0, message.clone())));
            skipped.sort_by_key(|(index, _)| *index);
            return LedgerCatalog {
                catalog: Arc::new(catalog),
                skipped,
            };
        }
        let legacy = refused.into_iter().filter(|(at, _)| accepted[*at].2).collect::<Vec<_>>();
        for (at, message) in legacy.into_iter().rev() {
            let (index, _, _) = accepted.remove(at);
            skipped.push((index, message));
        }
    }
}

/// Publish `output`, what the analyzer `provider` found in `ledger`, as its snapshot of it: checked against the
/// analyzer's manifest in the ledger's catalog ([`Catalog::of_ledger`]), and carrying the digest of that manifest,
/// which every read of the snapshot checks. The data for the ledger's slot of the analyzer
/// (`zhang_core::extensions::Snapshot::ok`), or an error saying what does not hold: then nothing is published.
pub fn publish(ledger: &Ledger, provider: &str, output: &abi::Output) -> Result<Arc<dyn Any + Send + Sync>, String> {
    let catalog = Catalog::of_ledger(ledger);
    let provider = catalog
        .published_provider(provider)
        .ok_or_else(|| format!("the catalog of the ledger has no tables of {}", provider))?;
    let entries = LedgerCache::of(ledger).entries(ledger).rows.len();
    let snapshot = extension::abi::snapshot(provider, output, entries)?;
    Ok(Arc::new(snapshot))
}

/// [`publish`] reading `output`, the output as the host reads it first, straight into cells: the same snapshot, or
/// `None` when the quick reading does not take the output as it is; [`publish`] then reads it the long way and says
/// what does not hold. An analysis's output is read at every load, so at every write.
pub fn publish_raw(ledger: &Ledger, provider: &str, output: &abi::RawOutput<'_>) -> Option<Arc<dyn Any + Send + Sync>> {
    let catalog = Catalog::of_ledger(ledger);
    let provider = catalog.published_provider(provider)?;
    let entries = LedgerCache::of(ledger).entries(ledger).rows.len();
    let snapshot = extension::reader::snapshot(provider, output, entries)?;
    Some(Arc::new(snapshot))
}

pub use crate::extension::abi::MAX_OUTPUT_VALUES;
