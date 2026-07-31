//! Auto-injection of the in-tree MCP servers.
//!
//! One `build_*_definition` per server. Under the `[mcp].enabled`
//! master gate the launcher prepends a stdio entry for each server its
//! own block enables, and the vendor spawns those sidecars itself.
//! Skills is the only one ALSO gated on content — an empty
//! `SkillsRegistry` means nothing to serve — and the only one that
//! passes state on the command line (`--skill-dir <json>` per root).
//!
//! References declared in each skill's frontmatter resolve relative
//! to the skill's own bundle directory at read time — the sidecar
//! maintains no separate references-root concept.
//!
//! Each server's resolved name is what vendors prefix tool calls with
//! (`mcp__hyprpilot-skills__list_skills`, …) and is RESERVED: a
//! same-named configured server is replaced. Auto-accept rides through
//! `HyprpilotExtension.auto_accept_tools` — the server's own globs when
//! set, else the `[mcp]`-level ones (default `["*"]`), so by default
//! every tool is projected as auto-approved unless the captain has
//! tightened them. **That default applies to the harness too**: turning
//! `[mcp.harness].enabled` on without per-server globs auto-approves
//! `spawn`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::McpConfig;
use crate::mcp::{HyprpilotExtension, MCPDefinition};
use crate::skills::SkillsRegistry;

/// Build the general-tools catalog entry.
///
/// Like the harness and unlike skills, this is not gated on having
/// content to serve — the tool list is fixed, so the captain's
/// `enabled` flag is the whole gate.
#[must_use]
pub fn build_tools_definition(cfg: &McpConfig, source: PathBuf) -> Option<MCPDefinition> {
    let tools = cfg.serve.clone().unwrap_or_default();
    if !tools.is_enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": ["mcp", "serve"],
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });

    Some(MCPDefinition {
        name: tools.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            auto_accept_tools: tools
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: tools
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

/// Build the harness catalog entry.
///
/// Separate from the skills entry so the two servers get independent
/// process lifetimes and independent tool policy — auto-accepting a
/// skill read and auto-accepting `spawn` are not the same decision.
///
/// Unlike skills, this is NOT gated on having content to serve: the
/// harness always has tools. It is gated purely on the captain enabling
/// it, which is deliberate — see [`HarnessServerConfig`].
#[must_use]
pub fn build_harness_definition(cfg: &McpConfig, source: PathBuf) -> Option<MCPDefinition> {
    let harness = cfg.harness.clone().unwrap_or_default();
    if !harness.is_enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args = vec!["mcp".to_string(), "harness".to_string()];
    if let Some(max) = harness.max_sessions {
        args.push("--max-sessions".to_string());
        args.push(max.to_string());
    }
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": args,
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });

    Some(MCPDefinition {
        name: harness.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            // Per-server policy wins; otherwise the `[mcp]` default.
            auto_accept_tools: harness
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: harness
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

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
    let skills_cfg = cfg.skills.clone().unwrap_or_default();
    if !skills_cfg.is_enabled() || skills.list().is_empty() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args: Vec<String> = vec!["mcp".to_string(), "skills".to_string()];
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
        name: skills_cfg.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            auto_accept_tools: skills_cfg
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: skills_cfg
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::config::mcp::{HarnessServerConfig, ToolsServerConfig};
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

    /// The security-relevant default. `spawn` runs a profile's
    /// `command`, so a captain who never mentions the harness must not
    /// get an entry for it.
    #[test]
    fn harness_is_not_injected_by_default() {
        assert!(build_harness_definition(&default_cfg(), PathBuf::from("<test>")).is_none());
    }

    #[test]
    fn harness_is_injected_once_enabled() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>")).expect("enabled harness injects");
        assert_eq!(def.name, crate::config::mcp::DEFAULT_HARNESS_SERVER_NAME);
    }

    /// Enabling the harness without per-server globs inherits the
    /// `[mcp]`-level `["*"]`, which auto-approves `spawn`. Pinned so
    /// the consequence is a deliberate choice rather than a surprise —
    /// flip this test if the default ever tightens.
    #[test]
    fn enabling_harness_inherits_the_permissive_accept_default() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>")).expect("injects");
        assert_eq!(def.hyprpilot.auto_accept_tools, vec!["*".to_string()]);
    }

    #[test]
    fn per_server_globs_override_rather_than_merge() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                auto_accept_tools: Some(vec!["list_profiles".into()]),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>")).expect("injects");
        assert_eq!(
            def.hyprpilot.auto_accept_tools,
            vec!["list_profiles".to_string()],
            "the `[mcp]`-level `*` must not survive alongside a per-server list"
        );
    }

    #[test]
    fn tools_server_is_injected_by_default() {
        let def = build_tools_definition(&default_cfg(), PathBuf::from("<test>")).expect("serve defaults on");
        assert_eq!(def.name, crate::config::mcp::DEFAULT_TOOLS_SERVER_NAME);
    }

    #[test]
    fn disabling_a_server_skips_only_that_one() {
        let cfg = McpConfig {
            serve: Some(ToolsServerConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        assert!(build_tools_definition(&cfg, PathBuf::from("<test>")).is_none());
        assert!(build_harness_definition(&cfg, PathBuf::from("<test>")).is_some());
    }

    #[test]
    fn a_renamed_server_reserves_its_new_name() {
        let cfg = McpConfig {
            serve: Some(ToolsServerConfig {
                name: Some("mytools".into()),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_tools_definition(&cfg, PathBuf::from("<test>")).expect("injects");
        assert_eq!(def.name, "mytools");
    }
}
