//! Agent Client Protocol adapter.
//!
//! Live runtime active. One `tokio::spawn`-ed actor per instance
//! (`runtime::run_instance`) owns the child + `ConnectionTo<Agent>`;
//! `AcpInstances` is the Tauri-managed registry of those actors.
//! `session/submit` spawns the vendor on first hit then reuses the
//! live session for follow-up prompts.
//!
//! `ClientSideConnection` is not a thing in `agent-client-protocol` 0.11
//! — the crate uses `Client.builder().connect_with(transport, main_fn)`.
//! `Send`-safe throughout; no `LocalSet` required.
//!
//! Permission handling routes through `DefaultPermissionController`
//! (K-245): profile reject/accept globs resolve without UI traffic,
//! and only the `AskUser` path reaches the webview as
//! `acp:permission-request`. The call-site wraps the waiter's
//! `rx.await` in `tokio::time::timeout(WAITER_TIMEOUT, rx)` so a
//! prompt left unanswered for 10 minutes falls through to
//! `Cancelled` without wedging the ACP session.
//!
//! Layering: nothing outside `src-tauri/src/adapters/` may
//! `use crate::adapters::acp::*`. That's a layering violation — the
//! rest of the crate talks to `dyn Adapter` or to the concrete
//! `AcpInstances` registry (re-exported at the adapter root).

pub mod agents;
pub mod client;
pub mod instance;
pub mod instances;
pub mod mapping;
pub mod resolve;
pub mod runtime;
pub mod spawn;

pub use instances::AcpInstances;
#[allow(unused_imports)]
pub use instances::InstanceKey;

use async_trait::async_trait;
use std::sync::Arc;

use super::{
    Adapter, AdapterError, AdapterId, AdapterResult, Bootstrap, Capabilities, InstanceHandle, ResolvedInstance,
    UserTurnInput,
};
use crate::rpc::protocol::RpcError;

/// JSON-RPC error → adapter error. `-32602` is a caller-side shape
/// problem (`InvalidRequest`); every other code — internal errors,
/// transport / backend failures, hyprpilot-specific codes in the
/// `-32000..-32099` range — surfaces as `Backend`.
fn map_rpc_error(err: RpcError) -> AdapterError {
    match err.code {
        -32602 => AdapterError::InvalidRequest(err.message),
        _ => AdapterError::Backend(err.message),
    }
}

/// Thin wrapper that `impl Adapter for AcpAdapter` so the rest of the
/// crate can interact with `dyn Adapter`. Composes the live
/// `AcpInstances` registry; vendor quirks (per-agent
/// `Box<dyn AcpAgent>`) are resolved inside `runtime::run_instance`
/// via `agents::match_provider_agent`, so `AcpAdapter` itself stays a
/// single-field struct.
#[derive(Clone)]
pub struct AcpAdapter {
    instances: Arc<AcpInstances>,
}

impl AcpAdapter {
    #[must_use]
    pub fn new(instances: Arc<AcpInstances>) -> Self {
        Self { instances }
    }
}

#[async_trait]
impl Adapter for AcpAdapter {
    fn id(&self) -> AdapterId {
        AdapterId::Acp
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            load_session: true,
            list_sessions: true,
            permissions: true,
            terminals: true,
        }
    }

    async fn start_instance(&self, resolved: ResolvedInstance, bootstrap: Bootstrap) -> AdapterResult<InstanceHandle> {
        let agent_id = resolved.agent.id.clone();
        let key = self
            .instances
            .ensure(InstanceKey::new_v4(), resolved, bootstrap.into())
            .await
            .map_err(map_rpc_error)?;
        Ok(InstanceHandle {
            agent_id,
            instance_id: key.as_string(),
            session_id: None,
        })
    }

    async fn submit(
        &self,
        input: UserTurnInput,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> AdapterResult<serde_json::Value> {
        let UserTurnInput::Text(text) = input;
        self.instances
            .submit(&text, None, agent_id, profile_id)
            .await
            .map_err(map_rpc_error)
    }

    async fn cancel(&self, agent_id: Option<&str>) -> AdapterResult<serde_json::Value> {
        self.instances.cancel(None, agent_id).await.map_err(map_rpc_error)
    }

    async fn info(&self) -> AdapterResult<serde_json::Value> {
        self.instances.info().await.map_err(map_rpc_error)
    }

    async fn shutdown(&self) {
        self.instances.shutdown().await;
    }
}
