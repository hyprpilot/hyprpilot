//! Ripgrep completion source — manual trigger (Tab / Ctrl+Space)
//! that scans the active instance's cwd for words matching the
//! captain's prefix. Built on BurntSushi's `grep` family of crates
//! (the same primitives ripgrep itself ships with) — no `rg`
//! subprocess, no JSON parse, in-process cancellation via an
//! `AtomicBool` flag the registry flips.
//!
//! Inspired by `mikavilpas/blink-ripgrep.nvim`'s argv shape:
//!
//!     rg --no-config --json --word-regexp \
//!        --max-filesize=1M --ignore-case \
//!        -- <prefix>[\w_-]+ <project_root>
//!
//! Translated to in-process Rust: build a `RegexMatcher` for the
//! same pattern, walk the project root with `ignore::WalkBuilder`
//! (gitignore-aware, same crate ripgrep uses), and search each
//! file with `grep_searcher::Searcher`. Sink dedupes matched
//! bytes into a `HashSet<Box<str>>`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, Sink, SinkMatch};
use ignore::WalkBuilder;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use tokio::task;

use crate::completion::{
    resolve_cwd, token_before_cursor, CompletionContext, CompletionItem, CompletionKind, CompletionSource, Replacement,
    ReplacementRange,
};

/// Default minimum prefix when no config override is supplied.
/// Single letters return thousands of matches and burn CPU.
/// blink-ripgrep uses 3.
const DEFAULT_MIN_PREFIX: usize = 3;
/// Cap candidates returned to UI.
const MAX_RESULTS: usize = 50;
/// Ignore files larger than 1 MiB — same as blink-ripgrep's default.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// Cap on individual file matches collected before the sink halts.
/// Single-file mass-match (e.g., a generated source file with the
/// captain's prefix repeated thousands of times) shouldn't dominate
/// the candidate set.
const MAX_MATCHES_PER_FILE: usize = 50;

pub struct RipgrepSource {
    /// Auto-trigger on plain typing (no manual sigil). When false,
    /// ripgrep only fires when the UI ships `manual: true`
    /// (Tab / Ctrl+Space). Captain-driven via `[completion.ripgrep] auto`.
    auto: bool,
    /// Per-source min token length. Captain-driven via
    /// `[completion.ripgrep] min_prefix`; defaults to 3.
    min_prefix: usize,
}

impl RipgrepSource {
    pub fn new() -> Self {
        Self {
            auto: true,
            min_prefix: DEFAULT_MIN_PREFIX,
        }
    }

    /// Construct from the captain-supplied `[completion.ripgrep]`
    /// config block. Unset fields fall through to the defaults.
    pub fn from_config(cfg: &crate::config::RipgrepCompletionConfig) -> Self {
        Self {
            auto: cfg.auto.unwrap_or(true),
            min_prefix: cfg.min_prefix.unwrap_or(DEFAULT_MIN_PREFIX),
        }
    }
}

impl Default for RipgrepSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CompletionSource for RipgrepSource {
    fn id(&self) -> &'static str {
        "ripgrep"
    }

    fn detect(&self, text: &str, cursor: usize, manual: bool) -> Option<CompletionContext> {
        // Manual fires unconditionally; auto fires only when the
        // captain has opted in. Either path still requires a
        // meaningful prefix.
        if !manual && !self.auto {
            return None;
        }
        let (start, token) = token_before_cursor(text, cursor, &['_', '-']);
        if token.len() < self.min_prefix {
            return None;
        }
        Some(CompletionContext {
            trigger_offset: start,
            cursor,
            query: token.to_string(),
            sigil: None,
            manual,
        })
    }

    async fn fetch(
        &self,
        ctx: CompletionContext,
        cwd: Option<&Path>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<CompletionItem>> {
        let root = resolve_cwd(cwd);
        let prefix = ctx.query.clone();
        let cancel_clone = Arc::clone(&cancel);

        // grep_searcher is sync (callbacks block the thread). Run the
        // walk under spawn_blocking so the tokio runtime stays free
        // to service other RPCs / UI events.
        let words: HashSet<String> = task::spawn_blocking(move || -> Result<HashSet<String>> {
            let pattern = format!(r"\b{}[\w_-]*", regex::escape(&prefix));
            let matcher = RegexMatcherBuilder::new()
                .case_insensitive(true)
                .word(false) // we already have \b in the pattern
                .build(&pattern)?;

            let mut out: HashSet<String> = HashSet::new();
            let walker = WalkBuilder::new(&root)
                .max_filesize(Some(MAX_FILE_BYTES))
                .standard_filters(true)
                .build();

            for entry in walker {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.file_type().is_some_and(|t| !t.is_file()) {
                    continue;
                }

                let mut sink = WordSink::new(&mut out, &cancel_clone);
                let _ = Searcher::new().search_path(&matcher, entry.path(), &mut sink);
            }
            Ok(out)
        })
        .await??;

        // Rank dedupe set against the captain's query via nucleo for
        // a usable score order; the underlying regex already matched
        // the prefix.
        let pattern = Pattern::parse(&ctx.query, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut scored: Vec<(u32, String)> = words
            .into_iter()
            .filter_map(|word| {
                pattern
                    .score(nucleo_matcher::Utf32Str::Ascii(word.as_bytes()), &mut matcher)
                    .map(|s| (s, word))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.truncate(MAX_RESULTS);

        let range = ReplacementRange {
            start: ctx.trigger_offset,
            end: ctx.cursor,
        };

        let items = scored
            .into_iter()
            .map(|(_score, word)| CompletionItem {
                label: word.clone(),
                detail: Some("ripgrep".into()),
                kind: CompletionKind::Word,
                replacement: Replacement {
                    range: range.clone(),
                    text: word,
                },
                resolve_id: None, // documentation = match context; lazy fetch deferred for v1
            })
            .collect();
        Ok(items)
    }

    async fn resolve(&self, _resolve_id: &str) -> Result<Option<String>> {
        // v1: ripgrep items don't carry a resolve_id; the popover's
        // doc panel hides for word-kind rows. Future: expose match
        // context (5 surrounding lines from the source file).
        Ok(None)
    }
}

/// Sink that collects matched bytes into a HashSet, halting early
/// when the cancel flag flips or when per-file match count exceeds
/// the cap.
struct WordSink<'a> {
    out: &'a mut HashSet<String>,
    cancel: &'a Arc<AtomicBool>,
    file_count: usize,
}

impl<'a> WordSink<'a> {
    fn new(out: &'a mut HashSet<String>, cancel: &'a Arc<AtomicBool>) -> Self {
        Self {
            out,
            cancel,
            file_count: 0,
        }
    }
}

impl Sink for WordSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if self.file_count >= MAX_MATCHES_PER_FILE {
            return Ok(false);
        }
        let bytes = mat.bytes();
        // grep_searcher matches one line at a time; extract every
        // word-shaped run so a single line with multiple matches
        // contributes them all.
        for word in extract_words(bytes) {
            self.out.insert(word);
            self.file_count += 1;
            if self.file_count >= MAX_MATCHES_PER_FILE {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn extract_words(line: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &b) in line.iter().enumerate() {
        let c = b as char;
        let is_word = c.is_ascii_alphanumeric() || c == '_' || c == '-';
        if is_word {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            if let Ok(w) = std::str::from_utf8(&line[s..i]) {
                out.push(w.to_string());
            }
        }
    }
    if let Some(s) = start {
        if let Ok(w) = std::str::from_utf8(&line[s..]) {
            out.push(w.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detect_auto_default_claims_word_without_manual() {
        // Default config has `auto: true`, so plain typing claims the
        // trailing word once it crosses min_prefix.
        let source = RipgrepSource::new();
        let text = "scrolling through the wires";
        let cursor = text.len();
        let ctx = source.detect(text, cursor, false).unwrap();
        assert_eq!(ctx.query, "wires");
    }

    #[test]
    fn detect_auto_disabled_requires_manual() {
        // Captain opted out of auto-trigger.
        let cfg = crate::config::RipgrepCompletionConfig {
            auto: Some(false),
            debounce_ms: None,
            min_prefix: None,
        };
        let source = RipgrepSource::from_config(&cfg);
        let text = "scrolling through the wires";
        let cursor = text.len();
        // Without manual flag, source declines.
        assert!(source.detect(text, cursor, false).is_none());
        // Manual still fires.
        let ctx = source.detect(text, cursor, true).unwrap();
        assert_eq!(ctx.query, "wires");
    }

    #[test]
    fn detect_rejects_short_prefix() {
        let source = RipgrepSource::new();
        // 2-char prefix is too short.
        assert!(source.detect("hello ab", 8, true).is_none());
    }

    #[test]
    fn extract_words_splits_punctuation() {
        let words = extract_words(b"foo bar_baz qux-1, foo");
        assert_eq!(words, vec!["foo", "bar_baz", "qux-1", "foo"]);
    }

    #[tokio::test]
    async fn fetch_walks_cwd_dedupes_words() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn helloWorld() {} helloMars").unwrap();
        std::fs::write(dir.path().join("b.rs"), "let helloWorld = 1;").unwrap();

        let source = RipgrepSource::new();
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 5,
            query: "hel".into(),
            sigil: None,
            manual: true,
        };
        let items = source
            .fetch(ctx, Some(dir.path()), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"helloWorld"));
        assert!(labels.contains(&"helloMars"));
        // Dedupe: only one helloWorld even though it appears twice.
        let count = labels.iter().filter(|l| **l == "helloWorld").count();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn fetch_honors_cancel_flag() {
        let dir = tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("{i}.rs")), "helloWorld helloMars helloPluto").unwrap();
        }
        let cancel = Arc::new(AtomicBool::new(true));
        let source = RipgrepSource::new();
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 5,
            query: "hel".into(),
            sigil: None,
            manual: true,
        };
        let items = source.fetch(ctx, Some(dir.path()), cancel).await.unwrap();
        // Cancel flag was true from the start; walker exits immediately.
        assert!(items.is_empty());
    }
}
