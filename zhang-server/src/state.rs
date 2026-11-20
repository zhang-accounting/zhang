use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Instant;

use axum::extract::FromRef;
use gotcha::{GotchaContext, Schematic};
use log::info;
use serde::Serialize;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use zhang_core::ledger::Ledger;
use zhang_core::ZhangError;

use crate::auth::SharedAuth;
use crate::broadcast::Broadcaster;
use crate::error::ServerError;
use crate::plugin_writes::PluginWrites;
use crate::routes::plugin_router::{PluginRouteState, PluginRoutes};
use crate::{load_again, ReloadSender, ServerResult};

/// Why the last reload failed, kept while the ledger served is the one loaded before it (#492): `/api/info` shows
/// it, the SSE `ReloadFailed` event carries it, and `POST /api/reload` answers it. A reload that succeeds clears it.
#[derive(Debug, Clone, Serialize, Schematic)]
pub struct ReloadFailure {
    /// the file the failure is in, when it is one file's, as a syntax error is; as the ledger names its files
    pub file: Option<String>,
    /// the reason, as the log has it
    pub message: String,
}

impl From<&ZhangError> for ReloadFailure {
    fn from(error: &ZhangError) -> Self {
        let file = match error {
            ZhangError::PestError { path, .. } | ZhangError::InvalidUtf8 { path, .. } => Some(path.clone()),
            ZhangError::ProcessError { span, .. } => span.filename.as_ref().map(|file| file.display().to_string()),
            ZhangError::FileError { path, .. } => Some(path.display().to_string()),
            _ => None,
        };
        ReloadFailure {
            file,
            message: error.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct SharedLedger(pub Arc<RwLock<Ledger>>);

impl Deref for SharedLedger {
    type Target = Arc<RwLock<Ledger>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SharedLedger {
    /// The ledger, to write its files ([`LedgerWriter`]): one writer at a time, until the write is done, so no other
    /// write reads or saves a file in between, and a write's places in a file cannot be made stale by another, while
    /// the readers go on reading. A ledger an earlier write left [stale](Ledger::stale) is reloaded first, so the write
    /// reads the files as they are, by the reload every reload of the served ledger runs ([`LedgerWriter::reload`]):
    /// files that cannot be loaded, or a reload that panics, are [`ServerError::UnloadableLedger`], kept for
    /// `/api/info` and told to the readers. A writer hands what its write came to to [`wrote`]. A write that edits no
    /// place the ledger loaded, such as the save of a whole file, needs no reload: it takes [`SharedLedger::writer`].
    pub async fn for_writing(&self, reload_sender: &Arc<ReloadSender>) -> ServerResult<LedgerWriter> {
        let mut ledger = self.writer(reload_sender).await;
        // a plugin's commit marks the sender, not the ledger, as it holds the read lock ([`ReloadSender::mark_written`])
        if ledger.stale || reload_sender.write_pending() {
            ledger.reload().await.map_err(ServerError::UnloadableLedger)?;
            // the readers read on while the write plans and writes
            ledger.downgrade();
        }
        Ok(ledger)
    }

    /// The ledger, to write its files as it is ([`LedgerWriter`]), without the reload of [`SharedLedger::for_writing`]
    pub async fn writer(&self, reload_sender: &Arc<ReloadSender>) -> LedgerWriter {
        LedgerWriter::new(self.0.clone(), reload_sender.clone()).await
    }
}

/// The write cycle of the served ledger `ledger`: what its writers hold exclusively, from the start of their write to
/// its end ([`LedgerWriter`]), a write of the API, the save of a file and a reload alike, and what the request of a
/// router plugin that may write holds shared while the plugin runs ([`crate::routes::plugin_router`]). There is one
/// per ledger lock, found by it, so every writer of a ledger shares it whatever reload sender it was handed (a test
/// hands one per request), as they all shared the ledger's write lock when a writer held that for its whole write
pub(crate) fn write_cycle(ledger: &Arc<RwLock<Ledger>>) -> Arc<RwLock<()>> {
    /// a ledger lock, by a weak reference, which keeps its allocation while it is listed, so no other lock takes its
    /// address, and its write cycle
    type Cycle = (Weak<RwLock<Ledger>>, Arc<RwLock<()>>);
    static CYCLES: Mutex<Vec<Cycle>> = Mutex::new(Vec::new());
    let mut cycles = CYCLES.lock().unwrap_or_else(PoisonError::into_inner);
    cycles.retain(|(served, _)| served.strong_count() > 0);
    if let Some((_, cycle)) = cycles.iter().find(|(served, _)| std::ptr::eq(served.as_ptr(), Arc::as_ptr(ledger))) {
        return cycle.clone();
    }
    let cycle = Arc::new(RwLock::new(()));
    cycles.push((Arc::downgrade(ledger), cycle.clone()));
    cycle
}

/// The served ledger, held by a writer: a write of the API ([`SharedLedger::for_writing`]), the save of a whole file
/// ([`SharedLedger::writer`]), or a reload of the reload listener. It reads as the [`Ledger`] served.
///
/// A writer writes its files, then reloads the ledger before it answers ([`wrote_and_reloaded`]), and the readers never
/// wait for that load: they read the ledger served until the one loaded replaces it, at once. The locks, always taken
/// in this order, and none of them while a later one is held:
///
/// 1. The [write cycle](write_cycle), exclusive, from the start of the writer to its end: writes and reloads run one
///    after the other, exactly as when a writer held the ledger's write lock for its whole write, so a write plans
///    against the ledger its files hold. The request of a router plugin that may write holds it shared, so a commit
///    never comes between a write and its reload.
/// 2. The ledger's lock: read while the writer plans and writes its files, so the readers read on; released while the
///    next ledger loads, with no lock held; written for the swap, and kept written from then on until the writer is
///    dropped, so what a write records with the ledger it answers from (its tombstones, the ids it answers) is seen
///    with that ledger and never before (a write that reloads first, before it plans, reads it again once swapped).
///    The writer never waits for the write lock holding the read lock.
/// 3. The reload sender's own locks (the tombstones, the fingerprint, the failure kept) and the commit lock of the
///    plugins' writes: short, and never held across an `.await`.
///
/// The ledger a reload replaces is dropped off the async workers and with no lock held, after the swap, and before the
/// next reload loads ([`ReloadSender::retire`]).
pub struct LedgerWriter {
    /// the ledger served, as the writer holds it now
    served: Held,
    ledger: Arc<RwLock<Ledger>>,
    reload_sender: Arc<ReloadSender>,
    /// the write cycle, held until the writer is dropped; released after the ledger, as the fields drop in order
    _cycle: OwnedRwLockWriteGuard<()>,
}

/// how a [`LedgerWriter`] holds the ledger served
enum Held {
    /// read, with the readers
    Read(OwnedRwLockReadGuard<Ledger>),
    /// written, from a swap on
    Written(OwnedRwLockWriteGuard<Ledger>),
    /// not at all, for a moment within a method of the writer, between the read lock and the write lock
    Released,
}

impl Deref for LedgerWriter {
    type Target = Ledger;

    fn deref(&self) -> &Ledger {
        match &self.served {
            Held::Read(ledger) => ledger,
            Held::Written(ledger) => ledger,
            Held::Released => unreachable!("a writer holds the ledger outside its own methods"),
        }
    }
}

impl LedgerWriter {
    /// `ledger` held by a writer, once the writers before it are done
    pub(crate) async fn new(ledger: Arc<RwLock<Ledger>>, reload_sender: Arc<ReloadSender>) -> LedgerWriter {
        let cycle = write_cycle(&ledger).write_owned().await;
        let served = Held::Read(ledger.clone().read_owned().await);
        LedgerWriter {
            served,
            ledger,
            reload_sender,
            _cycle: cycle,
        }
    }

    /// The reload every reload of the served ledger runs, the one the listener runs and the one a write runs alike:
    /// the ledger is loaded again with no lock held ([`load_again`]), while the readers read the ledger served, and
    /// replaces it at once, under the write lock, which the writer keeps ([`LedgerWriter`]). A load that fails, or
    /// panics, replaces nothing: the ledger served stays the previous one. What the reload came to is kept for
    /// `/api/info`, the readers are told, and the midnight reload is scheduled again for the ledger now served
    pub(crate) async fn reload(&mut self) -> Result<(), ReloadFailure> {
        info!("start reloading...");
        let start_time = Instant::now();
        // the ledger the reload before replaced is freed first: freed while this one loads, it slows the load
        self.reload_sender.retired().await;
        let outcome = match load_again(self.reloader()).await {
            Ok((reloaded, fingerprint)) => {
                let replaced = self.swap(reloaded).await;
                self.reload_sender.swapped(fingerprint);
                self.reload_sender.retire(replaced);
                info!("ledger is reloaded successfully in {:?}", start_time.elapsed());
                Ok(())
            }
            Err(failure) => Err(failure),
        };
        let reload_sender = self.reload_sender.clone();
        reload_sender.reloaded(&outcome, self).await;
        outcome
    }

    /// serve `reloaded` in place of the ledger served, which is answered, holding the write lock from now on
    async fn swap(&mut self, reloaded: Ledger) -> Ledger {
        let mut served = match std::mem::replace(&mut self.served, Held::Released) {
            Held::Written(served) => served,
            held => {
                // the read lock let go first: the write lock waits for every reader
                drop(held);
                self.ledger.clone().write_owned().await
            }
        };
        let replaced = std::mem::replace(&mut *served, reloaded);
        self.served = Held::Written(served);
        replaced
    }

    /// hold the ledger read again after a swap, so the readers read on
    pub(crate) fn downgrade(&mut self) {
        if let Held::Written(served) = std::mem::replace(&mut self.served, Held::Released) {
            self.served = Held::Read(served.downgrade());
        } else {
            unreachable!("only a written ledger is downgraded")
        }
    }

    /// mark the ledger served [stale](Ledger::stale), under the write lock for that moment only
    async fn mark_stale(&mut self) {
        match &mut self.served {
            Held::Written(served) => served.stale = true,
            Held::Read(_) | Held::Released => {
                self.served = Held::Released;
                let mut served = self.ledger.clone().write_owned().await;
                served.stale = true;
                self.served = Held::Read(served.downgrade());
            }
        }
    }
}

/// `result`, what a write to the files of the ledger `ledger` holds came to. Whatever it is, the files may have
/// changed, or were found changed since the load: the ledger is [stale](Ledger::stale), so the next write reloads it
/// first, and the reload that serves the readers is asked for. A write refused for a file changed since the load works
/// when tried again.
pub async fn wrote<T>(ledger: &mut LedgerWriter, result: ServerResult<T>) -> ServerResult<T> {
    ledger.mark_stale().await;
    ledger.reload_sender.reload();
    result
}

/// [`wrote`], for a write whose answer must hold for the next request: once the files are written, the served ledger
/// is reloaded before the writer answers, as every reload of it runs ([`LedgerWriter::reload`]), still held by the
/// writer, which holds it written from the swap on. A request sent after the answer reads the ledger with the write,
/// and the ids it gives. Next to the value is whether that reload succeeded: a reload that failed is kept for
/// `/api/info` and told to the readers, the ledger served is the one loaded before, behind the files, and is
/// [stale](Ledger::stale), so the next write reloads it first. A write that failed is [`wrote`]'s.
pub async fn wrote_and_reloaded<T>(ledger: &mut LedgerWriter, result: ServerResult<T>) -> ServerResult<(T, bool)> {
    let value = match result {
        Ok(value) => value,
        failed => return wrote(ledger, failed).await.map(|value| (value, false)),
    };
    // pending until the reload succeeds: a writer dropped before it (its client went away) leaves the next write to
    // reload first
    ledger.reload_sender.mark_pending();
    let reloaded = ledger.reload().await.is_ok();
    if !reloaded {
        ledger.mark_stale().await;
    }
    Ok((value, reloaded))
}

#[derive(Clone)]
pub struct SharedBroadcaster(pub Arc<Broadcaster>);

impl Deref for SharedBroadcaster {
    type Target = Arc<Broadcaster>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct SharedReloadSender(pub Arc<ReloadSender>);

impl Deref for SharedReloadSender {
    type Target = Arc<ReloadSender>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// what the writes of router plugins share, see [`PluginWrites`]
#[derive(Clone)]
pub struct SharedPluginWrites(pub Arc<PluginWrites>);

impl Deref for SharedPluginWrites {
    type Target = Arc<PluginWrites>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct AppState {
    pub ledger: SharedLedger,
    pub broadcaster: SharedBroadcaster,
    pub reload_sender: SharedReloadSender,
    pub auth: SharedAuth,
    /// the limits of the router plugins' routes, and the requests each plugin handles now
    pub plugin_routes: Arc<PluginRoutes>,
    /// what the writes of router plugins share: the key of their preview tokens and the lock of their commits
    pub plugin_writes: SharedPluginWrites,
}

impl FromRef<GotchaContext<AppState, ()>> for SharedLedger {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        input.state.ledger.clone()
    }
}

impl FromRef<GotchaContext<AppState, ()>> for SharedBroadcaster {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        input.state.broadcaster.clone()
    }
}
impl FromRef<GotchaContext<AppState, ()>> for SharedReloadSender {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        input.state.reload_sender.clone()
    }
}

impl FromRef<GotchaContext<AppState, ()>> for PluginRouteState {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        PluginRouteState {
            ledger: input.state.ledger.clone(),
            routes: input.state.plugin_routes.clone(),
            reload_sender: input.state.reload_sender.clone(),
            writes: input.state.plugin_writes.clone(),
        }
    }
}

impl FromRef<GotchaContext<AppState, ()>> for SharedAuth {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        input.state.auth.clone()
    }
}
