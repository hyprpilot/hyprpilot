//! Transport-agnostic adapter layer.
//!
//! An `Adapter` is a way to talk to an agent — today ACP subprocesses,
//! tomorrow HTTP-based vendor APIs. This module owns the generic
//! vocabulary every adapter impl emits: instance handles, transcript
//! items, permission prompts, tool-call records, profile config.
//! ACP-specific shapes live under `adapters::acp` and bridge into the
//! generic types via `From`/`TryFrom` in `adapters::acp::mapping`.
//!
//! The split: `rpc::` / `ctl::` / `daemon::` / `config::` only ever
//! import from `adapters::*`. They do not reach into `adapters::acp::*`
//! — that's a layering violation, caught by the lint guard.

// Pass 1 scaffold: generic types + trait land here; Pass 2 relocates
// ACP code behind them and wires every consumer. Until then some items
// read as unused — the `EchoAdapter` test exercises the trait surface.
#![allow(dead_code, unused_imports)]

pub mod acp;
pub mod instance;
pub mod permission;
pub mod profile;
pub mod tool;
pub mod transcript;

use async_trait::async_trait;

pub use instance::{InstanceEvent, InstanceEventStream, InstanceHandle, InstanceKey, InstanceState};
pub use permission::{PermissionOptionView, PermissionPrompt, PermissionReply};
pub use profile::{AgentConfig, AgentProvider, ProfileConfig, ResolvedInstance};
pub use tool::{ToolCall, ToolCallContent, ToolState};
pub use transcript::{ToolCallRecord, TranscriptItem, TurnRecord, UserTurnInput};

/// Closed set of known transport kinds. The string wire-name is stable
/// — it appears in tracing spans and (future) config `transport =
/// "acp"` fields. New adapter → new variant + new impl + new match arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterId {
    Acp,
}

impl AdapterId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AdapterId::Acp => "acp",
        }
    }
}

/// Static capability bits each adapter advertises at construction. Used
/// by UI pickers to disable features against adapters that don't
/// advertise the underlying hook (e.g. "resume session" is ACP-only until
/// an HTTP vendor ships its own equivalent). Bool rather than
/// `Option<…>` because every field is a yes/no feature flag.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub load_session: bool,
    pub list_sessions: bool,
    pub permissions: bool,
    pub terminals: bool,
}

/// Structured adapter-level error. Every adapter impl maps its
/// transport-specific error variants onto these. `rpc::` / `ctl::` /
/// Tauri commands only ever see this enum.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// Backend (process / HTTP endpoint / library) responded with an
    /// operational failure — timeout, refused connection, vendor
    /// error. Body is a human-readable string. Maps to `-32603` at the
    /// JSON-RPC boundary.
    #[error("adapter backend: {0}")]
    Backend(String),
    /// Caller supplied invalid params — unknown agent id, unknown
    /// profile id, bad shape. Maps to `-32602` at the JSON-RPC
    /// boundary.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Feature not supported by this adapter (e.g. `load_session`
    /// against an adapter whose `Capabilities::load_session` is
    /// `false`). Maps to `-32601` at the JSON-RPC boundary.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type AdapterResult<T> = Result<T, AdapterError>;

/// Bootstrap discriminator for `start_instance`. `Fresh` spawns a new
/// session; `Resume` rebinds an existing one; `ListOnly` spawns an
/// ephemeral actor that serves `list_sessions` + `shutdown` without
/// ever binding to a session. Shared across adapter impls — today
/// only ACP consumes it, but the semantics (`new` vs `load` vs
/// `init-only`) translate to any transport that owns session state.
#[derive(Debug, Clone)]
pub enum Bootstrap {
    Fresh,
    /// Session id is opaque to the generic layer — impls parse it into
    /// their own wire type on receipt.
    Resume(String),
    ListOnly,
}

/// Primary trait every transport impl satisfies. Future `HttpAdapter`
/// slots in here next to `AcpAdapter`.
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    fn id(&self) -> AdapterId;

    fn capabilities(&self) -> Capabilities;

    /// Spawn a new instance for the resolved `(agent, profile)` pair.
    /// Registers the instance internally keyed by `InstanceKey`;
    /// returns the handle callers keep to address follow-up submits /
    /// cancels against.
    async fn start_instance(
        &self,
        resolved: ResolvedInstance,
        bootstrap: Bootstrap,
    ) -> AdapterResult<InstanceHandle>;

    /// Submit a prompt against the addressed `(agent_id, profile_id)`
    /// pair, spawning a new instance on first hit or reusing the live
    /// one. Returns a JSON envelope the RPC / Tauri surfaces pass
    /// straight through.
    async fn submit(
        &self,
        text: &str,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> AdapterResult<serde_json::Value>;

    /// Cancel the active turn on the addressed agent's instance.
    async fn cancel(&self, agent_id: Option<&str>) -> AdapterResult<serde_json::Value>;

    /// Snapshot of every live instance the adapter owns.
    async fn info(&self) -> AdapterResult<serde_json::Value>;

    /// Best-effort drain of every live instance. Called from
    /// `daemon::shutdown` before `app.exit(0)`.
    async fn shutdown(&self);
}

#[cfg(test)]
mod tests {
    //! Throwaway shape-reusability check: a trivial `EchoAdapter` must
    //! satisfy the `Adapter` trait without reaching into
    //! `adapters::acp`. Pins the trait's transport-agnosticism at
    //! compile + test time.

    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct EchoAdapter;

    #[async_trait]
    impl Adapter for EchoAdapter {
        fn id(&self) -> AdapterId {
            AdapterId::Acp
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }

        async fn start_instance(
            &self,
            _resolved: ResolvedInstance,
            _bootstrap: Bootstrap,
        ) -> AdapterResult<InstanceHandle> {
            Err(AdapterError::Unsupported("echo: start_instance".into()))
        }

        async fn submit(
            &self,
            text: &str,
            _agent_id: Option<&str>,
            _profile_id: Option<&str>,
        ) -> AdapterResult<serde_json::Value> {
            Ok(json!({ "echo": text }))
        }

        async fn cancel(&self, _agent_id: Option<&str>) -> AdapterResult<serde_json::Value> {
            Ok(json!({ "cancelled": false }))
        }

        async fn info(&self) -> AdapterResult<serde_json::Value> {
            Ok(json!({ "sessions": [] }))
        }

        async fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn echo_adapter_satisfies_trait() {
        let a: Box<dyn Adapter> = Box::new(EchoAdapter);
        assert_eq!(a.id(), AdapterId::Acp);
        let v = a.submit("hi", None, None).await.expect("echo submit ok");
        assert_eq!(v["echo"], "hi");
        let info = a.info().await.expect("info ok");
        assert_eq!(info["sessions"], json!([]));
    }
}
