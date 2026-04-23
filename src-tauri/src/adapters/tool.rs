//! Generic tool-call vocabulary. One `ToolCall` per logical tool
//! invocation; adapters update `ToolState` + append `ToolCallContent`
//! as the vendor emits subsequent notifications.

use serde::{Deserialize, Serialize};

/// Identity + kind of a tool call. `id` is the adapter-issued
/// identifier (stable across updates for the same call); `kind` keys
/// into the theme's `kind.*` palette (`read` / `write` / `bash` /
/// `search` / `agent` / `think` / `terminal` / `acp`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// Free-form for now — adapters normalise onto the theme keys
    /// listed above. Unknown kinds fall back to the neutral `acp` hue
    /// on the UI side.
    pub kind: String,
    /// Human-readable title / label. Typically the tool name or a
    /// short summary.
    pub title: String,
}

/// Phase of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Pending,
    Running,
    Completed,
    Failed,
}

/// One accumulated content block for a tool call. Variants mirror
/// the shapes the UI actually renders; adapters map their wire
/// shapes onto these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolCallContent {
    /// Text the tool emitted (stdout / log line / inline result).
    Text { text: String },
    /// File read / write payload preview.
    File { path: String, snippet: Option<String> },
    /// Raw JSON the tool produced. Pass-through for adapter-specific
    /// payloads the UI doesn't render structurally.
    Json { value: serde_json::Value },
}
