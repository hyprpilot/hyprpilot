//! Per-actor owner. One `AcpInstance` per live `(agent_id,
//! profile_id?)` entry in `AcpInstances`. Carries the identity bits +
//! the command channel the registry uses to drive the actor.
//!
//! The actor body itself (the long-lived tokio task) lives in
//! `runtime::run_instance`; this struct is just the handle we keep
//! around after spawn.

use std::sync::Arc;

use agent_client_protocol::schema::SessionId;
use tokio::sync::mpsc;

use super::runtime::InstanceCommand;

/// Handle the registry keeps after `start_instance`. Dropping it
/// cancels the actor (via the `cmd_tx` drop + the actor's select
/// loop observing `None` from the mpsc receiver).
#[derive(Debug)]
pub struct AcpInstance {
    pub agent_id: String,
    /// `Some` when a `[[profiles]]` entry resolved during ensure,
    /// `None` for bare-agent resolutions (no profile selected). Used
    /// by `info` to report the live instance's origin and by future
    /// UI pickers to group twins by their profile.
    pub profile_id: Option<String>,
    pub cmd_tx: mpsc::UnboundedSender<InstanceCommand>,
    /// Populated after the first prompt's `session/new` resolves.
    /// `None` while the instance is still bootstrapping.
    pub session_id: Arc<tokio::sync::RwLock<Option<SessionId>>>,
}

impl AcpInstance {
    pub async fn current_session_id(&self) -> Option<String> {
        self.session_id.read().await.as_ref().map(|id| id.0.to_string())
    }
}
