use std::sync::Arc;

use axum::extract::FromRef;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;

use crate::broadcast::Broadcaster;
use crate::ReloadSender;

#[derive(Clone)]
pub struct AppState {
    pub ledger: Arc<RwLock<Ledger>>,
    pub broadcaster: Arc<Broadcaster>,
    pub reload_sender: Arc<ReloadSender>,
}

impl FromRef<AppState> for Arc<RwLock<Ledger>> {
    fn from_ref(input: &AppState) -> Self {
        input.ledger.clone()
    }
}

impl FromRef<AppState> for Arc<Broadcaster> {
    fn from_ref(input: &AppState) -> Self {
        input.broadcaster.clone()
    }
}
impl FromRef<AppState> for Arc<ReloadSender> {
    fn from_ref(input: &AppState) -> Self {
        input.reload_sender.clone()
    }
}
