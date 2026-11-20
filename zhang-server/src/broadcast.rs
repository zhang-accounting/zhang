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

    /// whether a client must not miss the event: it tells the client to fetch what it shows again, the ledger reloaded,
    /// or failed to
    fn must_reach(&self) -> bool {
        matches!(self, BroadcastEvent::Reload | BroadcastEvent::ReloadFailed(_))
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

    /// A client that does not read and would miss a reload event is let go: it gets what its buffer holds, then its
    /// stream ends, so it connects again and fetches anew. One that misses another event stays
    #[tokio::test]
    async fn a_client_that_would_miss_a_reload_is_let_go() {
        let broadcaster = super::Broadcaster::create();
        let mut stalled = broadcaster.new_client().await;
        let mut reading = broadcaster.new_client().await;
        let _event = reading.recv().await.expect("connected");
        // the buffer holds 10 events: `Connected` and 9 more
        for _ in 0..9 {
            broadcaster.broadcast(BroadcastEvent::NewVersionFound { version: "9.9.9".to_owned() }).await;
            let _event = reading.recv().await.expect("the event");
        }
        broadcaster.broadcast(BroadcastEvent::NewVersionFound { version: "9.9.9".to_owned() }).await;
        assert_eq!(broadcaster.client_number().await, 2, "missing another event, the client stays");
        let _event = reading.recv().await.expect("the event");

        broadcaster.broadcast(BroadcastEvent::Reload).await;
        assert_eq!(broadcaster.client_number().await, 1, "missing a reload, the client is let go");
        let _event = reading.recv().await.expect("the reading client gets the reload");
        let mut buffered = 0;
        while let Some(_event) = stalled.recv().await {
            buffered += 1;
        }
        assert_eq!(buffered, 10, "the client let go gets what its buffer holds, then its stream ends");
    }

    /// The ping leaves room for a reload event in the buffer of a client that does not read
    #[tokio::test]
    async fn a_ping_leaves_room_for_a_reload() {
        let broadcaster = super::Broadcaster::create();
        let mut stalled = broadcaster.new_client().await;
        for _ in 0..20 {
            broadcaster.remove_stale_clients().await;
        }
        broadcaster.broadcast(BroadcastEvent::Reload).await;
        assert_eq!(broadcaster.client_number().await, 1, "the reload fits");
        let mut last = None;
        while let Ok(event) = stalled.try_recv() {
            last = Some(format!("{event:?}"));
        }
        assert!(last.is_some_and(|it| it.contains("Reload")), "the reload is the last event buffered");
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

/// the room a client's buffer keeps for a reload event: a ping is sent only while the buffer has room for the ping and
/// for it
const PING_ROOM: usize = 2;

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

    /// Removes the clients that are gone from the broadcast list. A client that does not read is pinged only while
    /// its buffer has room for the ping and for the next reload event ([`PING_ROOM`]), and the ping never waits for it
    async fn remove_stale_clients(&self) {
        let mut inner = self.inner.lock().await;
        inner
            .clients
            .retain(|client| client.capacity() < PING_ROOM || !matches!(client.try_send(Event::default().comment("ping")), Err(TrySendError::Closed(_))));
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
    /// it, and every writer and reader after it. A client that would miss a reload event is let go instead: its
    /// stream ends once it read what its buffer holds ([`crate::routes::common::sse`]), so it connects again and
    /// fetches what it shows anew, rather than show a ledger it was never told changed. Disconnected clients are let
    /// go too
    pub async fn broadcast(&self, msg: BroadcastEvent) {
        let must_reach = msg.must_reach();
        let mut inner = self.inner.lock().await;
        inner.clients.retain(|client| match client.try_send(msg.to_data()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => !must_reach,
            Err(TrySendError::Closed(_)) => false,
        });
    }

    pub async fn client_number(&self) -> usize {
        self.inner.lock().await.clients.len()
    }
}
