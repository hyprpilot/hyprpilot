//! Wire shape every frontend (Vue overlay today, Neovim plugin
//! tomorrow) reads off `acp:transcript` / `acp:permission-request`
//! events. The daemon's per-tool formatters produce this; consumers
//! render verbatim.
//!
//! The shape is **rendering content only**. Lifecycle state
//! (`pending` / `running` / `completed` / `failed`) and the ACP tool
//! kind live on the surrounding `ToolCallRecord` — no need to
//! duplicate them here. Presentation chrome (icon / pill style /
//! permission-flow surface) is the consumer's call: each frontend
//! resolves `(toolKind, adapter, wireName)` against its own table.

use serde::{Deserialize, Serialize};

/// Single key/value row surfaced under the spec-sheet's structured
/// fields. `label` is a lowercase short prefix ("path" / "pattern"
/// / "tool"); `value` is free-form mono text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolField {
    pub label: String,
    pub value: String,
}

/// Daemon-authored tool-call presentation content. Every consumer
/// reads this off the wire; presentation chrome (icon / pill /
/// permission-flow surface) layers per-frontend via a
/// `(toolKind, adapter, wireName)` lookup table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormattedToolCall {
    /// Composed display string for the pill's center cell.
    pub title: String,
    /// Optional pill-right-cell metric ("1.4s" / "2 edits" / "11 chars").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
    /// Markdown body (fenced code blocks + prose). Consumer renders
    /// as markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tool execution result rendered as preformatted plain text
    /// (stdout / diff / file content).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Structured key/value rows for arg dumps.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fields: Vec<ToolField>,
}
