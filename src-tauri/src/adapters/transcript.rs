//! Generic transcript vocabulary. Turn records, tool-call records,
//! user-turn input shape. The ACP `SessionUpdate` stream bridges into
//! `TranscriptItem` variants via `adapters::acp::mapping`. HTTP-based
//! adapters will ship their own mapping but hand the same enum out.

use serde::{Deserialize, Serialize};

use super::tool::{ToolCall, ToolCallContent, ToolState};

/// One entry in an instance's transcript. The UI renders a ChatTurn
/// per `Turn` + a ToolChip per `ToolCall`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
pub struct ToolCallRecord {
    pub call: ToolCall,
    pub state: ToolState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolCallContent>,
}

/// User-side submit payload. Keeps the adapter's `submit` signature
/// structured (rather than a bare `&str` that can't grow with file
/// attachments / multimodal content later). Plain `Text(String)`
/// covers every adapter today; future variants slot in behind
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UserTurnInput {
    Text(String),
}

impl UserTurnInput {
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
}
