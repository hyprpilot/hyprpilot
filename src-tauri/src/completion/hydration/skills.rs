//! `#{hyprpilot://skills/<slug>}` token hydrator.
//!
//! Owns the `hyprpilot` URI scheme. Parses the addressed sub-resource
//! out of the token's value portion and produces an `Attachment`
//! whose **body is the hydration blob** — a short markdown brief
//! pointing the agent at the MCP resource + tools it needs to read
//! the skill body on demand. The full SKILL.md body is **never**
//! shipped on the wire; the agent fetches it lazily via
//! `mcp__hyprpilot__read_skill` (or by reading the resource URI
//! directly).
//!
//! The attachment's `slug` field is the **full resource URI**
//! (`hyprpilot://skills/<slug>`) — same shape the palette inserts
//! and the MCP server exposes. Makes attachments universally
//! identifiable across palette → daemon → agent.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::{AcpAdapter, Attachment};

use super::TokenHydrator;

/// Generic hydrator for `#{hyprpilot://<subresource>/<id>}` tokens.
/// Single hydrator covers every in-tree hyprpilot sub-resource
/// because they all share the same MCP server (`hyprpilot`) and the
/// same downstream auto-inject machinery — splitting per-subresource
/// would just duplicate the lookup.
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
    /// Project a skill slug onto a hydration-blob attachment. The
    /// attachment's `body` is the markdown brief; the actual SKILL.md
    /// body never lands on the wire here (the agent reads it lazily
    /// via `mcp__hyprpilot__read_skill`). The attachment `slug` is
    /// the **full resource URI** (`hyprpilot://skills/<slug>`) so
    /// every consumer can identify it without parsing.
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
        let bundle_dir = skill
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| skill.path.clone());
        let blob = render_skill_blob(parsed.as_str(), &title, &skill.description, &bundle_dir);
        Some(Attachment {
            slug: skill_resource_uri(parsed.as_str()),
            path: skill.path.clone(),
            body: blob,
            title: Some(title),
            data: None,
            mime: Some("text/markdown".to_string()),
        })
    }
}

/// Build the full resource URI for a skill slug — the same shape the
/// palette autocomplete inserts and the MCP server exposes. Single
/// authoritative source so callers can't drift.
#[must_use]
pub fn skill_resource_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}")
}

/// Render the markdown hydration blob the agent reads in place of the
/// skill body. Carries:
///   * the resource URI + matching tool names so the agent knows the
///     MCP affordance;
///   * the absolute bundle directory so the agent can resolve sibling
///     files (scripts, fixtures, references) relative to it without
///     re-asking the daemon;
///   * a short generic guidance block on how to acknowledge a loaded
///     skill, how to handle prerequisites, and when to ask vs. act.
///
/// The guidance is intentionally generic — the brief tells the agent
/// **how to behave when a skill is loaded**, not what any specific
/// skill instructs. The skill body itself (read on demand) is the
/// authoritative source of domain-specific rules.
#[must_use]
pub fn render_skill_blob(slug: &str, title: &str, description: &str, bundle_dir: &Path) -> String {
    let uri = skill_resource_uri(slug);
    let bundle = bundle_dir.display();
    let desc_line = if description.trim().is_empty() {
        String::new()
    } else {
        format!("\n{description}")
    };
    format!(
        "## Skill attached: `{slug}` ({title}){desc_line}\n\
\n\
- **Resource URI**: `{uri}` — read it via `mcp__hyprpilot__read_skill` with `{{\"slug\": \"{slug}\"}}`, or `resources/read` on the URI directly.\n\
- **Bundle directory**: `{bundle}` — resolve any scripts, fixtures, or files the skill references relative to this path.\n\
- **References**: bundled via `mcp__hyprpilot__load_skill_references` with `{{\"slug\": \"{slug}\"}}`, or by reading `{uri}/references`.\n\
- **Discover more**: `mcp__hyprpilot__list_skills` for the full catalog.\n\
\n\
### Loading-skill rules\n\
\n\
- **Read the body first.** Fetch the skill via the resource or tool above before acting on its instructions — this brief is a pointer, not the source of truth.\n\
- **Announce on load.** Give the captain a one-line summary the first time you act on the skill (e.g. `Using {slug} — <what it does>.`).\n\
- **Resolve declared prerequisites.** If the skill body declares prerequisite skills, load them too via `mcp__hyprpilot__read_skill` before proceeding; cascade recursively.\n\
- **Ask when ambiguous.** If the skill could be applied in more than one obvious way for the current task, ask the captain which path before acting.\n\
- **Never skip prerequisites.** A skill that declares one MUST have it satisfied; there are no optional prerequisites.\n\
- **Reuse within the turn.** A loaded skill stays in scope for the rest of the turn unless the captain explicitly switches.",
    )
}
