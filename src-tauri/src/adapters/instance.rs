//! Generic actor-lifecycle vocabulary. Each adapter owns a registry
//! of `Instance`s keyed by `InstanceKey`. An instance is our record
//! of a running agent process (or HTTP session) + its channels — it
//! outlives any single wire-session cycle and survives re-binds.
//!
//! "session" is reserved for the adapter's wire concept (e.g. the
//! ACP session id the agent issues via `session/new`); "instance" is
//! always our owner/record. See the CLAUDE.md glossary.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::permission::PermissionOptionView;

/// Registry key — `(agent_id, profile_id?)`. `profile_id` is `None`
/// for bare-agent resolutions. Two calls with the same `agent_id` but
/// distinct profiles get distinct instances — profile switch bakes a
/// new system prompt / model at spawn time, so a mid-instance switch
/// would be misleading.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceKey {
    pub agent_id: String,
    pub profile_id: Option<String>,
}

impl InstanceKey {
    /// Stable string projection — used as the `instance_id` field on
    /// Tauri events and as a log-span discriminator. `agent_id` when
    /// `profile_id` is absent; `agent_id:profile_id` otherwise.
    #[must_use]
    pub fn as_string(&self) -> String {
        match &self.profile_id {
            Some(p) => format!("{}:{}", self.agent_id, p),
            None => self.agent_id.clone(),
        }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
        options: Vec<PermissionOptionView>,
    },
}

/// Handle returned by `Adapter::start_instance`. Holds just enough
/// identity for callers to address follow-up submits + cancels
/// against; the concrete channels live inside the adapter's own
/// registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceHandle {
    pub key: InstanceKey,
    /// Populated once the wire-session lands (e.g. `session/new`
    /// resolves on ACP). `None` while the instance is still
    /// bootstrapping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Stream typealias — `broadcast::Receiver` per subscriber. One
/// instance of events multiplexed across the daemon's consumers
/// (Tauri bridge, tests, future UI pickers).
pub type InstanceEventStream = broadcast::Receiver<InstanceEvent>;
