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

/// One pill-right-cell metric. Tagged enum on the wire
/// (`{ "kind": "diff", "added": 12, "removed": 3 }`). Frontends
/// switch on `kind` and render each variant via its own chrome —
/// `Text` as a dim mono pill, `Diff` as a +N / −M two-pill pair,
/// `Duration` formatted via the UI's `formatDuration` helper,
/// `Matches` (defined for future use; no tool currently emits it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stat {
    /// Free-form pill text — used for tools whose stat doesn't fit
    /// any structured variant (todo status counts, ad-hoc summaries).
    Text { value: String },
    /// Line-count diff for write / edit / multi_edit tools. UI
    /// renders `+added` (ok-toned pill) and `−removed` (err-toned
    /// pill); zero side hides.
    Diff { added: u32, removed: u32 },
    /// Wall-clock duration in milliseconds. UI formats via
    /// `formatDuration(ms)` → `"850ms"` / `"3s"` / `"1m 3s"`.
    Duration { ms: u64 },
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
    /// Pill-right-cell metrics. Multiple stats render side-by-side
    /// as separate mini-pills (e.g. `+12 −3` AND `2.4s` for a slow
    /// edit). Empty vec when no stat applies. Always serialised
    /// (even when empty) — frontends type `stats` as a required
    /// `Stat[]` and `.length` / `.reduce` reads on `undefined` would
    /// crash the render after the first empty-stats update.
    #[serde(default)]
    pub stats: Vec<Stat>,
    /// Markdown body (fenced code blocks + prose). Consumer renders
    /// as markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tool execution result rendered as preformatted plain text
    /// (stdout / diff / file content).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Structured key/value rows for arg dumps. Always serialised
    /// (even when empty) — same rationale as `stats`: frontends type
    /// it as required.
    #[serde(default)]
    pub fields: Vec<ToolField>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the wire shape every frontend depends on. A rename of
    /// `kind` or a variant tag here would silently strand the UI.
    #[test]
    fn stat_serde_roundtrip() {
        let cases = [
            (
                Stat::Text { value: "hello".into() },
                r#"{"kind":"text","value":"hello"}"#,
            ),
            (
                Stat::Diff { added: 12, removed: 3 },
                r#"{"kind":"diff","added":12,"removed":3}"#,
            ),
            (Stat::Duration { ms: 12345 }, r#"{"kind":"duration","ms":12345}"#),
        ];
        for (stat, expected) in cases {
            let json = serde_json::to_string(&stat).expect("serialise");
            assert_eq!(json, expected, "{stat:?}");
            let back: Stat = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(back, stat);
        }
    }

    /// Empty `stats` MUST still serialise as `"stats":[]`. Frontends
    /// type the field as required — dropping it produces a runtime
    /// `Cannot read properties of undefined` the moment a tool emits
    /// an update with no stats (which is the steady state for every
    /// tool that doesn't surface a metric: read, glob, plan, …).
    #[test]
    fn formatted_tool_call_emits_empty_stats_array() {
        let f = FormattedToolCall {
            title: "bash".into(),
            stats: Vec::new(),
            description: None,
            output: None,
            fields: Vec::new(),
        };
        let json = serde_json::to_string(&f).expect("serialise");
        assert!(
            json.contains("\"stats\":[]"),
            "empty stats vec must serialise as []: {json}"
        );
        assert!(
            json.contains("\"fields\":[]"),
            "empty fields vec must serialise as []: {json}"
        );
    }
}
