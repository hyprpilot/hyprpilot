//! Auto-injection of the hyprpilot-hosted in-tree MCP server.
//!
//! When a launch's effective `[mcp]` config has `enabled = true` AND
//! its `[[mcp.skills]]` catalog resolves to a non-empty
//! `SkillsRegistry`, the launcher prepends a single stdio MCP entry to
//! the catalog it hands the vendor. That entry spawns `hyprpilot mcp
//! serve` as a sidecar; the sidecar reads the skills straight from
//! disk via the `--skill-dir <json>` args the launcher passes.
//!
//! References declared in each skill's frontmatter resolve relative
//! to the skill's own bundle directory at read time — the sidecar
//! maintains no separate references-root concept.
//!
//! Server name is **`hyprpilot`** — the same name vendors prefix tool
//! calls with (`mcp__hyprpilot__list_skills`, …). Auto-accept rides
//! through `HyprpilotExtension.auto_accept_tools` from the resolved
//! `McpConfig` (default `["*"]`), so every tool on this server is
//! projected as auto-approved onto the vendor's approval surface
//! unless the captain has tightened the globs.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::McpConfig;
use crate::mcp::{HyprpilotExtension, MCPDefinition};
use crate::skills::SkillsRegistry;

/// The single server name the sidecar registers under. Load-bearing
/// because downstream tool-name attribution
/// (`mcp__hyprpilot__list_skills`) keys off the same constant.
pub const SKILLS_SERVER_NAME: &str = "hyprpilot";

/// Build the catalog entry the launcher prepends to the vendor's MCP
/// config.
///
/// Returns `None` when the registry is empty — auto-inject is gated
/// on having something to serve. The caller is responsible for the
/// `enabled` gate (see `effective_mcp_with` in
/// `resolve/mod.rs`); this builder assumes the captain
/// wants the server when called.
///
/// `cfg.auto_accept_tools` / `cfg.auto_reject_tools` ride through to
/// the projected entry's `HyprpilotExtension` namespace so the
/// per-server tool policy handles them uniformly with user-declared
/// `mcps`.
///
/// `raw` is constructed in the **user-input** JSON shape (matching
/// what `mcpServers[name]` carries on disk) — `project_transport`
/// re-projects from the user shape so it expects `command: <string>`
/// and `env: { K: V }`.
#[must_use]
pub fn build_auto_inject_definition(
    skills: &Arc<SkillsRegistry>,
    cfg: &McpConfig,
    source: PathBuf,
) -> Option<MCPDefinition> {
    // Gate on dirs having at least one loaded skill — if the
    // directories are empty or all skills match the ignore globs,
    // there's nothing to serve.
    if skills.list().is_empty() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args: Vec<String> = vec!["mcp".to_string(), "serve".to_string()];
    // Pass directories + de-duplicated ignore globs instead of
    // enumerating individual `--skill slug=path` entries. The sidecar
    // scans dirs with the same `SkillsRegistry` discovery code the
    // launcher uses — adding a new `<slug>/SKILL.md` to a configured
    // directory is immediately visible on the next `reload` without
    // restarting the session.
    // Each directory is serialized as a JSON object so per-dir ignore
    // lists survive the CLI round-trip without flattening — the sidecar
    // can reconstruct the exact same `ResolvedSkillEntry` set the
    // launcher computed, with each root's suppression applied only to
    // that root's discoveries.
    for entry in skills.dirs() {
        let json = serde_json::json!({
            "dir": entry.dir.display().to_string(),
            "ignore": entry.ignore_patterns,
        });
        args.push("--skill-dir".to_string());
        args.push(json.to_string());
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
            include_tools: None,
            exclude_tools: Vec::new(),
            auto_accept_tools: cfg.auto_accept_tools().to_vec(),
            auto_reject_tools: cfg.auto_reject_tools().to_vec(),
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

    fn default_cfg() -> McpConfig {
        McpConfig::default()
    }

    #[test]
    fn empty_registry_skips_injection() {
        assert!(build_auto_inject_definition(&empty_registry(), &default_cfg(), PathBuf::from("<test>")).is_none());
    }
}
