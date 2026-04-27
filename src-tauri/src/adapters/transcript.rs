//! Generic transcript vocabulary — the typed `TranscriptItem` enum
//! the UI renders inline in the chat scroll, plus the user-turn
//! input shape and attachment helpers shared across transports.
//!
//! The wire pipeline:
//!
//! 1. Transport receives wire-format updates (ACP `SessionUpdate`,
//!    future HTTP messages, etc.).
//! 2. Transport-side mapping projects each update into a
//!    `TranscriptItem` variant (or returns `None` for non-transcript
//!    side-channel events).
//! 3. Daemon publishes `InstanceEvent::Transcript { item: TranscriptItem }`
//!    onto the registry broadcast.
//! 4. UI consumes typed `kind`-tagged shape; switch dispatches per
//!    variant.
//!
//! Forward-compat: variants the wire carries that our Rust enum
//! doesn't recognize map to `TranscriptItem::Unknown { kind, payload }`
//! at the mapping step. UI's `default` arm renders a placeholder.
//! Adding a typed variant is one entry per layer.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::permission::PermissionOptionView;

/// One entry in an instance's transcript. Covers user-side
/// (`UserPrompt`, `UserText`) and assistant-side
/// (`AgentText`, `AgentThought`, `ToolCall`, `ToolCallUpdate`,
/// `Plan`, `PermissionRequest`) items the UI renders inline.
///
/// Session-metadata updates (mode/model/title/usage) are *not*
/// transcript items — they ride on dedicated `InstanceEvent`
/// variants instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[non_exhaustive]
pub enum TranscriptItem {
    /// User's submitted prompt (text + attachments). Emitted once
    /// per user turn at submit time, daemon-authoritative — the UI
    /// no longer mirrors optimistically off the submit call.
    UserPrompt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
    },
    /// Streaming user echo from the agent (rare; some agents echo
    /// the user's prompt back). Maps from `UserMessageChunk`.
    UserText { text: String },
    /// Streaming agent reply. Maps from `AgentMessageChunk`.
    AgentText { text: String },
    /// Streaming agent reasoning. Maps from `AgentThoughtChunk`.
    AgentThought { text: String },
    /// Tool call initiated by the agent.
    ToolCall(ToolCallRecord),
    /// Delta update to an existing tool call (status, output, etc.).
    ToolCallUpdate(ToolCallUpdateRecord),
    /// Agent's execution plan.
    Plan(PlanRecord),
    /// Permission prompt for an agent action — surfaced inline so
    /// the UI can render it in-context. Same payload as
    /// `InstanceEvent::PermissionRequest`; the latter remains the
    /// authoritative bus the awaiting call site reads. UI today
    /// keeps rendering the sticky permission stack and ignores this
    /// transcript variant — flip the renderer if you want inline
    /// rendering later.
    PermissionRequest(PermissionRequestRecord),
    /// Forward-compat catch-all. Mapping emits this when the wire
    /// carries a variant our Rust enum doesn't recognize.
    /// `wireKind` carries the original discriminator string from the
    /// transport (e.g. ACP `sessionUpdate`); `payload` is the raw
    /// shape so consumers can still inspect it.
    Unknown {
        wire_kind: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    User,
    Assistant,
}

/// One tool-call as the agent first announced it. `id` ties together
/// later `ToolCallUpdate` records. `tool_kind` is the wire string
/// (`read` / `edit` / `execute` / `terminal` / etc.); the UI uses
/// it for theme + chip dispatch.
///
/// Field is named `tool_kind` (not `kind`) because TranscriptItem's
/// serde discriminator is `kind` — flattening this record into the
/// `ToolCall` variant would otherwise collide.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub id: String,
    /// Closed-set kind wire string (ACP `ToolKind`). Lower-cased.
    pub tool_kind: String,
    /// Human-readable title the agent supplied ("Read package.json").
    pub title: String,
    /// Initial state — almost always `pending` or `running`.
    pub state: ToolCallState,
    /// Optional UI-displayable args summary (`command` for Bash,
    /// `path` for fs tools, JSON otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_args: Option<String>,
    /// Initial content blocks the agent attached.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolCallContentItem>,
}

/// Delta update to an existing tool call. Each field that's `Some`
/// patches the previous record; `None` means "no change". UI
/// reduces these into the running tool-call view keyed by `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdateRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ToolCallState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_args: Option<String>,
    /// Content delta — appended to whatever the running view holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolCallContentItem>,
}

/// Lifecycle phase of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallState {
    Pending,
    Running,
    Completed,
    Failed,
}

/// One content piece attached to a tool call. Variants mirror the
/// shapes the UI actually renders; transports map their wire shapes
/// onto these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ToolCallContentItem {
    /// Text the tool emitted (stdout / log line / inline result).
    Text { text: String },
    /// File read / write payload preview.
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snippet: Option<String>,
    },
    /// Raw JSON the tool produced. Pass-through for transport-specific
    /// payloads the UI doesn't render structurally.
    Json { value: serde_json::Value },
}

/// Agent's execution plan — list of steps the agent intends to
/// take, ordered. Each entry has a content blob (markdown today)
/// and a priority hint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRecord {
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub content: String,
    /// `low` / `medium` / `high` — wire string from the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// `pending` / `in_progress` / `completed` — wire string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Permission prompt embedded in the transcript. Same fields as
/// `InstanceEvent::PermissionRequest` minus the routing scaffolding
/// (those live on the envelope, not the item).
///
/// Field is named `tool_kind` (not `kind`) for the same
/// discriminator-collision reason as `ToolCallRecord`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestRecord {
    pub request_id: String,
    pub tool: String,
    pub tool_kind: String,
    pub args: String,
    pub options: Vec<PermissionOptionView>,
}

/// User-side submit payload. Keeps the adapter's `submit` signature
/// structured (rather than a bare `&str` that can't grow with file
/// attachments / multimodal content later). `Prompt { text,
/// attachments }` is the live shape; palette-picked skills travel
/// through `attachments`.
///
/// **Wire-encoding convention** (every transport-side encoder MUST
/// honor): emit attachments first as wire-format resources carrying
/// each attachment's `body`, `file_uri()`, and `mime_type()`, **then**
/// the prose `text`. Agents read context before instructions. Each
/// transport implements its own ~5-line encoder against this
/// convention (ACP: `acp::instance::build_prompt_blocks`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[non_exhaustive]
pub enum UserTurnInput {
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
    },
}

impl UserTurnInput {
    /// Convenience for the bare-text path (no attachments).
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        Self::Prompt {
            text: s.into(),
            attachments: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_attachments(text: impl Into<String>, attachments: Vec<Attachment>) -> Self {
        Self::Prompt {
            text: text.into(),
            attachments,
        }
    }
}

/// One palette-picked skill (today) attached to a user turn. The
/// body is snapshotted at pick time so the user sees exactly what
/// they chose; re-pick to refresh after edits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// Skill slug (or any future attachment-source key). Used for
    /// dedup + UI keying.
    pub slug: String,
    /// Absolute path to the source file.
    pub path: PathBuf,
    /// Snapshot of the body at pick time. Inlined onto the
    /// transport-specific resource block so the agent reads the
    /// same thing the user did.
    pub body: String,
    /// Optional human-readable label; the UI shows it on the
    /// composer pill. Falls back to `slug` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Attachment {
    /// Wire URI for this attachment — `file://<absolute path>`.
    /// Reused by every transport's user-turn encoder so the agent
    /// can dedupe / reference the attachment by URI.
    #[must_use]
    pub fn file_uri(&self) -> String {
        format!("file://{}", self.path.display())
    }

    /// MIME type of the attachment body. Resolved from the file's
    /// path extension via the `mime_guess` crate; falls back to
    /// `application/octet-stream` for paths with no extension or an
    /// unknown extension. Every transport-side encoder uses this on
    /// the wire so the agent can dispatch attachment handling per
    /// content type.
    #[must_use]
    pub fn mime_type(&self) -> String {
        mime_guess::from_path(&self.path)
            .first_or_octet_stream()
            .essence_str()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_prompt_round_trips_kind_tag() {
        let item = TranscriptItem::UserPrompt {
            text: "hi".into(),
            attachments: vec![],
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "user_prompt");
        assert_eq!(v["text"], "hi");
        let parsed: TranscriptItem = serde_json::from_value(v).unwrap();
        match parsed {
            TranscriptItem::UserPrompt { text, .. } => assert_eq!(text, "hi"),
            other => panic!("expected UserPrompt, got {other:?}"),
        }
    }

    #[test]
    fn agent_text_round_trips() {
        let item = TranscriptItem::AgentText { text: "ok".into() };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "agent_text");
        assert_eq!(v["text"], "ok");
    }

    #[test]
    fn unknown_round_trips_with_payload() {
        let item = TranscriptItem::Unknown {
            wire_kind: "future_variant".into(),
            payload: serde_json::json!({"foo": 1}),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "unknown");
        assert_eq!(v["wireKind"], "future_variant");
        assert_eq!(v["payload"]["foo"], 1);
    }

    #[test]
    fn tool_call_record_round_trips() {
        let record = ToolCallRecord {
            id: "tc-1".into(),
            tool_kind: "read".into(),
            title: "Read package.json".into(),
            state: ToolCallState::Running,
            raw_args: Some("package.json".into()),
            content: vec![],
        };
        let item = TranscriptItem::ToolCall(record);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "tool_call");
        assert_eq!(v["id"], "tc-1");
        assert_eq!(v["toolKind"], "read");
        assert_eq!(v["state"], "running");
    }

    #[test]
    fn attachment_file_uri_includes_absolute_path() {
        let a = Attachment {
            slug: "git-commit".into(),
            path: PathBuf::from("/tmp/skills/git-commit/SKILL.md"),
            body: "stage".into(),
            title: None,
        };
        assert_eq!(a.file_uri(), "file:///tmp/skills/git-commit/SKILL.md");
        assert_eq!(a.mime_type(), "text/markdown");
    }
}
