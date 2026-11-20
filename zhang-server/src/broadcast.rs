use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::Event;
use serde::Serialize;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::time::interval;

use crate::state::ReloadFailure;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum BroadcastEvent {
    Reload,
    /// a reload failed, with its `file` and `message`: the ledger served is the one loaded before it (#492)
    ReloadFailed(ReloadFailure),
    Connected,
    NewVersionFound {
        version: String,
    },
}

impl BroadcastEvent {
    pub fn to_data(&self) -> Event {
        Event::default().json_data(self).unwrap()
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::BroadcastEvent;
    use crate::state::ReloadFailure;

    /// A client that does not read misses what its buffer cannot hold, and the broadcast does not wait for it; a
    /// client that reads gets every event
    #[tokio::test]
    async fn a_client_that_does_not_read_does_not_hold_up_a_broadcast() {
        let broadcaster = super::Broadcaster::create();
        let _stalled = broadcaster.new_client().await;
        let mut reading = broadcaster.new_client().await;
        let broadcasts = async {
            for _ in 0..50 {
                broadcaster.broadcast(BroadcastEvent::Reload).await;
                let _event = reading.recv().await.expect("the event");
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), broadcasts)
            .await
            .expect("the broadcasts do not wait for the client that does not read");
    }

    /// the event the web UI switches on by `type`, with the failure's fields beside it
    #[test]
    fn a_failed_reload_event_carries_the_file_and_the_message() {
        let event = BroadcastEvent::ReloadFailed(ReloadFailure {
            file: Some("main.zhang".to_owned()),
            message: "cannot parse main.zhang: unexpected input at line 3, column 1".to_owned(),
        });
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({"type": "ReloadFailed", "file": "main.zhang", "message": "cannot parse main.zhang: unexpected input at line 3, column 1"})
        );
    }
}

pub struct Broadcaster {
    inner: Mutex<BroadcasterInner>,
}

#[derive(Debug, Clone, Default)]
struct BroadcasterInner {
    clients: Vec<Sender<Event>>,
}

impl Broadcaster {
    /// Constructs new broadcaster and spawns ping loop.
    pub fn create() -> Arc<Self> {
        let this = Arc::new(Broadcaster {
            inner: Mutex::new(BroadcasterInner::default()),
        });

        Broadcaster::spawn_ping(Arc::clone(&this));

        this
    }

    /// Pings clients every 10 seconds to see if they are alive and remove them from the broadcast
    /// list if not.
    fn spawn_ping(this: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(10));

            loop {
                interval.tick().await;
                this.remove_stale_clients().await;
            }
        });
    }

    /// Removes the clients that are gone from the broadcast list. A client that does not read, whose buffer is full,
    /// stays: the ping never waits for one
    async fn remove_stale_clients(&self) {
        let clients = self.inner.lock().await.clients.clone();

        let mut ok_clients = Vec::new();

        for client in clients {
            if !matches!(client.try_send(Event::default().comment("ping")), Err(TrySendError::Closed(_))) {
                ok_clients.push(client.clone());
            }
        }

        self.inner.lock().await.clients = ok_clients;
    }

    /// Registers client with broadcaster, returning an SSE response body.
    pub async fn new_client(&self) -> Receiver<Event> {
        let (tx, rx) = tokio::sync::mpsc::channel(10);

        tx.send(BroadcastEvent::Connected.to_data()).await.unwrap();

        self.inner.lock().await.clients.push(tx);

        rx
    }

    /// Broadcasts `msg` to all clients, without waiting for any: a client that does not read (a suspended phone, a
    /// connection whose window is shut), whose buffer is full, misses it, rather than hold up the reload that tells
    /// it, and every writer and reader after it. Disconnected clients get swept up by `remove_stale_clients`
    pub async fn broadcast(&self, msg: BroadcastEvent) {
        let clients = self.inner.lock().await.clients.clone();
        for client in &clients {
            client.try_send(msg.to_data()).ok();
        }
    }

    pub async fn client_number(&self) -> usize {
        self.inner.lock().await.clients.len()
    }
}
