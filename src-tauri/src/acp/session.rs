//! Session registry shared across the RPC + Tauri command surfaces.
//!
//! Scaffold today — `submit` / `cancel` / `info` return the pre-K-239
//! stub shapes so the wire contract stays stable while the live
//! session runtime lands in the K-239 follow-up.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::AgentsConfig;
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct AgentId(pub String);

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AcpSessionState {
    Idle,
    Streaming { started_at: Instant },
    AwaitingPermission,
    Failed { error: String },
}

#[derive(Debug)]
pub struct AcpSessions {
    #[allow(dead_code)]
    pub(crate) config: AgentsConfig,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
}

impl AcpSessions {
    #[must_use]
    pub fn new(config: AgentsConfig, status: Arc<StatusBroadcast>) -> Self {
        Self { config, status }
    }

    pub async fn submit(&self, text: &str, _agent_id: Option<&str>) -> Result<Value, RpcError> {
        Ok(json!({ "accepted": true, "text": text }))
    }

    pub async fn cancel(&self, _agent_id: Option<&str>) -> Result<Value, RpcError> {
        Ok(json!({ "cancelled": false, "reason": "no active session" }))
    }

    pub async fn info(&self) -> Result<Value, RpcError> {
        Ok(json!({ "sessions": [] }))
    }

    /// Cleanup hook called from `daemon::shutdown` before
    /// `app.exit(0)`. Scaffold today; the live version cancels each
    /// session and waits for child drain.
    pub async fn shutdown(&self) {
        tracing::debug!("acp::shutdown: registry is empty (scaffold)");
    }
}
