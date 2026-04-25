//! Generic transcript vocabulary. Turn records, tool-call records,
//! user-turn input shape. The ACP `SessionUpdate` stream bridges into
//! `TranscriptItem` variants via `adapters::acp::mapping`. HTTP-based
//! adapters will ship their own mapping but hand the same enum out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::tool::{ToolCall, ToolCallContent, ToolState};

/// One entry in an instance's transcript. The UI renders a ChatTurn
/// per `Turn` + a ToolChip per `ToolCall`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TranscriptItem {
    Turn(TurnRecord),
    ToolCall(ToolCallRecord),
}

/// User or assistant utterance. `speaker` flags which side produced
/// the text; `text` is the content block concatenation. Multi-block
/// utterances collapse into one `TurnRecord` per logical turn —
/// adapters that split a turn across multiple wire notifications are
/// responsible for the reassembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    pub speaker: Speaker,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    User,
    Assistant,
}

/// One tool-call at whatever phase the adapter last reported.
/// `ToolCall` carries the identity bits (`id`, `kind`), `state` the
/// lifecycle, `content` the accumulated inputs + outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub call: ToolCall,
    pub state: ToolState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolCallContent>,
}

/// User-side submit payload. Keeps the adapter's `submit` signature
/// structured (rather than a bare `&str` that can't grow with file
/// attachments / multimodal content later). `Prompt { text,
/// attachments }` is the live shape; palette-picked skills travel
/// through `attachments` and project onto the wire as
/// `ContentBlock::Resource` per ACP mapping.
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
    /// Convenience for the bare-text path (no attachments). Existing
    /// call sites that don't yet thread attachments through funnel
    /// through this — adding palette state on top is purely additive.
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
/// they chose; re-pick to refresh after edits. Wire shape is
/// stable across `session_submit` (Tauri command) and
/// `session/submit` (RPC).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// Skill slug (or any future attachment-source key). Used for
    /// dedup + UI keying.
    pub slug: String,
    /// Absolute path to the source file. Mapped onto the agent
    /// wire as `file://<path>` inside `ContentBlock::Resource.uri`.
    pub path: PathBuf,
    /// Snapshot of the skill body at pick time. Inlined onto
    /// `ContentBlock::Resource.text` so the agent reads the same
    /// thing the user did.
    pub body: String,
    /// Optional human-readable label; the UI shows it on the
    /// composer pill. Falls back to `slug` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
