use std::ops::Deref;
use std::sync::Arc;

use axum::extract::FromRef;
use gotcha::GotchaContext;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;

use crate::broadcast::Broadcaster;
use crate::ReloadSender;

#[derive(Clone)]
pub struct SharedLedger(pub Arc<RwLock<Ledger>>);

impl Deref for SharedLedger {
    type Target = Arc<RwLock<Ledger>>;

    fn deref(&self) -> &Self::Target {
        &self.0
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
