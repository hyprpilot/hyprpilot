//! Generic instance registry shared by every `Adapter` impl.
//!
//! `AdapterRegistry<H>` owns the HashMap + insertion-order vec +
//! focused-id pointer + broadcast channel. ACP's `AcpAdapter`
//! composes it; a future `HttpAdapter` will too — the transport-side
//! facade stays a thin wrapper that parses params, delegates, and
//! maps errors. Adding a transport means a new `impl InstanceActor`
//! and a new adapter struct; the registry stays as-is.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, Mutex, RwLock};

use super::instance::{InstanceActor, InstanceEvent, InstanceEventStream, InstanceInfo, InstanceKey};
use super::{AdapterError, AdapterResult};

/// Capacity of the instance-event broadcast. Slow subscribers drop
/// notifications; the webview + ctl clients resync from the next
/// tick. See `InstanceEventStream` — every subscriber must handle
/// `RecvError::Lagged`.
const EVENT_BROADCAST_CAPACITY: usize = 256;

pub struct AdapterRegistry<H: InstanceActor> {
    instances: Mutex<HashMap<InstanceKey, Arc<H>>>,
    /// Insertion-order index parallel to `instances`. Auto-focus
    /// picks the oldest surviving key; restart preserves its slot.
    order: Mutex<Vec<InstanceKey>>,
    /// Single-valued focus pointer. Explicit `focus` writes;
    /// shutdown-of-focused + first-spawn-when-empty auto-focus; drain
    /// clears.
    focused: RwLock<Option<InstanceKey>>,
    events_tx: broadcast::Sender<InstanceEvent>,
}

impl<H: InstanceActor> Default for AdapterRegistry<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: InstanceActor> AdapterRegistry<H> {
    #[must_use]
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            instances: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            focused: RwLock::new(None),
            events_tx,
        }
    }

    /// Broadcast handle for publishers outside the registry (actor
    /// tasks emitting `State`/`Transcript`/`PermissionRequest`).
    #[must_use]
    pub fn events_tx(&self) -> broadcast::Sender<InstanceEvent> {
        self.events_tx.clone()
    }

    /// Subscribe to every lifecycle + transcript + registry event.
    /// Consumers must handle `RecvError::Lagged` — the channel silently
    /// drops notifications otherwise.
    #[must_use]
    pub fn subscribe(&self) -> InstanceEventStream {
        self.events_tx.subscribe()
    }

    /// Insert a freshly-spawned handle. Caller owns key generation
    /// (adapter-side: `InstanceKey::new_v4()`). Empty-registry →
    /// auto-focus the inserted key; emits `InstancesChanged` + (on
    /// first spawn) `InstancesFocused` in that order.
    pub async fn insert(&self, key: InstanceKey, handle: Arc<H>) -> AdapterResult<()> {
        let mut instances = self.instances.lock().await;
        let mut order = self.order.lock().await;
        if instances.contains_key(&key) {
            return Err(AdapterError::InvalidRequest(format!(
                "instance '{key}' already registered"
            )));
        }
        instances.insert(key, handle);
        order.push(key);
        let ids: Vec<String> = order.iter().map(|k| k.as_string()).collect();
        drop(instances);
        drop(order);

        let newly_focused = {
            let mut focused = self.focused.write().await;
            if focused.is_none() {
                *focused = Some(key);
                true
            } else {
                false
            }
        };

        let focused_id = self.focused.read().await.map(|k| k.as_string());
        let _ = self.events_tx.send(InstanceEvent::InstancesChanged {
            instance_ids: ids,
            focused_id,
        });
        if newly_focused {
            let _ = self.events_tx.send(InstanceEvent::InstancesFocused {
                instance_id: Some(key.as_string()),
            });
        }
        Ok(())
    }

    /// Insert at a specific insertion-order slot. Used by `restart`
    /// to keep the instance's ordering position across the
    /// drop → respawn swap. Out-of-bounds `slot` appends.
    pub async fn insert_at_slot(&self, slot: usize, key: InstanceKey, handle: Arc<H>) -> AdapterResult<()> {
        let mut instances = self.instances.lock().await;
        let mut order = self.order.lock().await;
        if instances.contains_key(&key) {
            return Err(AdapterError::InvalidRequest(format!(
                "instance '{key}' already registered"
            )));
        }
        instances.insert(key, handle);
        if slot >= order.len() {
            order.push(key);
        } else {
            order.insert(slot, key);
        }
        let ids: Vec<String> = order.iter().map(|k| k.as_string()).collect();
        drop(instances);
        drop(order);

        let focused_id = self.focused.read().await.map(|k| k.as_string());
        let _ = self.events_tx.send(InstanceEvent::InstancesChanged {
            instance_ids: ids,
            focused_id,
        });
        Ok(())
    }

    /// Remove an entry by key. Returns the handle so the adapter can
    /// shut it down without the registry lock held. Does NOT emit
    /// events — callers (`shutdown_one`, `drop_preserving_slot`)
    /// fire the right events after the drop settles.
    pub async fn remove(&self, key: InstanceKey) -> AdapterResult<Arc<H>> {
        let mut instances = self.instances.lock().await;
        let mut order = self.order.lock().await;
        let Some(handle) = instances.remove(&key) else {
            return Err(AdapterError::InvalidRequest(format!(
                "instance '{key}' not found in registry"
            )));
        };
        order.retain(|k| k != &key);
        Ok(handle)
    }

    /// Direct handle lookup. `None` when `key` is unknown. Used by
    /// ACP's submit path to address an existing instance's mpsc;
    /// generic code sticks to `info_for`.
    pub async fn get(&self, key: InstanceKey) -> Option<Arc<H>> {
        self.instances.lock().await.get(&key).cloned()
    }

    /// Snapshot of every live instance. Ordering matches insertion
    /// order (stable across ticks; the UI renders instance tabs off
    /// this).
    pub async fn list(&self) -> Vec<InstanceInfo> {
        let instances = self.instances.lock().await;
        let order = self.order.lock().await;
        order
            .iter()
            .filter_map(|k| instances.get(k).map(|h| h.info()))
            .collect()
    }

    /// Single-instance lookup for `instances/info`.
    pub async fn info_for(&self, key: InstanceKey) -> AdapterResult<InstanceInfo> {
        let instances = self.instances.lock().await;
        let handle = instances
            .get(&key)
            .ok_or_else(|| AdapterError::InvalidRequest(format!("instance '{key}' not found in registry")))?;
        Ok(handle.info())
    }

    /// Current focus pointer.
    pub async fn focused_id(&self) -> Option<InstanceKey> {
        *self.focused.read().await
    }

    /// Resolve a wire-supplied token to a key. Tries hyphenated UUID
    /// v4 parse first; on miss, scans live instances for a matching
    /// captain-set `name`. Returns `None` when neither matches.
    ///
    /// The two-stage lookup is unambiguous because `validate_instance_name`
    /// caps names at 16 chars (UUIDs are 36) and rejects hyphens-only-
    /// at-positions-8/13/18/23 — no slug-form name can collide with
    /// a hyphenated UUID's shape.
    pub async fn resolve_token(&self, token: &str) -> Option<InstanceKey> {
        if let Ok(key) = InstanceKey::parse(token) {
            let instances = self.instances.lock().await;
            if instances.contains_key(&key) {
                return Some(key);
            }
        }
        let instances = self.instances.lock().await;
        for (key, handle) in instances.iter() {
            if handle.name().await.as_deref() == Some(token) {
                return Some(*key);
            }
        }
        None
    }

    /// Rename an instance. `new_name == None` clears the name. Validates
    /// the slug shape + uniqueness across live instances (the same name
    /// can be reused after the prior holder shuts down). Broadcasts
    /// `InstanceRenamed` so the UI updates row labels without a refetch.
    pub async fn rename(&self, key: InstanceKey, new_name: Option<String>) -> AdapterResult<()> {
        let validated = match new_name {
            Some(n) => Some(super::instance::validate_instance_name(&n)?),
            None => None,
        };
        let instances = self.instances.lock().await;
        let target = instances
            .get(&key)
            .ok_or_else(|| AdapterError::InvalidRequest(format!("instance '{key}' not found in registry")))?
            .clone();
        if let Some(name_str) = &validated {
            for (other_key, other) in instances.iter() {
                if *other_key == key {
                    continue;
                }
                if other.name().await.as_deref() == Some(name_str.as_str()) {
                    return Err(AdapterError::InvalidRequest(format!(
                        "instance name '{name_str}' already in use"
                    )));
                }
            }
        }
        drop(instances);
        target.set_name(validated.clone()).await;
        let _ = self.events_tx.send(InstanceEvent::InstanceRenamed {
            instance_id: key.as_string(),
            name: validated,
        });
        Ok(())
    }

    /// Ordered list of every live key. Sized for `list` + event
    /// broadcast payloads.
    pub async fn ordered_keys(&self) -> Vec<InstanceKey> {
        self.order.lock().await.clone()
    }

    /// Designate the focused instance. Unknown id → `InvalidRequest`.
    /// Membership check + focus write happen under the same locks —
    /// a concurrent `shutdown_one` cannot race the check past us.
    pub async fn focus(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
        let instances = self.instances.lock().await;
        if !instances.contains_key(&key) {
            return Err(AdapterError::InvalidRequest(format!(
                "instance '{key}' not found in registry"
            )));
        }
        let mut focused = self.focused.write().await;
        *focused = Some(key);
        drop(focused);
        drop(instances);
        let _ = self.events_tx.send(InstanceEvent::InstancesFocused {
            instance_id: Some(key.as_string()),
        });
        Ok(key)
    }

    /// Best-effort shutdown of the instance at `key`. Broadcasts
    /// `InstancesChanged` after the drop settles + `InstancesFocused`
    /// if the shut-down instance was focused (auto-focus picks the
    /// oldest survivor, or clears to `None`).
    ///
    /// **Race:** the actor's shutdown ack is awaited without any
    /// registry locks held. A concurrent `insert` can land on
    /// `order.first()` before `auto_focus_after_drop` runs — if the
    /// shut-down instance was focused, the brand-new key becomes the
    /// auto-focus target rather than the oldest pre-existing
    /// survivor. Documented behavior; the UI reconciles via the
    /// event stream regardless.
    pub async fn shutdown_one(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
        let handle = self.remove(key).await?;
        handle.shutdown().await;

        self.auto_focus_after_drop(key).await;

        let ids: Vec<String> = self.order.lock().await.iter().map(|k| k.as_string()).collect();
        let focused_id = self.focused.read().await.map(|k| k.as_string());
        let _ = self.events_tx.send(InstanceEvent::InstancesChanged {
            instance_ids: ids,
            focused_id,
        });
        Ok(key)
    }

    /// Tear down the instance at `key`, returning the insertion-order
    /// slot so the adapter can respawn + reinsert at the same
    /// position. Used by `restart` to preserve ordering across the
    /// swap — without it, restart would re-order the instance to the
    /// tail and auto-focus would pick the wrong key on the next
    /// shutdown.
    pub async fn drop_preserving_slot(&self, key: InstanceKey) -> AdapterResult<usize> {
        let slot = {
            let order = self.order.lock().await;
            order.iter().position(|k| k == &key).unwrap_or(order.len())
        };
        let handle = self.remove(key).await?;
        handle.shutdown().await;
        Ok(slot)
    }

    /// Drain every live instance. Called from `daemon::shutdown`
    /// before `app.exit(0)`. Resets focus to `None` without emitting
    /// events — the process is on its way out.
    pub async fn drain(&self) -> Vec<Arc<H>> {
        let instances: Vec<Arc<H>> = {
            let mut instances = self.instances.lock().await;
            let mut order = self.order.lock().await;
            order.clear();
            instances.drain().map(|(_, v)| v).collect()
        };
        *self.focused.write().await = None;
        instances
    }

    async fn auto_focus_after_drop(&self, dropped: InstanceKey) {
        let mut focused = self.focused.write().await;
        if *focused != Some(dropped) {
            return;
        }
        let order = self.order.lock().await;
        let next = order.first().copied();
        *focused = next;
        let _ = self.events_tx.send(InstanceEvent::InstancesFocused {
            instance_id: next.map(|k| k.as_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DummyInstance {
        id: InstanceKey,
        agent_id: String,
        profile_id: Option<String>,
        mode: Option<String>,
        shutdown_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl InstanceActor for DummyInstance {
        fn info(&self) -> InstanceInfo {
            InstanceInfo {
                id: self.id.as_string(),
                name: None,
                agent_id: self.agent_id.clone(),
                profile_id: self.profile_id.clone(),
                session_id: None,
                mode: self.mode.clone(),
            }
        }

        async fn name(&self) -> Option<String> {
            None
        }

        async fn set_name(&self, _name: Option<String>) {}

        async fn shutdown(&self) {
            self.shutdown_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn dummy(id: InstanceKey, agent: &str) -> Arc<DummyInstance> {
        Arc::new(DummyInstance {
            id,
            agent_id: agent.into(),
            profile_id: None,
            mode: None,
            shutdown_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn dummy_with_mode(id: InstanceKey, agent: &str, mode: Option<&str>) -> Arc<DummyInstance> {
        Arc::new(DummyInstance {
            id,
            agent_id: agent.into(),
            profile_id: None,
            mode: mode.map(str::to_string),
            shutdown_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    #[tokio::test]
    async fn empty_registry_first_insert_auto_focuses() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let mut rx = reg.subscribe();
        let k = InstanceKey::new_v4();
        reg.insert(k, dummy(k, "a")).await.expect("insert");
        assert_eq!(reg.focused_id().await, Some(k));

        let e = rx.recv().await.expect("changed");
        assert!(matches!(e, InstanceEvent::InstancesChanged { .. }));
        let e = rx.recv().await.expect("focused");
        match e {
            InstanceEvent::InstancesFocused { instance_id } => {
                assert_eq!(instance_id, Some(k.as_string()));
            }
            other => panic!("expected InstancesFocused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn second_insert_does_not_steal_focus() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k1 = InstanceKey::new_v4();
        let k2 = InstanceKey::new_v4();
        reg.insert(k1, dummy(k1, "a")).await.unwrap();
        reg.insert(k2, dummy(k2, "b")).await.unwrap();
        assert_eq!(reg.focused_id().await, Some(k1));
    }

    #[tokio::test]
    async fn focus_unknown_is_invalid_request() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let err = reg.focus(InstanceKey::new_v4()).await.expect_err("must fail");
        assert!(matches!(err, AdapterError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn shutdown_focused_auto_focuses_oldest_survivor() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k1 = InstanceKey::new_v4();
        let k2 = InstanceKey::new_v4();
        reg.insert(k1, dummy(k1, "a")).await.unwrap();
        reg.insert(k2, dummy(k2, "b")).await.unwrap();
        reg.focus(k2).await.unwrap();
        reg.shutdown_one(k2).await.unwrap();
        assert_eq!(reg.focused_id().await, Some(k1));
    }

    #[tokio::test]
    async fn shutdown_last_clears_focus() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k = InstanceKey::new_v4();
        reg.insert(k, dummy(k, "a")).await.unwrap();
        reg.shutdown_one(k).await.unwrap();
        assert!(reg.focused_id().await.is_none());
    }

    #[tokio::test]
    async fn drop_preserving_slot_returns_position() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k1 = InstanceKey::new_v4();
        let k2 = InstanceKey::new_v4();
        let k3 = InstanceKey::new_v4();
        reg.insert(k1, dummy(k1, "a")).await.unwrap();
        reg.insert(k2, dummy(k2, "b")).await.unwrap();
        reg.insert(k3, dummy(k3, "c")).await.unwrap();
        let slot = reg.drop_preserving_slot(k2).await.unwrap();
        assert_eq!(slot, 1);
    }

    /// Restart-preserves-order: drop_preserving_slot returns the
    /// slot; insert_at_slot puts the new handle back at the same
    /// index so the registry's insertion order is identical post-swap.
    #[tokio::test]
    async fn insert_at_slot_preserves_insertion_order() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k1 = InstanceKey::new_v4();
        let k2 = InstanceKey::new_v4();
        let k3 = InstanceKey::new_v4();
        reg.insert(k1, dummy(k1, "a")).await.unwrap();
        reg.insert(k2, dummy(k2, "b")).await.unwrap();
        reg.insert(k3, dummy(k3, "c")).await.unwrap();

        let slot = reg.drop_preserving_slot(k2).await.unwrap();
        let k2_new = InstanceKey::new_v4();
        reg.insert_at_slot(slot, k2_new, dummy(k2_new, "b")).await.unwrap();

        let order = reg.ordered_keys().await;
        assert_eq!(order, vec![k1, k2_new, k3]);
    }

    /// TOCTOU regression: the held-lock pattern in `focus` prevents
    /// a concurrent `shutdown_one` from slipping between the
    /// membership check and the focus write. We can't reliably
    /// reproduce the race without injection points, but we pin the
    /// happy path + assert that `focus(k)` on a removed key fails
    /// with `InvalidRequest` rather than stamping a ghost.
    #[tokio::test]
    async fn focus_after_remove_rejects() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k = InstanceKey::new_v4();
        reg.insert(k, dummy(k, "a")).await.unwrap();
        reg.shutdown_one(k).await.unwrap();
        let err = reg.focus(k).await.expect_err("gone");
        assert!(matches!(err, AdapterError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn list_returns_inserted_info() {
        let reg: AdapterRegistry<DummyInstance> = AdapterRegistry::new();
        let k = InstanceKey::new_v4();
        reg.insert(k, dummy_with_mode(k, "agent-x", Some("plan")))
            .await
            .unwrap();
        let items = reg.list().await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].agent_id, "agent-x");
        assert_eq!(items[0].mode.as_deref(), Some("plan"));
    }
}
