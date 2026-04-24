//! Generic actor-lifecycle vocabulary. Each adapter owns a registry
//! of live instances keyed by `InstanceKey`. An instance is our record
//! of a running agent process (or HTTP session) + its channels — it
//! outlives any single wire-session cycle and survives re-binds.
//!
//! "session" is reserved for the adapter's wire concept (e.g. the
//! ACP session id the agent issues via `session/new`); "instance" is
//! always our owner/record. See the CLAUDE.md glossary.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::permission::PermissionOptionView;
use super::AdapterError;

/// Registry key. A UUID per live instance — collisions across twin
/// profiles are impossible by construction. Wire shape is the v4
/// hyphenated string; the UI treats it as opaque.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct InstanceKey(pub Uuid);

impl InstanceKey {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a wire string (hyphenated v4). Adapters propagate the
    /// `AdapterError::InvalidRequest` to their protocol layer
    /// (`-32602` for JSON-RPC).
    pub fn parse(s: &str) -> Result<Self, AdapterError> {
        if s.is_empty() {
            return Err(AdapterError::InvalidRequest("instance id cannot be empty".into()));
        }
        s.parse::<Uuid>()
            .map(Self)
            .map_err(|e| AdapterError::InvalidRequest(format!("invalid instance_id '{s}': {e}")))
    }

    #[must_use]
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Lifecycle phases an instance steps through. Adapters broadcast one
/// `InstanceEvent::State` per transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    /// Actor spawned; handshake in progress.
    Starting,
    /// Live — accepting prompts / cancels.
    Running,
    /// Clean teardown.
    Ended,
    /// Terminal failure (spawn failed, handshake rejected, wire
    /// protocol returned an error).
    Error,
}

/// Upstream events an adapter's instances emit. Registry bridges
/// these onto Tauri `acp:*` events today; future HTTP adapters would
/// bridge onto their own namespace.
///
/// Two naming axes:
/// - **Tauri event names** (colon-separated, consumed by `app.emit`):
///   `acp:instance-state`, `acp:instances-changed`, etc. The bridge
///   layer owns the mapping.
/// - **Topic strings** (dot-separated, returned by
///   [`InstanceEvent::topic`]): `instance.state`,
///   `instances.changed`. Used by tracing spans and the future
///   subscription filter layer (K-276).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InstanceEvent {
    State {
        agent_id: String,
        instance_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        state: InstanceState,
    },
    Transcript {
        agent_id: String,
        instance_id: String,
        session_id: String,
        update: serde_json::Value,
    },
    PermissionRequest {
        agent_id: String,
        instance_id: String,
        session_id: String,
        request_id: String,
        tool: String,
        kind: String,
        args: String,
        options: Vec<PermissionOptionView>,
    },
    /// Registry membership changed — an instance spawned, shut down,
    /// or restarted. `instance_ids` is the full post-change set;
    /// `focused_id` echoes the current focus so consumers reconcile
    /// both bits in a single event.
    InstancesChanged {
        instance_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        focused_id: Option<String>,
    },
    /// Focus pointer moved. `instance_id` is `None` when the registry
    /// emptied and no auto-focus target exists.
    InstancesFocused {
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
    },
}

impl InstanceEvent {
    /// Dot-separated topic name. Stable; the K-276 filter layer
    /// subscribes on these. Colon-separated Tauri event names live
    /// only in the bridge's mapping table.
    #[must_use]
    pub fn topic(&self) -> &'static str {
        match self {
            InstanceEvent::State { .. } => "instance.state",
            InstanceEvent::Transcript { .. } => "instance.transcript",
            InstanceEvent::PermissionRequest { .. } => "instance.permission_request",
            InstanceEvent::InstancesChanged { .. } => "instances.changed",
            InstanceEvent::InstancesFocused { .. } => "instances.focused",
        }
    }
}

/// Handle returned by `Adapter::spawn`. Holds just enough identity for
/// callers to address follow-up submits + cancels against; the
/// concrete channels live inside the adapter's own registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceHandle {
    pub agent_id: String,
    pub instance_id: String,
    /// Populated once the wire-session lands (e.g. `session/new`
    /// resolves on ACP). `None` while the instance is still
    /// bootstrapping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Flat snapshot of one live instance. Adapters surface the same
/// shape regardless of transport — consumers (RPC / UI pickers)
/// render off this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Per-instance operational mode (e.g. `plan` / `edit` for
    /// claude-code). `Some` when the spawning profile set one;
    /// `None` otherwise. Adapters interpret it at spawn time — the
    /// generic layer only carries + surfaces the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Stream typealias — `broadcast::Receiver` per subscriber. One
/// instance of events multiplexed across the daemon's consumers
/// (Tauri bridge, tests, future UI pickers).
///
/// Every consumer MUST handle `broadcast::error::RecvError::Lagged`
/// explicitly — the broadcast channel silently drops messages to the
/// lagging subscriber otherwise. The Tauri bridge does this in
/// `AcpAdapter::spawn_tauri_event_bridge`; new subscribers added by
/// K-276 / K-277 must too.
pub type InstanceEventStream = broadcast::Receiver<InstanceEvent>;

/// Per-actor contract the generic registry needs from each adapter's
/// handle type. ACP's `AcpInstance` implements this; a future
/// `HttpInstance` would too.
#[async_trait]
pub trait InstanceActor: Send + Sync + 'static {
    /// Identity + metadata snapshot for `list` / `info_for`.
    fn info(&self) -> InstanceInfo;

    /// Drain the actor. Best-effort — the registry timeouts on the
    /// ack so a wedged actor can't block shutdown.
    async fn shutdown(&self);
}

/// Parameter bundle for `Adapter::spawn`. Every field is optional;
/// adapters fall through to their own default-chain when a slot is
/// `None`. Constructed at the RPC / Tauri command boundary from user
/// input; the adapter then resolves it against its config.
#[derive(Debug, Default, Clone)]
pub struct SpawnSpec {
    pub profile_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub model: Option<String>,
}
