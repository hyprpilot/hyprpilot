//! Skills completion source — sigil `#` triggers slug autocomplete
//! against the daemon's [`SkillsRegistry`]. Picked items insert
//! `#{skills://<slug>}` into the textarea; submission-time
//! `attachments_hydrate` parses these back into `Attachment`
//! payloads on the wire.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use crate::completion::{
    token_before_cursor, CompletionContext, CompletionItem, CompletionKind, CompletionSource, Replacement,
    ReplacementRange,
};
use crate::skills::{SkillSlug, SkillsRegistry};

/// Cap on candidates returned per query — UI scrolls if more.
const MAX_RESULTS: usize = 50;

pub struct SkillsSource {
    registry: Arc<SkillsRegistry>,
}

impl SkillsSource {
    pub fn new(registry: Arc<SkillsRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl CompletionSource for SkillsSource {
    fn id(&self) -> &'static str {
        "skills"
    }

    fn detect(&self, text: &str, cursor: usize, _manual: bool) -> Option<CompletionContext> {
        // Find the `#` immediately preceding the slug-character run
        // ending at cursor. Slug chars are `[a-z0-9_-]` (matches
        // SkillSlug::parse). The `#` must sit at a word boundary —
        // either at start-of-text or after whitespace.
        let bytes = text.as_bytes();
        let (token_start, token) = token_before_cursor(text, cursor, &['_', '-']);
        // Token must be preceded by a `#`.
        if token_start == 0 {
            return None;
        }
        let sigil_idx = token_start - 1;
        if bytes[sigil_idx] as char != '#' {
            return None;
        }
        // `#` must be at a word boundary — start of text or preceded
        // by whitespace.
        if sigil_idx > 0 {
            let before = bytes[sigil_idx - 1] as char;
            if !before.is_whitespace() {
                return None;
            }
        }
        Some(CompletionContext {
            trigger_offset: sigil_idx,
            cursor,
            query: token.to_string(),
        })
    }

    async fn fetch(
        &self,
        ctx: CompletionContext,
        _cwd: Option<&Path>,
        _cancel: Arc<AtomicBool>,
    ) -> Result<Vec<CompletionItem>> {
        let skills = self.registry.list();
        let pattern = Pattern::parse(&ctx.query, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);

        // Score each skill against the query; nucleo's match() returns
        // None when the pattern doesn't fit.
        let mut scored: Vec<(u32, &str, String, String)> = skills
            .iter()
            .filter_map(|skill| {
                let label = skill.slug.as_str();
                pattern
                    .score(nucleo_matcher::Utf32Str::Ascii(label.as_bytes()), &mut matcher)
                    .map(|score| {
                        (
                            score,
                            skill.slug.as_str(),
                            skill.title.clone(),
                            skill.description.clone(),
                        )
                    })
            })
            .collect();
        // Higher score first; nucleo returns u32 where bigger is better.
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        scored.truncate(MAX_RESULTS);

        let range = ReplacementRange {
            start: ctx.trigger_offset,
            end: ctx.cursor,
        };

        let items = scored
            .into_iter()
            .map(|(_score, slug, title, description)| CompletionItem {
                // Title is the human-friendly name shown to the
                // captain; the slug stays in the URL token. Falls
                // back to slug when the SKILL.md has no title /
                // first-H1 (so the row never renders empty). Nucleo
                // matched against slug above, so typing `gc` for
                // `git-commit` still filters to the right entry.
                label: if title.trim().is_empty() {
                    slug.to_string()
                } else {
                    title
                },
                // Description renders in dim parens after the label;
                // None / empty hides the parens entirely.
                detail: if description.trim().is_empty() {
                    None
                } else {
                    Some(description)
                },
                kind: CompletionKind::Skill,
                replacement: Replacement {
                    range: range.clone(),
                    text: format!("#{{skills://{slug}}}"),
                },
                resolve_id: Some(format!("skills://{slug}")),
            })
            .collect();

        Ok(items)
    }

    async fn resolve(&self, resolve_id: &str) -> Result<Option<String>> {
        // resolve_id format: "skills://<slug>"
        let slug = resolve_id.strip_prefix("skills://").unwrap_or(resolve_id);
        let parsed = match SkillSlug::parse(slug) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        Ok(self.registry.get(&parsed).map(|s| s.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SkillsRegistry;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_registry_with(slugs: &[(&str, &str)]) -> Arc<SkillsRegistry> {
        let dir = tempdir().unwrap();
        for (slug, body) in slugs {
            let skill_dir = dir.path().join(slug);
            std::fs::create_dir_all(&skill_dir).unwrap();
            let frontmatter = format!("---\nname: {slug}\ndescription: {slug} description\n---\n");
            std::fs::write(skill_dir.join("SKILL.md"), format!("{frontmatter}\n{body}")).unwrap();
        }
        let registry = Arc::new(SkillsRegistry::new(vec![PathBuf::from(dir.path())]));
        registry.reload().unwrap();
        // Leak the tempdir so the files survive past the function (tests own
        // their own scoped tempdirs in real callers; here we just need
        // longevity for the in-test query path).
        std::mem::forget(dir);
        registry
    }

    #[test]
    fn detect_skill_at_word_boundary() {
        let registry = Arc::new(SkillsRegistry::new(vec![]));
        let source = SkillsSource::new(registry);
        let text = "hello #git";
        let cursor = text.len();
        let ctx = source.detect(text, cursor, false).unwrap();
        assert_eq!(ctx.trigger_offset, 6);
        assert_eq!(ctx.cursor, cursor);
        assert_eq!(ctx.query, "git");
    }

    #[test]
    fn detect_skill_at_text_start() {
        let registry = Arc::new(SkillsRegistry::new(vec![]));
        let source = SkillsSource::new(registry);
        let ctx = source.detect("#abc", 4, false).unwrap();
        assert_eq!(ctx.trigger_offset, 0);
        assert_eq!(ctx.query, "abc");
    }

    #[test]
    fn detect_skill_rejects_mid_word() {
        let registry = Arc::new(SkillsRegistry::new(vec![]));
        let source = SkillsSource::new(registry);
        // Mid-word `#` (no whitespace before) — not a sigil context.
        assert!(source.detect("foo#bar", 7, false).is_none());
    }

    #[test]
    fn detect_skill_rejects_when_no_sigil() {
        let registry = Arc::new(SkillsRegistry::new(vec![]));
        let source = SkillsSource::new(registry);
        assert!(source.detect("just typing", 11, false).is_none());
    }

    #[tokio::test]
    async fn fetch_ranks_and_inserts_uri_token() {
        let registry = make_registry_with(&[
            ("git-commit", "git commit body"),
            ("git-branch", "git branch body"),
            ("docker-up", "docker body"),
        ]);
        let source = SkillsSource::new(registry);
        let ctx = CompletionContext {
            trigger_offset: 0,
            cursor: 4,
            query: "git".into(),
        };
        let items = source.fetch(ctx, None, Arc::new(AtomicBool::new(false))).await.unwrap();
        // Both git-* skills match; docker-up doesn't.
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"git-commit"));
        assert!(labels.contains(&"git-branch"));
        assert!(!labels.contains(&"docker-up"));
        // First item's replacement is `#{skills://<slug>}`.
        assert!(items[0].replacement.text.starts_with("#{skills://"));
        assert!(items[0].replacement.text.ends_with('}'));
    }

    #[tokio::test]
    async fn resolve_returns_skill_body() {
        let registry = make_registry_with(&[("hello-world", "the body content")]);
        let source = SkillsSource::new(registry);
        let body = source.resolve("skills://hello-world").await.unwrap();
        assert!(body.is_some());
        assert!(body.unwrap().contains("the body content"));
    }

    #[tokio::test]
    async fn resolve_unknown_slug_returns_none() {
        let registry = make_registry_with(&[("real", "")]);
        let source = SkillsSource::new(registry);
        assert!(source.resolve("skills://nonexistent").await.unwrap().is_none());
    }
}
