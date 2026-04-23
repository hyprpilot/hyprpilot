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
//! Permission handling is auto-`Cancelled` today; the
//! `PermissionController` issue replaces the auto-deny with a real
//! trust-store. Every `session/request_permission` still reaches the
//! webview as `acp:permission-request` for observability.
//!
//! Layering: nothing outside `src-tauri/src/adapters/` may
//! `use crate::adapters::acp::*`. That's a layering violation — the
//! rest of the crate talks to `dyn Adapter` or to the concrete
//! `AcpInstances` registry (re-exported at the adapter root).

pub mod agents;
pub mod client;
pub mod commands;
pub mod instance;
pub mod instances;
pub mod mapping;
pub mod resolve;
pub mod runtime;
pub mod spawn;

pub use instances::{AcpInstances, InstanceKey};

use async_trait::async_trait;
use std::sync::Arc;

use super::{
    Adapter, AdapterError, AdapterId, AdapterResult, Bootstrap, Capabilities, InstanceHandle, ResolvedInstance,
};

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

    async fn start_instance(
        &self,
        _resolved: ResolvedInstance,
        _bootstrap: Bootstrap,
    ) -> AdapterResult<InstanceHandle> {
        Err(AdapterError::Unsupported(
            "AcpAdapter::start_instance: Adapter surface not yet wired through AcpInstances (K-251 follow-up)".into(),
        ))
    }

    async fn submit(
        &self,
        text: &str,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> AdapterResult<serde_json::Value> {
        self.instances
            .submit(text, agent_id, profile_id)
            .await
            .map_err(|err| AdapterError::InvalidRequest(err.message))
    }

    async fn cancel(&self, agent_id: Option<&str>) -> AdapterResult<serde_json::Value> {
        self.instances
            .cancel(agent_id)
            .await
            .map_err(|err| AdapterError::InvalidRequest(err.message))
    }

    async fn info(&self) -> AdapterResult<serde_json::Value> {
        self.instances
            .info()
            .await
            .map_err(|err| AdapterError::InvalidRequest(err.message))
    }

    async fn shutdown(&self) {
        self.instances.shutdown().await;
    }
}
