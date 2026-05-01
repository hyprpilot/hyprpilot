//! Composer autocomplete — daemon-side completion engines.
//!
//! Each engine implements [`CompletionSource`] and registers with
//! [`CompletionRegistry`]. The RPC handler (`completion/query`) walks
//! sources in registration order; the first source whose [`detect`]
//! returns a context owns the response. Inspired by blink.cmp's
//! "source per engine" plugin pattern, except every source runs
//! in the daemon — UI just renders.
//!
//! Sources land in priority order:
//!   1. [`source::commands::CommandsSource`] — `/` at start of message.
//!   2. [`source::skills::SkillsSource`] — `#` at word boundary.
//!   3. [`source::path::PathSource`] — `./`, `~/`, `/<path>` at word
//!      boundary.
//!   4. [`source::ripgrep::RipgrepSource`] — manual fallback (Tab /
//!      Ctrl+Space) over the active instance's transcript + cwd.
//!
//! Cancellation: every fetch takes an `Arc<AtomicBool>` cancel flag
//! the registry flips when a newer query arrives. Static sources
//! (skills / commands) finish in sub-ms and ignore it; ripgrep
//! checks it between match emits and halts the walk early. The
//! flag is the cooperative escape hatch — dropping the future on
//! the JoinHandle side already aborts the task, the flag just
//! avoids work the dispatcher already abandoned.
//!
//! [`detect`]: CompletionSource::detect

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod cancellations;
pub mod commands;
pub mod source;

pub use cancellations::CompletionCancellations;

/// Closed-set of source kinds — drives the UI's per-row icon and
/// the analytics-friendly `source_id` log key. Wire-shape uses
/// snake_case strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Skill,
    Path,
    Word,
    Command,
}

/// A trigger context the source built from `detect()`. The registry
/// passes this back into `fetch()` so the source doesn't re-derive
/// what it already saw.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CompletionContext {
    /// Byte offset where the trigger starts in the textarea. The UI's
    /// replacement range starts here and ends at the cursor.
    pub trigger_offset: usize,
    /// Cursor position. The replacement range ends here.
    pub cursor: usize,
    /// Text from `trigger_offset` to `cursor`, minus the leading
    /// sigil if any. Sources nucleo-rank against this.
    pub query: String,
    /// Trigger sigil — `Some('#')` for skills, `Some('/')` for
    /// commands, etc. `None` for ripgrep (manual trigger).
    pub sigil: Option<char>,
    /// Set when the captain explicitly asked for completions via
    /// Tab / Ctrl+Space. Sources that decline auto-trigger can
    /// still detect when this is true.
    pub manual: bool,
}

/// Where the picked item replaces text in the textarea. Byte
/// offsets refer to the captain's full composer text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Replacement {
    pub range: ReplacementRange,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementRange {
    pub start: usize,
    pub end: usize,
}

/// One row in the completion popover. The UI renders these
/// verbatim — daemon already ranked + truncated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub kind: CompletionKind,
    pub replacement: Replacement,
    /// Lazy doc-fetch key. UI sends `completion/resolve { resolveId }`
    /// 80ms after selection settles; daemon dispatches to the
    /// owning source's `resolve()`. `None` when this item has no
    /// documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_id: Option<String>,
}

/// A completion engine. Implementors live in
/// `completion::source::*`. The registry calls `detect` on every
/// query; the first source whose detect returns `Some` owns the
/// response and gets its `fetch` called. `resolve` is invoked
/// on-demand for documentation.
#[async_trait]
pub trait CompletionSource: Send + Sync {
    fn id(&self) -> &'static str;

    /// Inspect the captain's text + cursor and decide whether this
    /// source applies. Returns `None` to defer to the next source.
    /// `manual` is true when the captain explicitly asked for
    /// completions (Tab / Ctrl+Space).
    fn detect(&self, text: &str, cursor: usize, manual: bool) -> Option<CompletionContext>;

    /// Produce candidates ranked + truncated. Sources should
    /// nucleo-rank against `ctx.query` and cap output at 50.
    /// `cancel` is the registry-managed flag; ripgrep checks it
    /// between matches.
    async fn fetch(
        &self,
        ctx: CompletionContext,
        cwd: Option<&Path>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<CompletionItem>>;

    /// Return markdown documentation for the item identified by
    /// `resolve_id`. `None` means no docs available; the UI hides
    /// the doc panel.
    async fn resolve(&self, resolve_id: &str) -> Result<Option<String>>;
}

/// Ordered registry of every active source. Cheap to construct + clone
/// (sources are `Arc`-shaped internally where they need shared state);
/// the daemon constructs one and wraps in `Arc<RwLock<>>` for managed
/// state.
pub struct CompletionRegistry {
    sources: Vec<Arc<dyn CompletionSource>>,
}

impl CompletionRegistry {
    pub fn new() -> Self {
        Self { sources: Vec::new() }
    }

    pub fn with_source(mut self, source: Arc<dyn CompletionSource>) -> Self {
        self.sources.push(source);
        self
    }

    /// Walk sources in order, return `(source, ctx)` for the first
    /// one whose detect matches.
    pub fn detect(
        &self,
        text: &str,
        cursor: usize,
        manual: bool,
    ) -> Option<(Arc<dyn CompletionSource>, CompletionContext)> {
        for source in &self.sources {
            if let Some(ctx) = source.detect(text, cursor, manual) {
                return Some((Arc::clone(source), ctx));
            }
        }
        None
    }

    /// Look up a source by id — used by `completion/resolve` to
    /// route the resolve call to the source that produced the
    /// item (UI passes the source id back).
    pub fn source_by_id(&self, id: &str) -> Option<Arc<dyn CompletionSource>> {
        self.sources.iter().find(|s| s.id() == id).cloned()
    }
}

impl Default for CompletionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper: extract the cursor-anchored token (alphanumeric + a small
/// punctuation set) backwards from `cursor` until we hit whitespace
/// or the start of text. Used by every source's detect rule.
pub(crate) fn token_before_cursor<'a>(text: &'a str, cursor: usize, allowed_punct: &[char]) -> (usize, &'a str) {
    let bytes = text.as_bytes();
    let mut start = cursor;
    while start > 0 {
        let prev = start - 1;
        // Walk only over single-byte ASCII chars for the boundary
        // search — the token surface is ASCII-only anyway (slug
        // characters / path characters / word characters).
        let c = bytes[prev] as char;
        if c.is_ascii_alphanumeric() || allowed_punct.contains(&c) {
            start = prev;
        } else {
            break;
        }
    }
    (start, &text[start..cursor])
}

/// Helper: resolve cwd for a query, falling back to the binary's
/// own cwd if none was provided. Sources that need a base directory
/// (path, ripgrep) call this so they don't have to re-implement
/// the fallback.
pub(crate) fn resolve_cwd(cwd: Option<&Path>) -> PathBuf {
    cwd.map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}
