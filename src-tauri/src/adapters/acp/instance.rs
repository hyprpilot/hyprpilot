//! Per-actor owner. One `AcpInstance` per live `(agent_id,
//! profile_id?)` entry in the ACP adapter's registry. Carries the
//! identity bits + the command channel the registry uses to drive
//! the actor.
//!
//! The actor body itself (the long-lived tokio task) lives in
//! `runtime::run_instance`; this struct is just the handle we keep
//! around after spawn.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::SessionId;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use super::runtime::InstanceCommand;
use crate::adapters::instance::{InstanceActor, InstanceInfo, InstanceKey};

/// How long the registry waits for the actor to ack a `Shutdown`
/// command before dropping the handle. Matches the pre-refactor value.
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Handle the registry keeps after `start_instance`. Dropping it
/// cancels the actor (via the `cmd_tx` drop + the actor's select
/// loop observing `None` from the mpsc receiver).
#[derive(Debug)]
pub struct AcpInstance {
    pub key: InstanceKey,
    pub agent_id: String,
    /// `Some` when a `[[profiles]]` entry resolved during ensure,
    /// `None` for bare-agent resolutions (no profile selected). Used
    /// by `info` to report the live instance's origin and by future
    /// UI pickers to group twins by their profile.
    pub profile_id: Option<String>,
    /// Per-instance operational mode. Mirrored onto `InstanceInfo`.
    pub mode: Option<String>,
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

#[async_trait]
impl InstanceActor for AcpInstance {
    fn info(&self) -> InstanceInfo {
        // session_id read is sync-safe on the RwLock's try_read, but
        // we don't need it here — the registry's list path populates
        // it async via `current_session_id` when it matters. Keep
        // this call sync so the generic registry can assemble the
        // snapshot without an async fn on the trait.
        let session_id = self
            .session_id
            .try_read()
            .ok()
            .and_then(|s| s.as_ref().map(|id| id.0.to_string()));
        InstanceInfo {
            id: self.key.as_string(),
            agent_id: self.agent_id.clone(),
            profile_id: self.profile_id.clone(),
            session_id,
            mode: self.mode.clone(),
        }
    }

    async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::Shutdown { reply: tx }).is_err() {
            return;
        }
        let _ = tokio::time::timeout(SHUTDOWN_ACK_TIMEOUT, rx).await;
    }
}
