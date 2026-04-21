//! Session registry shared across the RPC + Tauri command surfaces.
//!
//! `AcpSessions` is the single Tauri managed-state entry the RPC
//! `SessionHandler` and the webview-facing `acp_*` Tauri commands
//! both reach into. Today the registry is skeletal — `submit` /
//! `cancel` / `info` return the same shapes the pre-K-239 handlers
//! did (so the wire contract holds while the runtime lands).
//!
//! The live-session wiring (spawn child → `ClientSideConnection` →
//! drive prompts) is staged as a follow-up; see the module docs on
//! `acp::mod` for the runway.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::AgentsConfig;
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

/// Registry-level agent identity. Matches `AgentConfig::id` on the
/// config side. Wrapper type keeps accidental `String` swaps at API
/// boundaries obvious. `dead_code` until the live-session follow-up
/// wires it into the registry map keys.
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

/// Lifecycle of a single live session. Drives the `AgentState`
/// broadcast through `StatusBroadcast::set`, and fans out via
/// `acp:session-state` Tauri events to the webview.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AcpSessionState {
    Idle,
    Streaming { started_at: Instant },
    AwaitingPermission,
    Failed { error: String },
}

/// Top-level registry held as Tauri managed state.
///
/// Today's public surface is the three stubs (`submit`, `cancel`,
/// `info`) that the `SessionHandler` delegates into. Each returns the
/// pre-K-239 shape so the JSON-RPC contract stays stable while the
/// live-session path lands in the next pass.
#[derive(Debug)]
pub struct AcpSessions {
    #[allow(dead_code)]
    pub(crate) config: AgentsConfig,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
}

impl AcpSessions {
    /// Construct an empty registry from the resolved `AgentsConfig` +
    /// shared status broadcast. Held in Tauri managed state; every
    /// handler reaches this via `app.state::<AcpSessions>()` or
    /// `HandlerCtx.sessions`.
    #[must_use]
    pub fn new(config: AgentsConfig, status: Arc<StatusBroadcast>) -> Self {
        Self { config, status }
    }

    /// `session/submit` handler body. Today: echo-back stub matching
    /// the pre-K-239 `CoreHandler` shape so downstream peers
    /// (`ctl submit`, tests, waybar) see an unchanged wire response.
    pub async fn submit(&self, text: &str, _agent_id: Option<&str>) -> Result<Value, RpcError> {
        Ok(json!({ "accepted": true, "text": text }))
    }

    /// `session/cancel` handler body. Stub today; wires through to
    /// `conn.cancel(CancelNotification)` on the addressed session in
    /// the live-session follow-up.
    pub async fn cancel(&self, _agent_id: Option<&str>) -> Result<Value, RpcError> {
        Ok(json!({
            "cancelled": false,
            "reason": "no active session",
        }))
    }

    /// `session/info` handler body. Stub today; returns the real
    /// per-agent session list in the live-session follow-up.
    pub async fn info(&self) -> Result<Value, RpcError> {
        Ok(json!({ "sessions": [] }))
    }

    /// Gracefully shut down every live session before the daemon exits.
    ///
    /// Today: no-op + a log line — the registry is empty (live session
    /// plumbing lands in the K-239 follow-up). The live version walks
    /// the registry, `conn.cancel(CancelNotification { .. })` each
    /// session, waits briefly for child drain, then drops the handles
    /// (the `tokio::process::Child` inside each session handle has
    /// `kill_on_drop(true)` to catch anything that didn't exit
    /// cleanly).
    ///
    /// Called from `rpc::server::shutdown_daemon` after the
    /// `{"killed": true}` response has flushed to the peer. Explicit
    /// orchestration, not drop-order implicit: when the live path
    /// lands we want a single well-known entry point to thread
    /// graceful-timeout logic through, not scattered `Drop` impls.
    pub async fn shutdown(&self) {
        // Live-session follow-up replaces this with a real walk. The
        // hook exists now so `shutdown_daemon` has a stable call
        // surface that doesn't grow branches when sessions actually
        // exist.
        tracing::debug!("acp::shutdown: registry is empty (scaffold)");
    }
}
