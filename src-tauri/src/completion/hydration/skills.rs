//! `#{hyprpilot://skills/<slug>}` token hydrator.
//!
//! Owns the `hyprpilot` URI scheme. Parses the addressed sub-resource
//! out of the token's value portion and produces a body-less
//! `Attachment` for the wire — the actual skill body is fetched
//! lazily by the agent via the auto-injected `hyprpilot` MCP server
//! (`mcp__hyprpilot__read_skill`). The daemon's
//! `attachment_to_block` substitutes a markdown hydration blob at
//! prompt-build time so the agent knows the URI + how to fetch.
//!
//! Today only `hyprpilot://skills/<slug>` is recognised. Future
//! sub-resources (e.g. `hyprpilot://workspace/...`) extend the
//! `parse_subresource` match.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::{AcpAdapter, Attachment};

use super::TokenHydrator;

/// Generic hydrator for `#{hyprpilot://<subresource>/<id>}` tokens.
/// Single hydrator covers every in-tree hyprpilot sub-resource because
/// they all share the same MCP server (`hyprpilot`) and the same
/// downstream auto-inject machinery — splitting per-subresource would
/// just duplicate the lookup.
pub struct HyprpilotTokenHydrator {
    adapter: Arc<AcpAdapter>,
}

impl HyprpilotTokenHydrator {
    #[must_use]
    pub fn new(adapter: Arc<AcpAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl TokenHydrator for HyprpilotTokenHydrator {
    fn scheme(&self) -> &'static str {
        "hyprpilot"
    }

    async fn hydrate(&self, value: &str) -> Option<Attachment> {
        let (kind, id) = value.split_once('/')?;
        match kind {
            "skills" => self.hydrate_skill(id).await,
            _ => {
                tracing::warn!(sub = kind, value, "hyprpilot token hydrator: unknown sub-resource");
                None
            }
        }
    }
}

impl HyprpilotTokenHydrator {
    /// Project a skill slug onto a body-less attachment. The actual
    /// body never lands on the wire — the daemon's
    /// `attachment_to_block` swaps in a hydration blob pointing at the
    /// `hyprpilot://skills/<slug>` MCP resource. Lookup against the
    /// focused-instance registry still happens because we need the
    /// path + title for the pill / detection heuristic; `body` stays
    /// empty.
    async fn hydrate_skill(&self, slug: &str) -> Option<Attachment> {
        use crate::skills::SkillSlug;
        let parsed = SkillSlug::parse(slug).ok()?;
        let registry = self.adapter.focused_skills().await?;
        let skill = registry.get(&parsed)?;
        let title = if skill.title.trim().is_empty() {
            parsed.as_str().to_string()
        } else {
            skill.title.clone()
        };
        Some(Attachment {
            slug: parsed.as_str().to_string(),
            path: skill.path.clone(),
            // Body stays empty — `attachment_to_block` substitutes a
            // markdown hydration blob pointing at the MCP resource.
            // Shipping the body here would defeat the lazy-fetch win.
            body: String::new(),
            title: Some(title),
            data: None,
            // mime intentionally absent — the daemon's
            // `is_skill_attachment` heuristic detects skill attachments
            // by slug + `SKILL.md` path + no binary data; an explicit
            // mime would flip them into the generic text path.
            mime: None,
        })
    }
}
