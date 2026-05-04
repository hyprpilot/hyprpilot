//! Path completion source — sigils `./`, `~/`, `/` at word boundary
//! trigger directory listing relative to the active instance's cwd.
//! Walks one segment at a time; captain types `/` to descend.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use tokio::fs;

use crate::completion::{
    resolve_cwd, CompletionContext, CompletionItem, CompletionKind, CompletionSource, Replacement, ReplacementRange,
};

const MAX_RESULTS: usize = 50;
/// Cap on file content shipped via `resolve()` — first N bytes of the
/// file. Binary files render as `(binary, X bytes)` instead.
const RESOLVE_PREVIEW_BYTES: usize = 16 * 1024;

pub struct PathSource;

impl PathSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PathSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CompletionSource for PathSource {
    fn id(&self) -> &'static str {
        "path"
    }

    fn detect(&self, text: &str, cursor: usize, _manual: bool) -> Option<CompletionContext> {
        // Path token: ./, ../, ~/, or / followed by [\w./_-]*. Walk
        // backwards from cursor over path-token chars, then verify we
        // hit a recognized prefix.
        let bytes = text.as_bytes();
        let mut start = cursor;
        while start > 0 {
            let prev = start - 1;
            let c = bytes[prev] as char;
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '_' | '-' | '~') {
                start = prev;
            } else {
                break;
            }
        }
        let token = &text[start..cursor];
        if token.is_empty() {
            return None;
        }
        // Prefix must match one of the recognized forms.
        let recognized = token.starts_with("./") || token.starts_with("../") || token.starts_with("~/")
            || token.starts_with('/')
            // Captain may have typed `.` or `~` and we want to keep
            // detecting until they hit `/` — but a bare `.` mid-text
            // isn't a path. Require the slash.
            ;
        if !recognized {
            return None;
        }
        // Word-boundary check: char before the token must be
        // whitespace, start-of-text, or `@` (the composer's
        // file-attachment trigger — captain types `@./foo.ts` to
        // pick a file as a Wire attachment instead of pasting the
        // path text). `@` was rejected as a non-whitespace boundary
        // before, defeating the attachment flow entirely.
        if start > 0 {
            let before = bytes[start - 1] as char;
            if !before.is_whitespace() && before != '@' {
                return None;
            }
        }
        Some(CompletionContext {
            trigger_offset: start,
            cursor,
            query: token.to_string(),
        })
    }

    async fn fetch(
        &self,
        ctx: CompletionContext,
        cwd: Option<&Path>,
        _cancel: Arc<AtomicBool>,
    ) -> Result<Vec<CompletionItem>> {
        let base = resolve_cwd(cwd);
        let (dir_to_list, query_segment) = split_path_query(&ctx.query, &base);
        let show_hidden = query_segment.starts_with('.');

        let mut entries = match fs::read_dir(&dir_to_list).await {
            Ok(rd) => rd,
            Err(_) => return Ok(Vec::new()),
        };

        let mut candidates: Vec<(String, bool)> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            candidates.push((name, is_dir));
        }

        let pattern = Pattern::parse(&query_segment, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);

        let mut scored: Vec<(u32, String, bool)> = candidates
            .into_iter()
            .filter_map(|(name, is_dir)| {
                let score: Option<u32> = if query_segment.is_empty() {
                    Some(0)
                } else {
                    pattern.score(nucleo_matcher::Utf32Str::Ascii(name.as_bytes()), &mut matcher)
                };
                score.map(|s| (s, name, is_dir))
            })
            .collect();
        // Directories sort before files at equal score; otherwise
        // higher score first.
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| (b.2 as u8).cmp(&(a.2 as u8)))
                .then_with(|| a.1.cmp(&b.1))
        });
        scored.truncate(MAX_RESULTS);

        // Compute the prefix of the original token we keep — everything
        // up to the last `/` — so the replacement preserves the
        // captain's typed directory hierarchy and only swaps the final
        // segment.
        let last_slash = ctx.query.rfind('/').map(|i| i + 1).unwrap_or(0);
        let prefix = &ctx.query[..last_slash];

        let range = ReplacementRange {
            start: ctx.trigger_offset,
            end: ctx.cursor,
        };

        let items = scored
            .into_iter()
            .map(|(_, name, is_dir)| {
                let mut text = format!("{prefix}{name}");
                if is_dir {
                    text.push('/');
                }
                CompletionItem {
                    label: name.clone(),
                    detail: Some(if is_dir { "dir".into() } else { "file".into() }),
                    kind: CompletionKind::Path,
                    replacement: Replacement {
                        range: range.clone(),
                        text,
                    },
                    resolve_id: Some(format!("path://{}", dir_to_list.join(&name).to_string_lossy())),
                }
            })
            .collect();
        Ok(items)
    }

    async fn resolve(&self, resolve_id: &str) -> Result<Option<String>> {
        let path = match resolve_id.strip_prefix("path://") {
            Some(p) => PathBuf::from(p),
            None => return Ok(None),
        };
        let metadata = match fs::metadata(&path).await {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        if metadata.is_dir() {
            return Ok(Some(format!("**directory** — `{}`", path.display())));
        }
        let bytes = match fs::read(&path).await {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        let preview = if bytes.iter().take(RESOLVE_PREVIEW_BYTES).any(|b| *b == 0) {
            format!("**binary** — `{}` ({} bytes)", path.display(), metadata.len())
        } else {
            let trimmed = if bytes.len() > RESOLVE_PREVIEW_BYTES {
                &bytes[..RESOLVE_PREVIEW_BYTES]
            } else {
                &bytes[..]
            };
            let lossy = String::from_utf8_lossy(trimmed);
            format!("```\n{lossy}\n```")
        };
        Ok(Some(preview))
    }
}

/// Resolve `query` against `base`, returning `(dir_to_list,
/// query_segment_for_filter)`.
///
/// - `./foo/bar` against base `/home/cenk/project` → list `/home/cenk/project/foo/`, filter for `bar`
/// - `../si` against base `/home/cenk/project/ui` → list `/home/cenk/project/`, filter for `si`
/// - `~/.cache` against base anything → list `$HOME/`, filter for `.cache`
/// - `/usr/lo` → list `/usr/`, filter for `lo`
fn split_path_query(query: &str, base: &Path) -> (PathBuf, String) {
    let last_slash = query.rfind('/');
    let (head, tail) = match last_slash {
        Some(i) => (&query[..=i], &query[i + 1..]),
        None => ("", query),
    };
    let head_path = if head.starts_with('/') {
        PathBuf::from(head)
    } else if let Some(stripped) = head.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(stripped)
        } else {
            PathBuf::from(stripped)
        }
    } else if let Some(stripped) = head.strip_prefix("./") {
        base.join(stripped)
    } else if head.starts_with("../") {
        base.join(head)
    } else if head.is_empty() {
        base.to_path_buf()
    } else {
        base.join(head)
    };
    (head_path, tail.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detect_dot_slash() {
        let source = PathSource::new();
        let ctx = source.detect("see ./fo", 8, false).unwrap();
        assert_eq!(ctx.trigger_offset, 4);
        assert_eq!(ctx.query, "./fo");
    }

    #[test]
    fn detect_tilde_slash() {
        let source = PathSource::new();
        let ctx = source.detect("~/.config", 9, false).unwrap();
        assert_eq!(ctx.trigger_offset, 0);
        assert_eq!(ctx.query, "~/.config");
    }

    #[test]
    fn detect_absolute_slash() {
        let source = PathSource::new();
        let ctx = source.detect("/usr/local", 10, false).unwrap();
        assert_eq!(ctx.trigger_offset, 0);
        assert_eq!(ctx.query, "/usr/local");
    }

    #[test]
    fn detect_rejects_mid_word_path() {
        let source = PathSource::new();
        // No whitespace before the path token.
        assert!(source.detect("hello./foo", 10, false).is_none());
    }

    #[test]
    fn detect_rejects_bare_dot() {
        let source = PathSource::new();
        // Bare `.` without slash isn't a path.
        assert!(source.detect("hello.", 6, false).is_none());
    }

    #[tokio::test]
    async fn fetch_lists_directory_entries() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("apple")).unwrap();
        std::fs::write(dir.path().join("banana.txt"), "b").unwrap();
        std::fs::write(dir.path().join(".hidden"), "h").unwrap();

        let source = PathSource::new();
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 2,
            query: "./".into(),
        };
        let items = source
            .fetch(ctx, Some(dir.path()), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"apple"));
        assert!(labels.contains(&"banana.txt"));
        assert!(!labels.contains(&".hidden"), "hidden file leaked");
    }

    #[tokio::test]
    async fn fetch_includes_hidden_when_query_starts_with_dot() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden"), "h").unwrap();
        std::fs::write(dir.path().join("visible"), "v").unwrap();

        let source = PathSource::new();
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 3,
            query: "./.".into(),
        };
        let items = source
            .fetch(ctx, Some(dir.path()), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&".hidden"));
    }

    #[test]
    fn split_path_query_relative_with_segment() {
        let base = PathBuf::from("/home/cenk/project");
        let (dir, q) = split_path_query("./foo/bar", &base);
        assert_eq!(dir, PathBuf::from("/home/cenk/project/foo/"));
        assert_eq!(q, "bar");
    }

    #[test]
    fn split_path_query_no_slash_uses_base() {
        let base = PathBuf::from("/home/cenk/project");
        let (dir, q) = split_path_query("foo", &base);
        assert_eq!(dir, base);
        assert_eq!(q, "foo");
    }
}
