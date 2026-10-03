use std::ops::Deref;
use std::sync::Arc;

use axum::extract::FromRef;
use gotcha::GotchaContext;
use tokio::sync::{RwLock, RwLockWriteGuard};
use zhang_core::ledger::Ledger;

use crate::auth::SharedAuth;
use crate::broadcast::Broadcaster;
use crate::{ReloadSender, ServerResult};

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
    /// file in between, and a write's places in a file cannot be made stale by another. When an earlier write left it
    /// stale ([`Ledger::written`]), it is reloaded first, so the write reads what was written. A writer sets
    /// [`Ledger::written`] once it wrote, and asks for the reload that serves it.
    pub async fn for_writing(&self) -> ServerResult<RwLockWriteGuard<'_, Ledger>> {
        let mut ledger = self.write().await;
        if ledger.written {
            ledger.async_reload().await?;
        }
        Ok(ledger)
    }
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
