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

// K-251 follow-up: the generic adapter vocabulary (InstanceHandle,
// Bootstrap, AdapterError, UserTurnInput, …) is the canonical outside
// shape for future non-ACP adapters (HttpAdapter / …). Today only the
// ACP impl + test exercise the trait surface, so many items register
// as dead until a sibling adapter lands and the trait is called through
// `dyn Adapter` by the rpc / ctl layers. Re-check after the second
// adapter; narrow to per-item allows for what remains unused.
#![allow(dead_code)]

pub mod acp;
pub mod commands;
pub mod instance;
pub mod permission;
pub mod profile;
pub mod tool;
pub mod transcript;

use async_trait::async_trait;

// Re-exports are the canonical vocabulary future non-ACP adapters bind
// to. Today only the ACP impl + adapter test exercise most entries;
// drop the allow once the second adapter lands and callers reach them
// through `dyn Adapter` from `rpc::` / `ctl::`.
#[allow(unused_imports)]
pub use instance::{InstanceEvent, InstanceEventStream, InstanceHandle, InstanceState};
#[allow(unused_imports)]
pub use permission::{
    pick_allow_option_id, pick_reject_option_id, Decision, DefaultPermissionController, PermissionController,
    PermissionOptionView, PermissionOutcome, PermissionPrompt, PermissionReply, PermissionRequest, ToolCallRef,
};
#[allow(unused_imports)]
pub use profile::{AgentConfig, AgentProvider, ProfileConfig, ResolvedInstance};
#[allow(unused_imports)]
pub use tool::{ToolCall, ToolCallContent, ToolState};
#[allow(unused_imports)]
pub use transcript::{ToolCallRecord, TranscriptItem, TurnRecord, UserTurnInput};

// Concrete impls we re-export so out-of-layer callers never need to
// type `adapters::acp::*` — only `adapters::*`. Adding an `HttpAdapter`
// sibling later adds new re-exports here, not new import paths in
// `rpc::` / `ctl::` / `daemon::`.
#[allow(unused_imports)]
pub use acp::{AcpAdapter, AcpInstances};

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
    /// Registers the instance internally; returns the handle callers
    /// keep to address follow-up submits / cancels against.
    async fn start_instance(&self, resolved: ResolvedInstance, bootstrap: Bootstrap) -> AdapterResult<InstanceHandle>;

    /// Submit a prompt against the addressed `(agent_id, profile_id)`
    /// pair, spawning a new instance on first hit or reusing the live
    /// one. `input` is a structured enum (`UserTurnInput::Text` today;
    /// future `Attachment` / `Multimodal` variants slot in behind
    /// `#[non_exhaustive]`). Returns a JSON envelope the RPC / Tauri
    /// surfaces pass straight through.
    async fn submit(
        &self,
        input: UserTurnInput,
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
            resolved: ResolvedInstance,
            _bootstrap: Bootstrap,
        ) -> AdapterResult<InstanceHandle> {
            Ok(InstanceHandle {
                agent_id: resolved.agent.id.clone(),
                instance_id: resolved.agent.id,
                session_id: None,
            })
        }

        async fn submit(
            &self,
            input: UserTurnInput,
            _agent_id: Option<&str>,
            _profile_id: Option<&str>,
        ) -> AdapterResult<serde_json::Value> {
            let UserTurnInput::Text(text) = input;
            Ok(json!({ "echo": text }))
        }

        async fn cancel(&self, _agent_id: Option<&str>) -> AdapterResult<serde_json::Value> {
            Ok(json!({ "cancelled": false }))
        }

        async fn info(&self) -> AdapterResult<serde_json::Value> {
            Ok(json!({ "instances": [] }))
        }

        async fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn echo_adapter_satisfies_trait() {
        let a: Box<dyn Adapter> = Box::new(EchoAdapter);
        assert_eq!(a.id(), AdapterId::Acp);
        let v = a
            .submit(UserTurnInput::text("hi"), None, None)
            .await
            .expect("echo submit ok");
        assert_eq!(v["echo"], "hi");
        let info = a.info().await.expect("info ok");
        assert_eq!(info["instances"], json!([]));
    }

    /// Layering guard: no file outside `adapters/` may import from
    /// `crate::adapters::acp`. The rest of the crate talks to
    /// `dyn Adapter` or to the concrete types re-exported from
    /// `adapters::*` (today `AcpInstances`, `AcpAdapter`) plus the
    /// Tauri `#[command]`s at `adapters::commands`. Walks the source
    /// tree, reads every `.rs` file, and fails on any offending import.
    #[test]
    fn no_acp_imports_outside_adapters() {
        use std::fs;
        use std::path::Path;

        fn walk(dir: &Path, out: &mut Vec<String>) {
            for entry in fs::read_dir(dir).expect("read_dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    let rel = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path);
                    let rel_str = rel.to_string_lossy().to_string();
                    if rel_str.starts_with("src/adapters/") || rel_str.starts_with("src\\adapters\\") {
                        continue;
                    }
                    let body = fs::read_to_string(&path).expect("read file");
                    for (lineno, line) in body.lines().enumerate() {
                        if line.trim_start().starts_with("//") {
                            continue;
                        }
                        if line.contains("use crate::adapters::acp") || line.contains("use crate::acp") {
                            out.push(format!("{rel_str}:{}: {line}", lineno + 1));
                        }
                    }
                }
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        walk(&root, &mut offenders);
        assert!(
            offenders.is_empty(),
            "files outside adapters/ may not import from adapters::acp. Offenders:\n  {}",
            offenders.join("\n  ")
        );
    }
}
