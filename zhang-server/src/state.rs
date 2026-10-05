use std::ops::Deref;
use std::sync::Arc;

use axum::extract::FromRef;
use gotcha::{GotchaContext, Schematic};
use serde::Serialize;
use tokio::sync::{RwLock, RwLockWriteGuard};
use zhang_core::ledger::Ledger;
use zhang_core::ZhangError;

use crate::auth::SharedAuth;
use crate::broadcast::Broadcaster;
use crate::error::ServerError;
use crate::{ReloadSender, ServerResult};

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
    /// The ledger, to write its files: held exclusively until the write is done, so no other write reads or saves a
    /// file in between, and a write's places in a file cannot be made stale by another. A ledger an earlier write left
    /// [stale](Ledger::stale) is reloaded first, so the write reads the files as they are, by the reload every reload
    /// of the served ledger runs ([`ReloadSender::reload_in_place`]): files that cannot be loaded, or a reload that
    /// panics, are [`ServerError::UnloadableLedger`], kept for `/api/info` and told to the readers. A writer hands what
    /// its write came to to [`wrote`]. A write that edits no place the ledger loaded, such as the save of a whole file,
    /// needs no reload: it holds the lock itself.
    pub async fn for_writing(&self, reload_sender: &Arc<ReloadSender>) -> ServerResult<RwLockWriteGuard<'_, Ledger>> {
        let mut ledger = self.write().await;
        if ledger.stale {
            reload_sender.reload_in_place(&mut ledger).await.map_err(ServerError::UnloadableLedger)?;
        }
        Ok(ledger)
    }
}

/// `result`, what a write to the files of `ledger` came to. Whatever it is, the files may have changed, or were found
/// changed since the load: the ledger is [stale](Ledger::stale), so the next write reloads it first, and the reload
/// that serves the readers is asked for. A write refused for a file changed since the load works when tried again.
pub fn wrote<T>(ledger: &mut Ledger, reload_sender: &ReloadSender, result: ServerResult<T>) -> ServerResult<T> {
    ledger.stale = true;
    reload_sender.reload();
    result
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

#[derive(Clone)]
pub struct AppState {
    pub ledger: SharedLedger,
    pub broadcaster: SharedBroadcaster,
    pub reload_sender: SharedReloadSender,
    pub auth: SharedAuth,
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

impl FromRef<GotchaContext<AppState, ()>> for SharedAuth {
    fn from_ref(input: &GotchaContext<AppState, ()>) -> Self {
        input.state.auth.clone()
    }
}
