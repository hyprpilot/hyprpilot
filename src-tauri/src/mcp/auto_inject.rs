//! Auto-injection of the hyprpilot-hosted in-tree MCP server.
//!
//! When an instance's resolved skills registry is non-empty, the daemon
//! prepends a single stdio MCP entry to the `mcp_servers` array it
//! passes to `session/new` / `session/load`. That entry spawns
//! `hyprpilot mcp serve` as a sidecar; the sidecar reads the skills
//! straight from disk via paths the daemon passes as repeated `--skill
//! <slug>=<path>` args. References declared in each skill's frontmatter
//! resolve relative to the skill's own bundle directory at read time —
//! the sidecar maintains no separate references-root concept.
//!
//! Server name on the wire is **`hyprpilot`** — the same name the
//! palette autocomplete embeds (`#{hyprpilot://skills/<slug>}`) and
//! the same name vendors will prefix tool calls with
//! (`mcp__hyprpilot__list_skills`, …). Permission auto-accept rides
//! through `HyprpilotExtension.auto_accept_tools = ["*"]` so every
//! `tools/call` against this server short-circuits at lane 2 to
//! `Decision::Allow`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::mcp::{HyprpilotExtension, MCPDefinition};
use crate::skills::SkillsRegistry;

/// The single server name the sidecar registers under. Load-bearing
/// because the palette autocomplete (`#{hyprpilot://skills/<slug>}`)
/// and downstream tool-name attribution
/// (`mcp__hyprpilot__list_skills`) both key off the same constant.
pub const SKILLS_SERVER_NAME: &str = "hyprpilot";

/// Build the manifest entry the daemon prepends to `mcp_servers` for
/// `session/new` / `session/load` when the instance's skills registry
/// is non-empty. Returns `None` when there are no skills resolved for
/// this instance — auto-inject is gated on having something to serve.
///
/// `raw` is constructed in the **user-input** JSON shape (matching
/// what `mcpServers[name]` carries on disk) rather than the ACP wire
/// shape — `project_to_acp` re-projects from the user shape so it
/// expects `command: <string>` and `env: { K: V }`.
#[must_use]
pub fn build_auto_inject_definition(skills: &Arc<SkillsRegistry>, source: PathBuf) -> Option<MCPDefinition> {
    let entries = skills.list();
    if entries.is_empty() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args: Vec<String> = vec!["mcp".to_string(), "serve".to_string()];
    for entry in &entries {
        args.push("--skill".to_string());
        args.push(format!("{}={}", entry.slug.as_str(), entry.path.display()));
    }
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": args,
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });
    Some(MCPDefinition {
        name: SKILLS_SERVER_NAME.to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            auto_accept_tools: vec!["*".to_string()],
            auto_reject_tools: Vec::new(),
        },
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::skills::SkillsRegistry;

    use super::*;

    fn empty_registry() -> Arc<SkillsRegistry> {
        Arc::new(SkillsRegistry::new(Vec::new()))
    }

    #[test]
    fn empty_registry_skips_injection() {
        assert!(build_auto_inject_definition(&empty_registry(), PathBuf::from("<test>")).is_none());
    }
}
