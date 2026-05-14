//! Generic actor-lifecycle vocabulary. Each adapter owns a registry
//! of live instances keyed by `InstanceKey`. An instance is our record
//! of a running agent process (or HTTP session) + its channels — it
//! outlives any single wire-session cycle and survives re-binds.
//!
//! "session" is reserved for the adapter's wire concept (e.g. the
//! ACP session id the agent issues via `session/new`); "instance" is
//! always our owner/record. See the CLAUDE.md glossary.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::permission::PermissionOptionView;
use super::transcript::TranscriptItem;
use super::AdapterError;

/// Registry key. A UUID per live instance — collisions across twin
/// profiles are impossible by construction. Wire shape is the v4
/// hyphenated string; the UI treats it as opaque.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct InstanceKey(pub Uuid);

impl InstanceKey {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a wire string (hyphenated v4). Adapters propagate the
    /// `AdapterError::InvalidRequest` to their protocol layer
    /// (`-32602` for JSON-RPC).
    pub fn parse(s: &str) -> Result<Self, AdapterError> {
        if s.is_empty() {
            return Err(AdapterError::InvalidRequest("instance id cannot be empty".into()));
        }
        s.parse::<Uuid>()
            .map(Self)
            .map_err(|e| AdapterError::InvalidRequest(format!("invalid instance_id '{s}': {e}")))
    }

    #[must_use]
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
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
///
/// Two naming axes:
/// - **Tauri event names** (colon-separated, consumed by `app.emit`):
///   `acp:instance-state`, `acp:instances-changed`, etc. The bridge
///   layer owns the mapping.
/// - **Topic strings** (dot-separated, returned by
///   [`InstanceEvent::topic`]): `instance.state`,
///   `instances.changed`. Used by tracing spans and the future
///   subscription filter layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", rename_all_fields = "camelCase")]
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
        /// Active turn id while a `session/prompt` is in flight; `None`
        /// for spontaneous updates the agent emits outside a turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        /// Typed transcript item the UI renders. Transports map their
        /// wire-format updates into this enum; unknown variants land
        /// as `TranscriptItem::Unknown` so the wire shape stays
        /// forward-compatible without bricking sessions.
        item: TranscriptItem,
        /// `_meta` envelope pass-through from the originating
        /// `session/update` notification — vendor-specific extension
        /// data that lives outside the typed protocol shapes.
        /// Observability surface today; no UI consumer this PR. Future
        /// per-vendor UI hooks plug in by reading this field without
        /// another wire change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<serde_json::Value>,
    },
    /// Permission roundtrip closed — a waiter the controller was
    /// holding has been resolved (either via UI / remote answer or
    /// the 10-min `WAITER_TIMEOUT`). Mirrors / remote subscribers
    /// drop their `pending_permissions` row keyed on `request_id` so
    /// desktop ↔ remote stays in sync when the answer landed on the
    /// other transport.
    ///
    /// `option_id == "__expired__"` is the sentinel for the timeout
    /// path; `forget` doesn't carry an option, but the wire shape
    /// stays uniform so consumers don't branch on a separate variant.
    /// Captains shouldn't ever see this value in the UI — by the
    /// time it lands the row has already been removed from the
    /// snapshot.
    PermissionResolved {
        instance_id: String,
        request_id: String,
        option_id: String,
    },
    PermissionRequest {
        agent_id: String,
        instance_id: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        request_id: String,
        tool: String,
        kind: String,
        args: String,
        /// Raw `tool_call.rawInput` JSON (pass-through). Populated when
        /// the agent supplied one — bash gets `{ command }`, file ops
        /// `{ path }`, claude-code `ExitPlanMode` carries `{ plan }`.
        /// UI consumers extract structured fields here instead of
        /// re-parsing the collapsed `args` summary.
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "rawInput")]
        raw_input: Option<serde_json::Value>,
        /// Raw `tool_call.content[]` blocks (pass-through of the ACP
        /// wire shape — `{ type: 'content' | 'diff' | 'terminal', … }`).
        /// Some agents (claude-code's `Switch mode`) ship the markdown
        /// body here instead of on `raw_input`; the UI walks the array
        /// directly to render text / diff / terminal blocks without
        /// server-side joining.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<serde_json::Value>,
        options: Vec<PermissionOptionView>,
        /// Daemon-authored presentation view, formatted from `tool` +
        /// `kind` + `raw_input` + `content` via the formatter
        /// registry. The UI renders this verbatim on the permission
        /// row / modal — no client-side formatting fallback.
        formatted: crate::tools::formatter::types::FormattedToolCall,
    },
    /// A `session/prompt` request was accepted by the actor — the
    /// frontend uses `turn_id` to group every subsequent `Transcript`
    /// / `PermissionRequest` until the matching `TurnEnded` lands.
    TurnStarted {
        agent_id: String,
        instance_id: String,
        session_id: String,
        turn_id: String,
        /// Wall-clock (epoch ms) when the actor accepted the prompt.
        /// Pairs with `TurnEnded.ended_at` so the UI can render a
        /// total-elapsed chip on the Turn footer.
        started_at: u64,
    },
    /// The active `session/prompt` resolved (or errored). `stop_reason`
    /// mirrors the ACP `StopReason` wire string when the response was
    /// successful; `None` when the request errored / was cancelled.
    /// `error` carries the ACP / transport error message so the UI
    /// can surface it as a toast — without this any mid-prompt
    /// failure (rate limit, agent crash, transport hiccup) is
    /// invisible to the user.
    TurnEnded {
        agent_id: String,
        instance_id: String,
        session_id: String,
        turn_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Wall-clock (epoch ms) when the prompt resolved (success or
        /// error). UI subtracts `TurnStarted.started_at` to render
        /// the total-elapsed chip.
        ended_at: u64,
    },
    /// Registry membership changed — an instance spawned, shut down,
    /// or restarted. `instance_ids` is the full post-change set;
    /// `focused_id` echoes the current focus so consumers reconcile
    /// both bits in a single event.
    InstancesChanged {
        instance_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        focused_id: Option<String>,
    },
    /// Focus pointer moved. `instance_id` is `None` when the registry
    /// emptied and no auto-focus target exists.
    InstancesFocused {
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
    },
    /// Captain renamed an instance. `name` is the post-change value
    /// (`None` when the captain cleared the name). UI listens to keep
    /// row labels in sync without re-fetching the full instance list.
    InstanceRenamed {
        instance_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Terminal output / exit chunk. Pushed live as the agent's child
    /// process emits stdout / stderr; the UI accumulates these into a
    /// per-`terminal_id` scrollable card without polling
    /// `terminal/output`.
    Terminal {
        agent_id: String,
        instance_id: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        terminal_id: String,
        chunk: TerminalChunk,
    },
    /// Daemon-wide reload (`daemon/reload`) — config + skills + MCPs
    /// rescanned. Carries post-reload counts so subscribers can refresh
    /// their caches without a separate roundtrip.
    DaemonReloaded {
        profiles: usize,
        skills_count: usize,
        mcps_count: usize,
    },
    /// ACP `SessionInfoUpdate` notification — title / updatedAt only,
    /// per the schema. Carried as its own `InstanceEvent` rather than
    /// a transcript item because session metadata isn't transcript
    /// content. `title` and `updated_at` follow the wire shape: each
    /// is `Some(Some(s))` for a set value, `Some(None)` to explicitly
    /// clear, `None` to leave the previous value untouched.
    SessionInfoUpdate {
        agent_id: String,
        instance_id: String,
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        updated_at: Option<String>,
    },
    /// ACP `CurrentModeUpdate` notification — `current_mode_id` only.
    CurrentModeUpdate {
        agent_id: String,
        instance_id: String,
        session_id: String,
        current_mode_id: String,
    },
    /// Per-session usage telemetry — context budget + cost. Emitted
    /// on every `usage_update` notification claude-agent-acp pushes
    /// during a turn. UI attaches the latest reading to the active
    /// turn so the captain sees live spend + window utilisation.
    UsageUpdate {
        agent_id: String,
        instance_id: String,
        session_id: String,
        /// `turn_id` of the live turn at notification time, when
        /// one exists; absent for between-turn updates.
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        used: u64,
        size: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<crate::adapters::acp::instance::UsageCost>,
    },
    /// Adapter-advertised session config option categories. Generic
    /// vocabulary covering `mode` (spec-reserved), `model` (spec-
    /// reserved), and vendor extensions (`effort` adaptive-thinking
    /// for claude-agent-acp 0.21+, etc.). UI maps each category to a
    /// palette leaf so captains pick values without memorising the
    /// vendor vocab.
    ConfigOptionsUpdate {
        agent_id: String,
        instance_id: String,
        session_id: String,
        categories: Vec<SessionConfigOptionCategory>,
    },
    /// Daemon-side per-instance metadata refresh — NOT an ACP wire
    /// notification. The daemon emits this after `session/new` resolves
    /// (so the UI gets the resolved cwd + advertised modes/models),
    /// after every `session/prompt` resolution (turn-end refresh), and
    /// after a restart that swaps the cwd. claude-agent-acp doesn't
    /// emit `SessionInfoUpdate` or `CurrentModeUpdate` proactively, so
    /// without this push the header chrome would never see cwd / mode
    /// / model values.
    InstanceMeta {
        agent_id: String,
        instance_id: String,
        /// Spawning profile id, when one resolved during ensure
        /// (`Some`); `None` for bare-agent spawns. Carried on every
        /// InstanceMeta refresh so the header chrome can render the
        /// focused instance's profile pill — distinct from the user's
        /// persisted profile picker (which only changes on explicit
        /// selection, not on focus shifts).
        #[serde(skip_serializing_if = "Option::is_none")]
        profile_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        cwd: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_mode_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_model_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        available_modes: Vec<SessionModeInfo>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        available_models: Vec<SessionModelInfo>,
        /// Number of MCP servers wired to this instance — drives the
        /// header `+N mcps` pill. Computed from
        /// `effective_mcp_files_for(profile)` at spawn time and
        /// included in every InstanceMeta refresh so the UI never has
        /// to derive it independently.
        #[serde(default)]
        mcps_count: usize,
    },
    /// Fired once per instance start when the configured system
    /// prompt is actually being attached to the spawn. Skipped
    /// when `system_prompt.inject.on_create=false` (Bootstrap::Fresh)
    /// or `inject.on_restore=false` (Bootstrap::Resume). Drives the
    /// chat-side "system prompt attached" banner. `files` is the
    /// resolved path list so the UI can show the captain WHAT
    /// landed.
    SystemPromptInjected {
        agent_id: String,
        instance_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    },
}

/// Display-friendly snapshot of one ACP `SessionMode`. Mirrors
/// `agent_client_protocol::schema::SessionMode` but drops the
/// `_meta` field — the UI doesn't surface vendor-specific metadata
/// today.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Display-friendly snapshot of one ACP `ModelInfo` advertised by
/// `NewSessionResponse.models` (gated by the unstable
/// `session_model` feature). Same shape as `SessionModeInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One advertised value for a session-level config option (e.g. one
/// of `low | medium | high | xhigh | max` for `effort`). Mirrors
/// claude-agent-acp's `configOptions[].options[]` shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigOptionValue {
    /// Wire value the UI sends back via `set_session_config_option`.
    pub value: String,
    /// Display label.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One advertised category — mirrors claude-agent-acp's
/// `configOptions[].category`. Covers the spec-reserved `mode` /
/// `model` axes AND vendor extensions (`effort` for adaptive
/// thinking, future per-vendor toggles). UI builds a palette leaf
/// per category so the captain picks values without memorising the
/// vendor vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigOptionCategory {
    /// Wire id used both for `set_session_config_option` and as the
    /// palette leaf identifier.
    pub id: String,
    /// Display label for the category (e.g. "Effort").
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Currently-selected value, if any. UI marks the matching
    /// option as the active row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_value: Option<String>,
    /// Closed set of valid values. Empty when the category is a
    /// free-form text input — UI degrades to the modes-style picker
    /// when populated, hides the leaf when empty.
    #[serde(default)]
    pub options: Vec<SessionConfigOptionValue>,
}

/// Per-terminal payload variant. `Output` carries stdout / stderr
/// bytes as the child process emits them; `Exit` lands once on exit
/// with the resolved status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TerminalChunk {
    Output {
        stream: TerminalStream,
        data: String,
    },
    Exit {
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        signal: Option<String>,
    },
}

/// Which standard stream a terminal `Output` chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStream {
    Stdout,
    Stderr,
}

impl InstanceEvent {
    /// Dot-separated topic name. Stable contract for subscription
    /// filtering. Colon-separated Tauri event names live only in the
    /// bridge's mapping table.
    #[must_use]
    pub fn topic(&self) -> &'static str {
        match self {
            InstanceEvent::State { .. } => "instance.state",
            InstanceEvent::Transcript { .. } => "instance.transcript",
            InstanceEvent::PermissionRequest { .. } => "instance.permission_request",
            InstanceEvent::PermissionResolved { .. } => "instance.permission_resolved",
            InstanceEvent::TurnStarted { .. } => "instance.turn_started",
            InstanceEvent::TurnEnded { .. } => "instance.turn_ended",
            InstanceEvent::InstancesChanged { .. } => "instances.changed",
            InstanceEvent::InstancesFocused { .. } => "instances.focused",
            InstanceEvent::InstanceRenamed { .. } => "instance.renamed",
            InstanceEvent::Terminal { .. } => "terminal.output",
            InstanceEvent::DaemonReloaded { .. } => "daemon.reloaded",
            InstanceEvent::SessionInfoUpdate { .. } => "instance.session_info_update",
            InstanceEvent::CurrentModeUpdate { .. } => "instance.current_mode_update",
            InstanceEvent::UsageUpdate { .. } => "instance.usage_update",
            InstanceEvent::ConfigOptionsUpdate { .. } => "instance.config_options_update",
            InstanceEvent::InstanceMeta { .. } => "instance.meta",
            InstanceEvent::SystemPromptInjected { .. } => "instance.system_prompt_injected",
        }
    }

    /// Colon-separated public event name — the same string the Tauri
    /// webview listens on (`acp:transcript`, `acp:turn-started`, etc.)
    /// and that the WS remote bridge + the unix-socket `events/subscribe`
    /// handler both broadcast. Single source of truth across transports
    /// so a Neovim plugin, a remote SPA, and the desktop webview all
    /// subscribe to the same names.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        match self {
            InstanceEvent::State { .. } => "acp:instance-state",
            InstanceEvent::Transcript { .. } => "acp:transcript",
            InstanceEvent::PermissionRequest { .. } => "acp:permission-request",
            InstanceEvent::PermissionResolved { .. } => "acp:permission-resolved",
            InstanceEvent::TurnStarted { .. } => "acp:turn-started",
            InstanceEvent::TurnEnded { .. } => "acp:turn-ended",
            InstanceEvent::InstancesChanged { .. } => "acp:instances-changed",
            InstanceEvent::InstancesFocused { .. } => "acp:instances-focused",
            InstanceEvent::InstanceRenamed { .. } => "acp:instance-renamed",
            InstanceEvent::Terminal { .. } => "acp:terminal",
            InstanceEvent::DaemonReloaded { .. } => "daemon:reloaded",
            InstanceEvent::SessionInfoUpdate { .. } => "acp:session-info-update",
            InstanceEvent::CurrentModeUpdate { .. } => "acp:current-mode-update",
            InstanceEvent::UsageUpdate { .. } => "acp:usage-update",
            InstanceEvent::ConfigOptionsUpdate { .. } => "acp:config-options-update",
            InstanceEvent::InstanceMeta { .. } => "acp:instance-meta",
            InstanceEvent::SystemPromptInjected { .. } => "acp:system-prompt-injected",
        }
    }

    /// Optional `instance_id` for the event — `None` for daemon-global
    /// events (`DaemonReloaded`). Powers per-instance subscription
    /// filters: a Neovim buffer pinned to instance X subscribes with
    /// `{instance_id: "..."}` and gets only events tagged with that id.
    #[must_use]
    pub fn instance_id(&self) -> Option<&str> {
        match self {
            InstanceEvent::State { instance_id, .. }
            | InstanceEvent::Transcript { instance_id, .. }
            | InstanceEvent::PermissionRequest { instance_id, .. }
            | InstanceEvent::PermissionResolved { instance_id, .. }
            | InstanceEvent::TurnStarted { instance_id, .. }
            | InstanceEvent::TurnEnded { instance_id, .. }
            | InstanceEvent::InstanceRenamed { instance_id, .. }
            | InstanceEvent::Terminal { instance_id, .. }
            | InstanceEvent::SessionInfoUpdate { instance_id, .. }
            | InstanceEvent::CurrentModeUpdate { instance_id, .. }
            | InstanceEvent::UsageUpdate { instance_id, .. }
            | InstanceEvent::ConfigOptionsUpdate { instance_id, .. }
            | InstanceEvent::InstanceMeta { instance_id, .. }
            | InstanceEvent::SystemPromptInjected { instance_id, .. } => Some(instance_id),
            InstanceEvent::InstancesChanged { .. }
            | InstanceEvent::InstancesFocused { .. }
            | InstanceEvent::DaemonReloaded { .. } => None,
        }
    }
}

/// Flat snapshot of one live instance. Adapters surface the same
/// shape regardless of transport — consumers (RPC / UI pickers)
/// render off this.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub id: String,
    /// Captain-set addressable name. `None` until `instances/rename`
    /// sets it. Distinct from `id` (canonical UUID): scripts that
    /// captured the UUID stay valid even after rename. Validated as
    /// a slug at the rename boundary; safe to display verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Per-instance operational mode (e.g. `plan` / `edit` for
    /// claude-code). `Some` when the spawning profile set one;
    /// `None` otherwise. Adapters interpret it at spawn time — the
    /// generic layer only carries + surfaces the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Wire shape for instance entries on `instances/list` and
/// `boot_snapshot`. Renames `id → instanceId` and carries
/// `skip_serializing_if = Option::is_none` on every optional so the
/// JSON omits the key entirely instead of emitting `null` — UI
/// consumers branch on `entry.foo !== undefined`, and `null !==
/// undefined` slipped through to runtime crashes (`null.length`)
/// before this typed shape replaced the hand-rolled `json!` macro
/// at both call sites.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceListEntry {
    pub instance_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl From<&InstanceInfo> for InstanceListEntry {
    fn from(i: &InstanceInfo) -> Self {
        Self {
            instance_id: i.id.clone(),
            agent_id: i.agent_id.clone(),
            profile_id: i.profile_id.clone(),
            session_id: i.session_id.clone(),
            name: i.name.clone(),
            mode: i.mode.clone(),
        }
    }
}

/// Wire shape for `agents_list` entries. Mirrors `InstanceListEntry`
/// in spirit — typed struct over hand-rolled `json!()` so the UI
/// can't drift on a renamed field, and `skip_serializing_if` keeps
/// optional fields off the wire (preserves the no-null invariant).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub id: String,
    pub provider: String,
    pub binding: String,
    pub is_default: bool,
}

/// Wire shape for `profiles_list` / `config/profiles` entries.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub is_default: bool,
}

/// Validate a captain-supplied instance name against the slug rule.
///
/// Rule: `[a-z0-9][a-z0-9_-]*`, lowercase, ≤16 chars. Lowercase /
/// slug-only because anything else needs shell quoting on the ctl
/// side, and the 16-char ceiling keeps log lines tidy without
/// being so tight that meaningful names get truncated. Returns
/// the validated owned `String` for ergonomics; the caller can
/// move it into the registry without re-allocating.
pub fn validate_instance_name(raw: &str) -> Result<String, AdapterError> {
    if raw.is_empty() {
        return Err(AdapterError::InvalidRequest("instance name must not be empty".into()));
    }
    if raw.len() > 16 {
        return Err(AdapterError::InvalidRequest(format!(
            "instance name '{raw}' exceeds 16-char limit"
        )));
    }
    let mut chars = raw.chars();
    let first = chars.next().expect("non-empty checked above");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(AdapterError::InvalidRequest(format!(
            "instance name '{raw}' must start with a lowercase letter or digit"
        )));
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            return Err(AdapterError::InvalidRequest(format!(
                "instance name '{raw}' contains illegal character '{c}' (lowercase a-z, 0-9, '-', '_' only)"
            )));
        }
    }
    Ok(raw.to_string())
}

/// Stream typealias — `broadcast::Receiver` per subscriber. One
/// instance of events multiplexed across the daemon's consumers
/// (Tauri bridge, tests, future UI pickers).
///
/// Every consumer MUST handle `broadcast::error::RecvError::Lagged`
/// explicitly — the broadcast channel silently drops messages to the
/// lagging subscriber otherwise.
pub type InstanceEventStream = broadcast::Receiver<InstanceEvent>;

/// Per-actor contract the generic registry needs from each adapter's
/// handle type. ACP's `AcpInstance` implements this; a future
/// `HttpInstance` would too.
#[async_trait]
pub trait InstanceActor: Send + Sync + 'static {
    /// Identity + metadata snapshot for `list` / `info_for`.
    fn info(&self) -> InstanceInfo;

    /// Captain-set name accessor. Distinct from `info().id`: the id
    /// is the canonical never-shifting key; the name is mutable.
    /// Async because the implementor's storage is `RwLock`-protected
    /// (see `AcpInstance::current_name`).
    async fn name(&self) -> Option<String>;

    /// Overwrite the captain-set name. The registry has already
    /// validated (slug rule + uniqueness); this is a raw write.
    async fn set_name(&self, name: Option<String>);

    /// Drain the actor. Best-effort — the registry timeouts on the
    /// ack so a wedged actor can't block shutdown.
    async fn shutdown(&self);
}

/// Parameter bundle for `Adapter::spawn`. Every field is optional;
/// adapters fall through to their own default-chain when a slot is
/// `None`. Constructed at the RPC / Tauri command boundary from user
/// input; the adapter then resolves it against its config.
///
/// `config_patches` carries kustomize-style overlay documents (one
/// per `--with-config` flag or `withConfig` array entry) that the
/// adapter folds onto its base `Config` before constructing the
/// instance. Patches are stored on the running instance so
/// `Adapter::restart` replays them against whatever config the
/// daemon currently has.
#[derive(Debug, Default, Clone)]
pub struct SpawnSpec {
    pub profile_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub config_patches: Vec<serde_json::Value>,
}

#[cfg(test)]
mod instance_list_entry_tests {
    use super::*;

    /// Pin the wire shape — `instances/list` and `boot_snapshot`
    /// both ship `InstanceListEntry`, and the UI consumer branches
    /// on `entry.foo !== undefined`. Earlier hand-rolled `json!`
    /// macros bypassed `skip_serializing_if` and emitted JSON `null`
    /// for unset optionals, which slipped past the UI guard and
    /// crashed at `null.length` (taking the boot pipeline with it).
    /// This test asserts the typed serialization omits unset keys.
    #[test]
    fn unset_optionals_are_omitted_not_null() {
        let entry = InstanceListEntry {
            instance_id: "abc-123".into(),
            agent_id: "claude-code".into(),
            profile_id: None,
            session_id: None,
            name: None,
            mode: None,
        };
        let value = serde_json::to_value(&entry).expect("serialize");
        let obj = value.as_object().expect("entry is JSON object");
        assert_eq!(
            obj.get("instanceId"),
            Some(&serde_json::Value::String("abc-123".into()))
        );
        assert_eq!(
            obj.get("agentId"),
            Some(&serde_json::Value::String("claude-code".into()))
        );
        // The whole point: unset Option<String> is OMITTED, never null.
        assert!(
            !obj.contains_key("profileId"),
            "profileId must be omitted, got {:?}",
            obj.get("profileId")
        );
        assert!(
            !obj.contains_key("sessionId"),
            "sessionId must be omitted, got {:?}",
            obj.get("sessionId")
        );
        assert!(
            !obj.contains_key("name"),
            "name must be omitted, got {:?}",
            obj.get("name")
        );
        assert!(
            !obj.contains_key("mode"),
            "mode must be omitted, got {:?}",
            obj.get("mode")
        );
    }

    #[test]
    fn set_optionals_round_trip() {
        let entry = InstanceListEntry {
            instance_id: "x".into(),
            agent_id: "claude-code".into(),
            profile_id: Some("personal/claude/opus".into()),
            session_id: Some("sess-1".into()),
            name: Some("alpha".into()),
            mode: Some("plan".into()),
        };
        let value = serde_json::to_value(&entry).expect("serialize");
        assert_eq!(value["profileId"], "personal/claude/opus");
        assert_eq!(value["sessionId"], "sess-1");
        assert_eq!(value["name"], "alpha");
        assert_eq!(value["mode"], "plan");
    }

    #[test]
    fn from_instance_info_renames_id_to_instance_id() {
        let info = InstanceInfo {
            id: "abc-123".into(),
            name: None,
            agent_id: "claude-code".into(),
            profile_id: None,
            session_id: None,
            mode: None,
        };
        let entry = InstanceListEntry::from(&info);
        assert_eq!(entry.instance_id, "abc-123");
        assert_eq!(entry.agent_id, "claude-code");
    }

    /// `event_name` is the public colon-separated string that every
    /// frontend (Tauri webview, WS bridge, unix-socket events handler)
    /// listens on. Single source of truth — this test pins the
    /// canonical names so a change to a string ripples through one
    /// site, not three.
    #[test]
    fn event_name_returns_canonical_strings() {
        let cases: &[(InstanceEvent, &str)] = &[
            (
                InstanceEvent::DaemonReloaded {
                    profiles: 0,
                    skills_count: 0,
                    mcps_count: 0,
                },
                "daemon:reloaded",
            ),
            (
                InstanceEvent::InstancesChanged {
                    instance_ids: Vec::new(),
                    focused_id: None,
                },
                "acp:instances-changed",
            ),
            (
                InstanceEvent::InstancesFocused { instance_id: None },
                "acp:instances-focused",
            ),
        ];
        for (evt, want) in cases {
            assert_eq!(evt.event_name(), *want);
        }
    }

    /// `instance_id` returns `None` for daemon-global events and
    /// `Some(id)` for per-instance events. Powers the per-connection
    /// filter in `events/subscribe`.
    #[test]
    fn instance_id_distinguishes_global_from_per_instance() {
        let global = InstanceEvent::DaemonReloaded {
            profiles: 0,
            skills_count: 0,
            mcps_count: 0,
        };
        assert_eq!(global.instance_id(), None);

        let scoped = InstanceEvent::InstanceRenamed {
            instance_id: "abc-123".into(),
            name: None,
        };
        assert_eq!(scoped.instance_id(), Some("abc-123"));
    }
}
