//! Daemon-side per-instance "needs attention" tracker.
//!
//! Layered ON TOP of the existing toast queue: toasts are transient,
//! easily missed (especially on remote / mobile); notifications are
//! persistent until the captain engages with the instance.
//!
//! Wire shape: a per-instance entry carrying a *set* of reasons.
//! Multiple raises against the same id dedup automatically — the
//! captain sees one row per instance regardless of how many of
//! `TurnEnded` / `PermissionRequested` / `InstanceError` stacked
//! up.
//!
//! Lifecycle:
//! - **Raise** on non-focused instances when one of the three events
//!   fires.
//! - **Clear** the whole entry on focus / permission resolved / prompt
//!   sent / clean shutdown.
//!
//! Driven by a single background task subscribed to the registry
//! broadcast. The task tracks `focused_id` locally (off
//! `InstancesFocused`) so the raise check uses listener-local state,
//! not a cross-Arc read that could TOCTOU with focus transitions.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::instance::{InstanceEvent, InstanceState};

/// Why an instance is asking for attention. A single entry may carry
/// multiple reasons (e.g. permission requested, then turn ended) —
/// resolution clears the whole entry, not individual reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationReason {
    /// Agent finished a turn (success or error). The captain may want
    /// to read the response or react to the failure.
    TurnEnded,
    /// Agent requested a permission. The captain needs to pick an
    /// option before the agent makes progress.
    PermissionRequested,
    /// Instance entered the `Error` lifecycle state — actor died,
    /// handshake failed, etc. Clean `Ended` does NOT raise (the
    /// captain typically initiated the shutdown).
    InstanceError,
}

/// One pending entry — keyed by instance id in the parent registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEntry {
    pub instance_id: String,
    /// Set of reasons. `BTreeSet` so the wire shape is deterministic
    /// (frontends rendering chip rows get a stable order without
    /// having to sort client-side).
    pub reasons: BTreeSet<NotificationReason>,
    /// Epoch ms when the entry was first raised. Sticky — subsequent
    /// raises against an existing entry don't update this so the
    /// captain's "since N seconds ago" chip stays anchored at the
    /// original raise.
    pub since: u64,
}

/// Daemon singleton — write-through state read by the snapshot RPC /
/// Tauri command / boot snapshot, and broadcast as
/// `InstanceEvent::NotificationsChanged` on every state transition.
#[derive(Debug)]
pub struct Notifications {
    inner: RwLock<HashMap<String, NotificationEntry>>,
    events_tx: broadcast::Sender<InstanceEvent>,
}

impl Notifications {
    /// Bind to the registry's broadcast sender so state mutations
    /// publish onto the same stream every other event rides.
    #[must_use]
    pub fn new(events_tx: broadcast::Sender<InstanceEvent>) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            events_tx,
        }
    }

    /// Snapshot — cheap clone of the current entry list, sorted by
    /// `since` ascending (oldest pending row first; mirrors the
    /// captain's intent that the longest-waiting instance bubbles to
    /// the top of the palette).
    pub fn list_snapshot(&self) -> Vec<NotificationEntry> {
        let g = self.inner.read().expect("notifications lock poisoned");
        let mut items: Vec<NotificationEntry> = g.values().cloned().collect();
        items.sort_by_key(|e| e.since);
        items
    }

    /// Lookup the entry for a single instance. `None` when the id has
    /// nothing pending. Powers the per-instance query path that
    /// external plugins (nvim, ctl scripting) drive their own
    /// per-instance notification surface off — `notifications/list`
    /// returns the full set; this returns just one row without the
    /// caller having to filter client-side.
    pub fn get(&self, instance_id: &str) -> Option<NotificationEntry> {
        self.inner
            .read()
            .expect("notifications lock poisoned")
            .get(instance_id)
            .cloned()
    }

    /// Raise — add `reason` to the instance's entry (creating it when
    /// absent). Idempotent on a re-raise of the same reason.
    pub fn raise(&self, instance_id: &str, reason: NotificationReason) {
        let changed = {
            let mut g = self.inner.write().expect("notifications lock poisoned");
            match g.get_mut(instance_id) {
                Some(entry) => entry.reasons.insert(reason),
                None => {
                    let mut reasons = BTreeSet::new();
                    reasons.insert(reason);
                    g.insert(
                        instance_id.to_string(),
                        NotificationEntry {
                            instance_id: instance_id.to_string(),
                            reasons,
                            since: now_ms(),
                        },
                    );
                    true
                }
            }
        };
        if changed {
            self.publish_snapshot();
        }
    }

    /// Clear — drop the whole entry for `instance_id`. Idempotent
    /// (missing entry → no-op, no event).
    pub fn clear(&self, instance_id: &str) {
        let removed = {
            let mut g = self.inner.write().expect("notifications lock poisoned");
            g.remove(instance_id).is_some()
        };
        if removed {
            self.publish_snapshot();
        }
    }

    /// Clear every entry. Idempotent — empty registry → no-op, no
    /// event. Powers the captain's "dismiss all" action in the
    /// header pill / palette without forcing N round-trips through
    /// per-instance `clear`.
    pub fn clear_all(&self) {
        let removed = {
            let mut g = self.inner.write().expect("notifications lock poisoned");
            let any = !g.is_empty();
            g.clear();
            any
        };
        if removed {
            self.publish_snapshot();
        }
    }

    fn publish_snapshot(&self) {
        let items = self.list_snapshot();
        // `send` only errors when there are no live receivers — the
        // Tauri bridge + WS bridge + events/subscribe consumers all
        // share one. A transient zero-receiver state is fine; future
        // subscribers see the next mutation's snapshot.
        let _ = self.events_tx.send(InstanceEvent::NotificationsChanged { items });
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Background task: subscribe to the registry's broadcast, apply the
/// raise / clear policy from the plan. The task tracks `focused_id`
/// locally off `InstancesFocused` events — the listener is single-
/// threaded so the local state stays in lockstep with the event
/// ordering it sees, and a raise check never races against a focus
/// shift it hasn't observed yet.
///
/// Spawned from `AcpAdapter::spawn_tauri_event_bridge` alongside the
/// busy-tracker — same lifetime, same broadcast.
pub fn spawn_listener(
    notifications: Arc<Notifications>,
    mut rx: broadcast::Receiver<InstanceEvent>,
    initial_focused_id: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        let mut focused_id: Option<String> = initial_focused_id;

        loop {
            match rx.recv().await {
                Ok(evt) => apply(&notifications, &mut focused_id, evt),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "notifications listener: lagged");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

fn apply(notifications: &Notifications, focused_id: &mut Option<String>, evt: InstanceEvent) {
    match evt {
        // ── focus transitions ─────────────────────────────────────────
        InstanceEvent::InstancesFocused { instance_id: Some(id) } => {
            // Update local mirror BEFORE clearing — a TurnEnded
            // arriving for `id` later in the queue will then correctly
            // see "this is focused, don't raise" off our local state.
            *focused_id = Some(id.clone());
            notifications.clear(&id);
        }
        InstanceEvent::InstancesFocused { instance_id: None } => {
            *focused_id = None;
        }

        // ── raise paths ───────────────────────────────────────────────
        InstanceEvent::TurnEnded { instance_id, .. } if focused_id.as_deref() != Some(instance_id.as_str()) => {
            notifications.raise(&instance_id, NotificationReason::TurnEnded);
        }
        InstanceEvent::PermissionRequest { instance_id, .. } if focused_id.as_deref() != Some(instance_id.as_str()) => {
            notifications.raise(&instance_id, NotificationReason::PermissionRequested);
        }
        InstanceEvent::State {
            instance_id,
            state: InstanceState::Error,
            ..
        } if focused_id.as_deref() != Some(instance_id.as_str()) => {
            notifications.raise(&instance_id, NotificationReason::InstanceError);
        }

        // ── clear paths ───────────────────────────────────────────────
        InstanceEvent::PermissionResolved { instance_id, .. } => {
            notifications.clear(&instance_id);
        }
        InstanceEvent::State {
            instance_id,
            state: InstanceState::Ended,
            ..
        } => {
            // Clean teardown — no longer relevant. The error path
            // above already raised on `State::Error`; a subsequent
            // `State::Ended` for the same id is harmless (clear is
            // idempotent).
            notifications.clear(&instance_id);
        }

        // Everything else is irrelevant to the notification state
        // machine — registry membership deltas, transcript chunks,
        // mode / model / config updates, terminal output, etc.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::formatter::types::FormattedToolCall;
    use tokio::sync::broadcast;

    fn make() -> (Arc<Notifications>, broadcast::Receiver<InstanceEvent>) {
        let (tx, rx) = broadcast::channel(16);
        (Arc::new(Notifications::new(tx)), rx)
    }

    fn turn_ended(id: &str) -> InstanceEvent {
        InstanceEvent::TurnEnded {
            agent_id: "claude-code".into(),
            instance_id: id.into(),
            session_id: "s".into(),
            turn_id: "t".into(),
            stop_reason: None,
            error: None,
            ended_at: 0,
        }
    }

    fn permission_request(id: &str) -> InstanceEvent {
        InstanceEvent::PermissionRequest {
            agent_id: "claude-code".into(),
            instance_id: id.into(),
            session_id: "s".into(),
            turn_id: None,
            request_id: "r".into(),
            tool: "Bash".into(),
            kind: "execute".into(),
            args: String::new(),
            raw_input: None,
            content: Vec::new(),
            options: Vec::new(),
            default_option_id: None,
            formatted: FormattedToolCall {
                title: String::new(),
                stats: Vec::new(),
                description: None,
                diff: None,
                output: None,
                fields: Vec::new(),
            },
        }
    }

    fn state_error(id: &str) -> InstanceEvent {
        InstanceEvent::State {
            agent_id: "claude-code".into(),
            instance_id: id.into(),
            session_id: None,
            state: InstanceState::Error,
        }
    }

    fn state_ended(id: &str) -> InstanceEvent {
        InstanceEvent::State {
            agent_id: "claude-code".into(),
            instance_id: id.into(),
            session_id: None,
            state: InstanceState::Ended,
        }
    }

    #[test]
    fn raise_on_turn_ended_when_not_focused() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        let items = n.list_snapshot();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].instance_id, "B");
        assert!(items[0].reasons.contains(&NotificationReason::TurnEnded));
    }

    #[test]
    fn skip_raise_when_focused() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("A"));
        assert!(n.list_snapshot().is_empty());
    }

    #[test]
    fn dedup_multi_reason_same_instance() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        apply(&n, &mut focused, permission_request("B"));
        let items = n.list_snapshot();
        assert_eq!(items.len(), 1, "one entry per instance");
        let reasons = &items[0].reasons;
        assert_eq!(reasons.len(), 2);
        assert!(reasons.contains(&NotificationReason::TurnEnded));
        assert!(reasons.contains(&NotificationReason::PermissionRequested));
    }

    #[test]
    fn since_sticky_across_subsequent_raises() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        let first_since = n.list_snapshot()[0].since;
        std::thread::sleep(std::time::Duration::from_millis(2));
        apply(&n, &mut focused, permission_request("B"));
        let second_since = n.list_snapshot()[0].since;
        assert_eq!(first_since, second_since, "since must not update on re-raise");
    }

    #[test]
    fn focus_clears_entry() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        apply(
            &n,
            &mut focused,
            InstanceEvent::InstancesFocused {
                instance_id: Some("B".into()),
            },
        );
        assert!(n.list_snapshot().is_empty());
        assert_eq!(focused.as_deref(), Some("B"));
    }

    #[test]
    fn permission_resolved_clears_whole_entry_even_with_other_reasons() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        apply(&n, &mut focused, permission_request("B"));
        apply(
            &n,
            &mut focused,
            InstanceEvent::PermissionResolved {
                instance_id: "B".into(),
                request_id: "r".into(),
                option_id: "allow_once".into(),
            },
        );
        assert!(
            n.list_snapshot().is_empty(),
            "the whole entry clears, including the TurnEnded reason"
        );
    }

    #[test]
    fn state_error_raises_when_not_focused() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, state_error("B"));
        let items = n.list_snapshot();
        assert_eq!(items.len(), 1);
        assert!(items[0].reasons.contains(&NotificationReason::InstanceError));
    }

    #[test]
    fn state_ended_clears_existing_entry() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, state_error("B"));
        apply(&n, &mut focused, state_ended("B"));
        assert!(n.list_snapshot().is_empty());
    }

    #[test]
    fn snapshot_sorts_by_since_ascending() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        std::thread::sleep(std::time::Duration::from_millis(2));
        apply(&n, &mut focused, turn_ended("C"));
        let items = n.list_snapshot();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].instance_id, "B", "oldest first");
        assert_eq!(items[1].instance_id, "C");
    }

    #[test]
    fn focused_none_does_not_panic_and_raises_for_any_instance() {
        let (n, _rx) = make();
        let mut focused: Option<String> = None;
        apply(&n, &mut focused, turn_ended("B"));
        assert_eq!(n.list_snapshot().len(), 1);
    }

    #[test]
    fn get_returns_entry_when_pending_none_otherwise() {
        let (n, _rx) = make();
        assert!(n.get("B").is_none(), "missing instance → None");

        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        let entry = n.get("B").expect("entry present after raise");
        assert_eq!(entry.instance_id, "B");
        assert!(entry.reasons.contains(&NotificationReason::TurnEnded));

        n.clear("B");
        assert!(n.get("B").is_none(), "clear drops the entry from get()");
    }

    #[test]
    fn clear_all_drops_every_entry() {
        let (n, _rx) = make();
        let mut focused = Some("A".to_string());
        apply(&n, &mut focused, turn_ended("B"));
        apply(&n, &mut focused, turn_ended("C"));
        apply(&n, &mut focused, state_error("D"));
        assert_eq!(n.list_snapshot().len(), 3);
        n.clear_all();
        assert!(n.list_snapshot().is_empty());
    }

    #[test]
    fn clear_all_on_empty_is_noop() {
        let (n, mut rx) = make();
        n.clear_all();
        // No event should have been published — empty → empty is a
        // no-op so passive subscribers don't see chatter on every
        // dismiss button press for an already-empty list.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn raise_then_clear_publishes_two_events() {
        let (n, mut rx) = make();
        n.raise("B", NotificationReason::TurnEnded);
        n.clear("B");
        let first = rx.recv().await.expect("first event");
        let second = rx.recv().await.expect("second event");
        match first {
            InstanceEvent::NotificationsChanged { items } => assert_eq!(items.len(), 1),
            other => panic!("unexpected event: {other:?}"),
        }
        match second {
            InstanceEvent::NotificationsChanged { items } => assert!(items.is_empty()),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
