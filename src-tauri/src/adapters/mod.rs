//! Transport-agnostic adapter layer.
//!
//! An `Adapter` is a way to talk to an agent — today ACP subprocesses,
//! tomorrow HTTP-based vendor APIs. This module owns the generic
//! vocabulary every adapter impl emits: instance handles, transcript
//! items, permission prompts, profile config. Transport-specific
//! mapping into `TranscriptItem` lives inside the transport module
//! (e.g. `acp::instance::map_session_update`).
//!
//! The split: `rpc::` / `ctl::` / `daemon::` / `config::` only ever
//! import from `adapters::*`. They do not reach into `adapters::acp::*`
//! — that's a layering violation, caught by the lint guard.

// Speculative trait expansion: most wire-method dispatchers default to
// `AdapterError::Unsupported` and only the AcpAdapter overrides the
// methods it supports today. Until handlers fully migrate to dispatch
// through `dyn Adapter` (and a sibling adapter lands), several trait
// methods register as dead.
#![allow(dead_code)]

pub mod acp;
pub mod commands;
pub mod instance;
pub mod permission;
pub mod profile;
pub mod registry;
pub mod transcript;

use async_trait::async_trait;

#[allow(unused_imports)]
pub use instance::{
    InstanceActor, InstanceEvent, InstanceEventStream, InstanceHandle, InstanceInfo, InstanceKey, InstanceState,
    SessionModeInfo, SessionModelInfo, SpawnSpec, TerminalChunk, TerminalStream,
};
#[allow(unused_imports)]
pub use permission::{
    pick_allow_option_id, pick_reject_option_id, Decision, DefaultPermissionController, PermissionController,
    PermissionOptionView, PermissionOutcome, PermissionPrompt, PermissionReply, PermissionRequest,
    PermissionRequestSnapshot, ToolCallRef,
};
#[allow(unused_imports)]
pub use profile::{AgentConfig, AgentProvider, ProfileConfig, ResolvedInstance};
#[allow(unused_imports)]
pub use registry::AdapterRegistry;
#[allow(unused_imports)]
pub use transcript::{
    Attachment, PermissionRequestRecord, PlanRecord, PlanStep, Speaker, ToolCallContentItem, ToolCallRecord,
    ToolCallState, ToolCallUpdateRecord, TranscriptItem, UserTurnInput,
};

#[allow(unused_imports)]
pub use acp::AcpAdapter;

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

/// Static capability bits each agent advertises. Used by UI pickers to
/// gate features against agents that don't advertise the underlying
/// hook (e.g. "resume session" is greyed out for an agent whose
/// `load_session` is `false`). Bool rather than `Option<…>` because
/// every field is a yes/no feature flag.
///
/// Capabilities are static per-agent (declared on the `AcpAgent` trait
/// in the ACP layer; future HTTP impls expose their own per-agent
/// declarations). Vendors are version-pinned through the package
/// manager — hyprpilot's static cap declaration tracks the pinned
/// version. Vendor lies surface as `AdapterError::Backend`, not as
/// runtime capability negotiation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// `session/load` — resume a persisted session.
    pub load_session: bool,
    /// `session/list` — enumerate persisted sessions.
    pub list_sessions: bool,
    /// `request_permission` flow + `permissions/*` RPC surface.
    pub permissions: bool,
    /// `terminal/*` tool requests (ACP terminal extension).
    pub terminals: bool,
    /// `models/set` — switch the active model on a live session.
    pub session_model_switch: bool,
    /// `modes/set` — switch the active operational mode on a live session.
    pub session_mode_switch: bool,
    /// `mcps/set` per-instance MCP enabled-list overrides.
    pub mcps_per_instance: bool,
    /// `commands/list` — slash-command catalogue cache.
    pub list_commands: bool,
    /// `instances/restart` accepts a `cwd` overlay (palette cwd swap).
    pub restart_with_cwd: bool,
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
///
/// Generic instance-lifecycle methods (`list` / `focused_id` / `focus`
/// / `shutdown_one` / `restart` / `info_for` / `subscribe`) forward
/// to each adapter's `AdapterRegistry<H>`; the transport-specific
/// methods (`submit` / `cancel` / `spawn` / `shutdown`) stay on the
/// impl. Adding an adapter means one new `impl Adapter`; the generic
/// methods stay one-line delegations.
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    fn id(&self) -> AdapterId;

    /// Per-agent static capability set. UI gates buttons on this
    /// lookup; daemon enforces by checking caps before dispatching
    /// gated trait methods.
    async fn capabilities_for_agent(&self, agent_id: &str) -> AdapterResult<Capabilities>;

    // ── generic registry ops ──────────────────────────────────────────
    async fn list(&self) -> Vec<InstanceInfo>;

    async fn info_for(&self, key: InstanceKey) -> AdapterResult<InstanceInfo>;

    async fn focused_id(&self) -> Option<InstanceKey>;

    async fn focus(&self, key: InstanceKey) -> AdapterResult<InstanceKey>;

    async fn shutdown_one(&self, key: InstanceKey) -> AdapterResult<InstanceKey>;

    /// Graceful drop + respawn preserving the insertion-order slot.
    /// Preserves `InstanceKey` (the UUID) too — callers subscribed to
    /// a specific key stay bound across the swap. Optional `cwd`
    /// overlays the resolved agent cwd before the new actor spawns —
    /// load-bearing for the K-266 cwd palette.
    async fn restart(&self, key: InstanceKey, cwd: Option<std::path::PathBuf>) -> AdapterResult<InstanceKey>;

    fn subscribe(&self) -> InstanceEventStream;

    // ── transport-specific ops ────────────────────────────────────────

    /// Spawn a new instance per the spec. Empty-registry → auto-focus
    /// the new instance (inside the registry). Returns the minted
    /// `InstanceKey`.
    async fn spawn(&self, spec: SpawnSpec) -> AdapterResult<InstanceKey>;

    /// Submit a prompt against an existing instance (when
    /// `instance_id` is provided) or spawn one for the resolved
    /// `(agent, profile)` pair. `input` is a structured enum
    /// (`UserTurnInput::Prompt { text, attachments }` today);
    /// returns a JSON envelope RPC + Tauri pass through verbatim.
    async fn submit(
        &self,
        input: UserTurnInput,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> AdapterResult<serde_json::Value>;

    /// Cancel the active turn on the addressed instance (by UUID), or
    /// fall back to the first live instance of `agent_id`.
    async fn cancel(&self, instance_id: Option<&str>, agent_id: Option<&str>) -> AdapterResult<serde_json::Value>;

    /// Snapshot of every live instance as a wire value. Keeps the
    /// legacy `session/info` shape; UI code reads [`Adapter::list`]
    /// for typed consumption.
    async fn info(&self) -> AdapterResult<serde_json::Value>;

    /// Drain every live instance. Called from `daemon::shutdown`
    /// before `app.exit(0)`.
    async fn shutdown(&self);

    /// Permission controller for prompts that need user approval.
    /// Default: `None` (transports that don't broker permissions).
    /// `AcpAdapter` returns `Some(...)` so the `permissions/*` RPC
    /// handlers can list / resolve pending prompts via the trait.
    fn permissions(&self) -> Option<std::sync::Arc<dyn crate::adapters::permission::PermissionController>> {
        None
    }

    // ── wire-method dispatch surface (S3 expansion) ───────────────────
    //
    // Each method has a `Capabilities`-gated default that returns
    // `AdapterError::Unsupported`. Concrete adapters override only the
    // methods their capability bit advertises true. A future HTTP
    // adapter slots in clean — implement what you support, leave the
    // rest at the default. Callers (RPC + Tauri handlers) dispatch
    // through `dyn Adapter`; capability-gating happens here so wire
    // surfaces stay symmetric across adapters.

    /// `agents/list` — every configured agent + its static
    /// `Capabilities`. Default returns an empty list (adapters with
    /// no agent registry).
    async fn list_agents(&self) -> AdapterResult<Vec<serde_json::Value>> {
        Ok(Vec::new())
    }

    /// `profiles/list` — every configured profile. Default returns
    /// an empty list.
    async fn list_profiles(&self) -> AdapterResult<Vec<serde_json::Value>> {
        Ok(Vec::new())
    }

    /// `session/list` — persisted session index for the addressed
    /// agent. Gated by `Capabilities::list_sessions`.
    async fn list_sessions(
        &self,
        _instance_id: Option<&str>,
        _agent_id: Option<&str>,
        _profile_id: Option<&str>,
        _cwd: Option<std::path::PathBuf>,
    ) -> AdapterResult<serde_json::Value> {
        Err(AdapterError::Unsupported(
            "session/list not supported by this adapter".into(),
        ))
    }

    /// `session/load` — resume a persisted session. Gated by
    /// `Capabilities::load_session`.
    async fn load_session(
        &self,
        _instance_id: Option<&str>,
        _agent_id: Option<&str>,
        _profile_id: Option<&str>,
        _session_id: String,
    ) -> AdapterResult<()> {
        Err(AdapterError::Unsupported(
            "session/load not supported by this adapter".into(),
        ))
    }

    /// `commands/list` — slash-command catalogue for the addressed
    /// instance. Gated by `Capabilities::list_commands`.
    async fn list_commands(&self, _instance_id: &str) -> AdapterResult<Vec<serde_json::Value>> {
        Err(AdapterError::Unsupported(
            "commands/list not supported by this adapter".into(),
        ))
    }

    /// `models/set` — switch active model on a live session. Gated by
    /// `Capabilities::session_model_switch`.
    async fn set_session_model(&self, _instance_id: &str, _model_id: &str) -> AdapterResult<serde_json::Value> {
        Err(AdapterError::Unsupported(
            "models/set not supported by this adapter".into(),
        ))
    }

    /// `modes/set` — switch active operational mode on a live session.
    /// Gated by `Capabilities::session_mode_switch`.
    async fn set_session_mode(&self, _instance_id: &str, _mode_id: &str) -> AdapterResult<serde_json::Value> {
        Err(AdapterError::Unsupported(
            "modes/set not supported by this adapter".into(),
        ))
    }

    /// `mcps/set` — install per-instance MCP enabled-list override.
    /// Gated by `Capabilities::mcps_per_instance`. Returns the previous
    /// override if any.
    async fn set_mcps_override(&self, _key: InstanceKey, _enabled: Vec<String>) -> AdapterResult<Option<Vec<String>>> {
        Err(AdapterError::Unsupported(
            "mcps/set not supported by this adapter".into(),
        ))
    }

    /// Effective MCP enabled-list for an instance. Per-instance
    /// override wins; otherwise the resolved profile's `mcps` field;
    /// otherwise `None` ("all enabled" semantics). Gated by
    /// `Capabilities::mcps_per_instance`.
    async fn mcps_list_for(&self, _key: InstanceKey) -> AdapterResult<Option<Vec<String>>> {
        Err(AdapterError::Unsupported(
            "mcps/list not supported by this adapter".into(),
        ))
    }

    /// Snapshot of every instance id currently mid-turn. Default
    /// returns an empty list — adapters without busy-tracking claim
    /// "nothing busy" so `daemon/shutdown` can proceed without a
    /// false-positive wedge.
    async fn busy_instance_ids(&self) -> Vec<String> {
        Vec::new()
    }

    /// Publish a `DaemonReloaded` event onto the adapter's event
    /// broadcast. Adapters with no registry no-op silently.
    fn publish_daemon_reloaded(&self, _profiles: usize, _skills_count: usize, _mcps_count: usize) {}
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

        async fn capabilities_for_agent(&self, _agent_id: &str) -> AdapterResult<Capabilities> {
            Ok(Capabilities::default())
        }

        async fn list(&self) -> Vec<InstanceInfo> {
            Vec::new()
        }

        async fn info_for(&self, key: InstanceKey) -> AdapterResult<InstanceInfo> {
            Err(AdapterError::InvalidRequest(format!("no instances (asked for {key})")))
        }

        async fn focused_id(&self) -> Option<InstanceKey> {
            None
        }

        async fn focus(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
            Err(AdapterError::InvalidRequest(format!("echo has no instance {key}")))
        }

        async fn shutdown_one(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
            Err(AdapterError::InvalidRequest(format!("echo has no instance {key}")))
        }

        async fn restart(&self, key: InstanceKey, _cwd: Option<std::path::PathBuf>) -> AdapterResult<InstanceKey> {
            Err(AdapterError::InvalidRequest(format!("echo has no instance {key}")))
        }

        fn subscribe(&self) -> InstanceEventStream {
            let (tx, rx) = tokio::sync::broadcast::channel(1);
            drop(tx);
            rx
        }

        async fn spawn(&self, _spec: SpawnSpec) -> AdapterResult<InstanceKey> {
            Err(AdapterError::Unsupported("echo cannot spawn".into()))
        }

        async fn submit(
            &self,
            input: UserTurnInput,
            _instance_id: Option<&str>,
            _agent_id: Option<&str>,
            _profile_id: Option<&str>,
        ) -> AdapterResult<serde_json::Value> {
            let UserTurnInput::Prompt { text, attachments } = input;
            Ok(json!({ "echo": text, "attachments": attachments.len() }))
        }

        async fn cancel(
            &self,
            _instance_id: Option<&str>,
            _agent_id: Option<&str>,
        ) -> AdapterResult<serde_json::Value> {
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
            .submit(UserTurnInput::text("hi"), None, None, None)
            .await
            .expect("echo submit ok");
        assert_eq!(v["echo"], "hi");
        let info = a.info().await.expect("info ok");
        assert_eq!(info["instances"], json!([]));
    }

    /// Layering guard: no file outside `adapters/` may import from
    /// `crate::adapters::acp`. The rest of the crate talks to
    /// `dyn Adapter` or to the concrete types re-exported from
    /// `adapters::*` (today `AcpAdapter`) plus the Tauri
    /// `#[command]`s at `adapters::commands`. Walks the source
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
