use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::rpc::protocol::{AgentState, StatusResult};

/// Capacity of the broadcast channel. Slow consumers lose messages — waybar
/// re-renders from the next tick so a dropped notification is recoverable.
const BROADCAST_CAPACITY: usize = 32;

/// Shared broadcaster that holds the current status snapshot and fans out
/// state changes to every active subscriber. `Arc<StatusBroadcast>` is
/// threaded through `RpcState` so both the server loop (subscriber
/// registration) and the daemon toggle handler (visibility flip) can reach
/// it.
///
/// Named `StatusBroadcast` rather than `StatusHub` — the project bans
/// `Hub`/`Manager`/`Store`/`Service` suffixes. The name describes the thing
/// (a broadcast) rather than applying a generic container suffix.
#[derive(Debug, Clone)]
pub struct StatusBroadcast {
    sender: broadcast::Sender<StatusResult>,
    snapshot: Arc<Mutex<StatusResult>>,
}

impl StatusBroadcast {
    /// Create a broadcaster with `visible = true` (window is shown at daemon boot).
    pub fn new(visible: bool) -> Self {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        let snapshot = Arc::new(Mutex::new(StatusResult {
            state: AgentState::Idle,
            visible,
            active_session: None,
        }));
        Self { sender, snapshot }
    }

    /// Return the current snapshot.
    pub fn get(&self) -> StatusResult {
        self.snapshot
            .lock()
            .expect("StatusBroadcast snapshot lock poisoned")
            .clone()
    }

    /// Update the snapshot and broadcast to all subscribers.
    /// Returns `true` if at least one subscriber received the notification.
    /// Used in tests today; K-239's ACP bridge will drive this in production.
    #[allow(dead_code)]
    pub fn set(&self, next: StatusResult) -> bool {
        let mut guard = self.snapshot.lock().expect("StatusBroadcast snapshot lock poisoned");
        *guard = next.clone();
        let delivered = matches!(self.sender.send(next), Ok(n) if n > 0);
        drop(guard);
        delivered
    }

    /// Update only the `visible` field, keeping other state intact.
    pub fn set_visible(&self, visible: bool) {
        let next = {
            let mut guard = self.snapshot.lock().expect("StatusBroadcast snapshot lock poisoned");
            guard.visible = visible;
            guard.clone()
        };
        if let Err(e) = self.sender.send(next) {
            // `SendError` here means "no active subscribers" — the common case
            // when no waybar (or other client) is attached. Trace-level, not
            // a warning.
            tracing::trace!(err = %e, "StatusBroadcast: no subscribers for visibility change");
        }
    }

    /// Subscribe to state changes. Returns the current snapshot (to send
    /// as the initial response) and a `Receiver` that will yield all future
    /// state changes.
    ///
    /// Registration and snapshot read happen under the same mutex to avoid a
    /// TOCTOU race: a `set()` between `subscribe()` and `get()` would
    /// otherwise deliver the same update twice (once in the initial snapshot
    /// the caller prints, once via the receiver).
    pub fn subscribe(&self) -> (StatusResult, broadcast::Receiver<StatusResult>) {
        let guard = self.snapshot.lock().expect("StatusBroadcast snapshot lock poisoned");
        let rx = self.sender.subscribe();
        let snapshot = guard.clone();
        drop(guard);
        (snapshot, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_receives_initial_snapshot() {
        let broadcast = StatusBroadcast::new(true);
        let (snap, _rx) = broadcast.subscribe();
        assert_eq!(snap.state, AgentState::Idle);
        assert!(snap.visible);
        assert_eq!(snap.active_session, None);
    }

    #[tokio::test]
    async fn subscriber_receives_state_change() {
        let broadcast = StatusBroadcast::new(true);
        let (_initial, mut rx) = broadcast.subscribe();

        let next = StatusResult {
            state: AgentState::Streaming,
            visible: true,
            active_session: Some("sess-1".into()),
        };
        assert!(broadcast.set(next.clone()));

        let received = rx.recv().await.expect("should receive notification");
        assert_eq!(received, next);
    }

    #[tokio::test]
    async fn set_visible_flips_visibility_field() {
        let broadcast = StatusBroadcast::new(true);
        let (_initial, mut rx) = broadcast.subscribe();

        broadcast.set_visible(false);

        let received = rx.recv().await.expect("notification");
        assert!(!received.visible);
        assert_eq!(received.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn drop_subscriber_does_not_leak_senders() {
        let broadcast = StatusBroadcast::new(true);
        {
            let (_snap, _rx) = broadcast.subscribe();
            // _rx is dropped here
        }
        // After the only receiver is dropped, send() should not error with
        // capacity issues — it returns Err(SendError) with no receivers.
        let next = StatusResult {
            state: AgentState::Streaming,
            visible: true,
            active_session: None,
        };
        // No receivers → set() returns false (no active subscribers), not a panic.
        assert!(!broadcast.set(next));
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_notification() {
        let broadcast = StatusBroadcast::new(true);
        let (_s1, mut rx1) = broadcast.subscribe();
        let (_s2, mut rx2) = broadcast.subscribe();

        let next = StatusResult {
            state: AgentState::Awaiting,
            visible: false,
            active_session: None,
        };
        broadcast.set(next.clone());

        assert_eq!(rx1.recv().await.unwrap(), next);
        assert_eq!(rx2.recv().await.unwrap(), next);
    }

    #[test]
    fn get_returns_current_snapshot_after_set() {
        let broadcast = StatusBroadcast::new(true);
        assert!(broadcast.get().visible);

        broadcast.set_visible(false);
        assert!(!broadcast.get().visible);
    }

    /// Regression for the TOCTOU race between `sender.subscribe()` and
    /// reading the snapshot: if a `set()` landed between those two steps,
    /// the subscriber would receive the same update twice (once in the
    /// initial snapshot its caller prints, once via its receiver).
    ///
    /// A `Barrier` forces an interleaving: a `set()` task is released the
    /// moment the subscribe task enters the subscribe call. The fix holds
    /// the snapshot lock across both `sender.subscribe()` and the snapshot
    /// clone, so the set either wins the lock (subscriber sees the new
    /// snapshot, receiver sees nothing) or loses it (subscriber sees the
    /// old snapshot, receiver sees the new one). Never both.
    #[tokio::test]
    async fn subscribe_and_set_race_does_not_double_deliver() {
        use std::sync::Barrier;
        use tokio::task;

        // Run the race many times — a single trial that happens to serialize
        // cleanly would pass even with a broken implementation.
        for _ in 0..50 {
            let broadcast = Arc::new(StatusBroadcast::new(true));
            let barrier = Arc::new(Barrier::new(2));

            let bc_sub = broadcast.clone();
            let barrier_sub = barrier.clone();
            let subscriber = task::spawn_blocking(move || {
                barrier_sub.wait();
                bc_sub.subscribe()
            });

            let bc_set = broadcast.clone();
            let barrier_set = barrier.clone();
            let next = StatusResult {
                state: AgentState::Streaming,
                visible: true,
                active_session: None,
            };
            let next_for_setter = next.clone();
            let setter = task::spawn_blocking(move || {
                barrier_set.wait();
                bc_set.set(next_for_setter)
            });

            let (snapshot, mut rx) = subscriber.await.unwrap();
            let _ = setter.await.unwrap();

            // Try to drain one notification non-blockingly.
            let received = tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv()).await;

            match received {
                Ok(Ok(sr)) => {
                    // Receiver saw the update. Snapshot must have been the
                    // *old* one — set() landed after subscribe() got the lock.
                    assert_eq!(
                        snapshot.state,
                        AgentState::Idle,
                        "if receiver got the new state, snapshot must still be old"
                    );
                    assert_eq!(sr, next);
                }
                Ok(Err(_)) | Err(_) => {
                    // Receiver saw nothing. Snapshot must have been the *new*
                    // one — set() landed before the subscriber acquired the
                    // lock, and `broadcast::subscribe()` only sees messages
                    // sent *after* subscription, so nothing to receive.
                    assert_eq!(
                        snapshot.state,
                        AgentState::Streaming,
                        "if receiver got nothing, snapshot must reflect the set()"
                    );
                }
            }
        }
    }
}
