//! `skills://<slug>` token hydrator.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::{AcpAdapter, Attachment};

use super::TokenHydrator;

/// Looks the slug up against the focused instance's `SkillsRegistry`
/// (the only authoritative skills view today — daemon-global skills
/// are gone) and projects the loaded skill into an `Attachment`.
/// Registered into the daemon's `TokenHydrators` at boot.
pub struct SkillTokenHydrator {
    adapter: Arc<AcpAdapter>,
}

impl SkillTokenHydrator {
    #[must_use]
    pub fn new(adapter: Arc<AcpAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl TokenHydrator for SkillTokenHydrator {
    fn scheme(&self) -> &'static str {
        "skills"
    }

    async fn hydrate(&self, value: &str) -> Option<Attachment> {
        use crate::skills::SkillSlug;
        let slug = SkillSlug::parse(value).ok()?;
        let registry = self.adapter.focused_skills().await?;
        let skill = registry.get(&slug)?;
        // Skill bundles live at `<root>/<slug>/SKILL.md`; the path's
        // basename is always the literal `SKILL.md`, which makes a
        // useless transcript pill. Prefer the frontmatter `title` when
        // authored, fall back to slug otherwise.
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
