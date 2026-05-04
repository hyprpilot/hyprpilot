//! Slash-commands completion source — `/` at start of message
//! triggers ACP `available_commands` autocomplete. The cache is
//! held externally as a shared `RwLock<Vec<CommandSummary>>`; this
//! source just reads it.
//!
//! v1 wiring: `detect()` recognizes `/` at start-of-message,
//! `fetch()` queries the shared cache. The
//! `available_commands_update` notification handling that
//! populates the cache on `AcpInstance` lands in a follow-up;
//! until then the cache stays empty and the source returns `[]`,
//! the popover renders "no completions" without crashing the
//! flow.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use async_trait::async_trait;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use crate::completion::{
    CompletionContext, CompletionItem, CompletionKind, CompletionSource, Replacement, ReplacementRange,
};

const MAX_RESULTS: usize = 50;

/// Adapter-agnostic projection used by the slash source. Mirrors the
/// fields ACP's `available_commands_update` ships.
#[derive(Debug, Clone)]
pub struct CommandSummary {
    pub name: String,
    pub description: Option<String>,
}

/// Shared cache the daemon constructs at boot and writes into when an
/// `available_commands_update` arrives. The slash source reads from
/// it on every `fetch`.
pub type CommandsCache = Arc<RwLock<Vec<CommandSummary>>>;

pub struct CommandsSource {
    cache: CommandsCache,
}

impl CommandsSource {
    pub fn new(cache: CommandsCache) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl CompletionSource for CommandsSource {
    fn id(&self) -> &'static str {
        "commands"
    }

    fn detect(&self, text: &str, cursor: usize, _manual: bool) -> Option<CompletionContext> {
        // `/` must sit at start-of-message — skipping leading
        // whitespace, the first non-whitespace char must be `/`.
        // Cursor must lie inside the slash + word-token run.
        let bytes = text.as_bytes();
        let mut idx = 0;
        while idx < bytes.len() && (bytes[idx] as char).is_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() || bytes[idx] as char != '/' {
            return None;
        }
        let slash_idx = idx;
        let mut end = slash_idx + 1;
        while end < bytes.len() {
            let c = bytes[end] as char;
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                end += 1;
            } else {
                break;
            }
        }
        if cursor < slash_idx + 1 || cursor > end {
            return None;
        }
        Some(CompletionContext {
            trigger_offset: slash_idx,
            cursor,
            query: text[slash_idx + 1..cursor].to_string(),
        })
    }

    async fn fetch(
        &self,
        ctx: CompletionContext,
        _cwd: Option<&Path>,
        _cancel: Arc<AtomicBool>,
    ) -> Result<Vec<CompletionItem>> {
        let commands = self.cache.read().map(|c| c.clone()).unwrap_or_default();

        let pattern = Pattern::parse(&ctx.query, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);

        let mut scored: Vec<(u32, CommandSummary)> = commands
            .into_iter()
            .filter_map(|cmd| {
                pattern
                    .score(nucleo_matcher::Utf32Str::Ascii(cmd.name.as_bytes()), &mut matcher)
                    .map(|s| (s, cmd))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.truncate(MAX_RESULTS);

        let range = ReplacementRange {
            start: ctx.trigger_offset,
            end: ctx.cursor,
        };

        let items = scored
            .into_iter()
            .map(|(_score, cmd)| {
                // Description lives behind the lazy doc panel, not on
                // the row — slash-command lists are dense and the name
                // alone reads cleanly. resolve_id carries the lookup
                // key whether or not a description is present so the
                // panel still shows "(no description)" rather than
                // disappearing on selection.
                CompletionItem {
                    label: cmd.name.clone(),
                    detail: None,
                    kind: CompletionKind::Command,
                    replacement: Replacement {
                        range: range.clone(),
                        text: format!("/{}", cmd.name),
                    },
                    resolve_id: Some(format!("commands://{}", cmd.name)),
                }
            })
            .collect();
        Ok(items)
    }

    async fn resolve(&self, resolve_id: &str) -> Result<Option<String>> {
        let name = match resolve_id.strip_prefix("commands://") {
            Some(n) => n,
            None => return Ok(None),
        };
        let commands = self.cache.read().map(|c| c.clone()).unwrap_or_default();
        let summary = commands.into_iter().find(|c| c.name == name);
        // Always return Some(...) so the docs panel renders even when
        // the agent didn't ship a description for the command — the
        // popover keeps showing the slash command's identity instead
        // of collapsing the panel mid-keystroke.
        let body = match summary {
            Some(c) => c.description.unwrap_or_else(|| "_(no description)_".to_string()),
            None => return Ok(None),
        };
        Ok(Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cache() -> CommandsCache {
        Arc::new(RwLock::new(Vec::new()))
    }

    fn populated_cache(entries: &[(&str, Option<&str>)]) -> CommandsCache {
        Arc::new(RwLock::new(
            entries
                .iter()
                .map(|(name, desc)| CommandSummary {
                    name: (*name).to_string(),
                    description: desc.map(|s| s.to_string()),
                })
                .collect(),
        ))
    }

    #[test]
    fn detect_slash_at_start() {
        let source = CommandsSource::new(empty_cache());
        let ctx = source.detect("/he", 3, false).unwrap();
        assert_eq!(ctx.trigger_offset, 0);
        assert_eq!(ctx.query, "he");
    }

    #[test]
    fn detect_slash_after_leading_whitespace() {
        let source = CommandsSource::new(empty_cache());
        let ctx = source.detect("  /he", 5, false).unwrap();
        assert_eq!(ctx.trigger_offset, 2);
    }

    #[test]
    fn detect_rejects_slash_mid_message() {
        let source = CommandsSource::new(empty_cache());
        // Slash mid-text — that's a path or content, not a command.
        assert!(source.detect("hello /world", 12, false).is_none());
    }

    #[tokio::test]
    async fn fetch_returns_empty_on_empty_cache() {
        let source = CommandsSource::new(empty_cache());
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 1,
            query: String::new(),
        };
        let items = source.fetch(ctx, None, Arc::new(AtomicBool::new(false))).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn fetch_ranks_against_query() {
        let cache = populated_cache(&[
            ("help", Some("show help")),
            ("clear", Some("clear chat")),
            ("compact", None),
        ]);
        let source = CommandsSource::new(cache);
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 3,
            query: "cl".into(),
        };
        let items = source.fetch(ctx, None, Arc::new(AtomicBool::new(false))).await.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"clear"));
        assert!(items[0].replacement.text.starts_with('/'));
    }

    #[tokio::test]
    async fn resolve_returns_description() {
        let cache = populated_cache(&[("help", Some("the help body"))]);
        let source = CommandsSource::new(cache);
        let body = source.resolve("commands://help").await.unwrap();
        assert_eq!(body.as_deref(), Some("the help body"));
    }
}
