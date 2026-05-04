//! `skills://<slug>` token hydrator.

use std::sync::Arc;

use crate::adapters::Attachment;
use crate::skills::SkillsRegistry;

use super::TokenHydrator;

/// Looks the slug up against the shared `SkillsRegistry` and projects
/// the loaded skill into an `Attachment`. Registered into the daemon's
/// `TokenHydrators` at boot (see `daemon::mod::setup_app`).
pub struct SkillTokenHydrator {
    registry: Arc<SkillsRegistry>,
}

impl SkillTokenHydrator {
    #[must_use]
    pub fn new(registry: Arc<SkillsRegistry>) -> Self {
        Self { registry }
    }
}

impl TokenHydrator for SkillTokenHydrator {
    fn scheme(&self) -> &'static str {
        "skills"
    }

    fn hydrate(&self, value: &str) -> Option<Attachment> {
        use crate::skills::SkillSlug;
        let slug = SkillSlug::parse(value).ok()?;
        let skill = self.registry.get(&slug)?;
        // Skill bundles live at `<root>/<slug>/SKILL.md`; the path's
        // basename is always the literal `SKILL.md`, which makes a
        // useless transcript pill ("SKILL.md", "SKILL.md", "SKILL.md"
        // for three different skill picks). The captain referenced
        // the skill by its slug (`#{skill/git-commit}` or palette
        // pick) — that's what they'll recognise. Prefer the
        // frontmatter `title` when authored, fall back to slug
        // otherwise. Empty frontmatter title (no `title:` field at
        // all) hits the slug path; non-empty title wins for richer
        // labels like "Git commit helper".
        let title = if skill.title.trim().is_empty() {
            slug.as_str().to_string()
        } else {
            skill.title.clone()
        };
        Some(Attachment {
            slug: slug.as_str().to_string(),
            path: skill.path.clone(),
            body: skill.body.clone(),
            title: Some(title),
            data: None,
            mime: Some("text/markdown".to_string()),
        })
    }
}
