//! Pure config→resolution core: profile resolution, root/withConfig
//! patch folding, and per-launch MCP/skills registry construction.
//! No process spawning — just `Config` + `ProfileConfig` in, a
//! resolved shape out.
//!
//! Consumed by the launcher (`spawn`); returns `anyhow::Error` so
//! nothing here depends on a caller-specific error type.

use std::sync::Arc;

use anyhow::Context;
use merge::Merge;
use serde::Serialize;
use serde_json::Value;

use crate::config::{Config, ProfileConfig};
use crate::profile::ResolvedProfile;

/// Summary row for one resolved profile — backs the `profiles`
/// subcommand's table / JSON output and the interactive picker.
/// `Default` exists for test fixtures: this struct is a display
/// projection that grows fields as the listing surfaces more, and
/// `..Default::default()` keeps each fixture asserting the one field it
/// cares about instead of restating every sibling.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub agent: String,
    /// The agent's vendor wire id (`claude-code` / `codex` /
    /// `opencode`). Distinct from `agent`, which is the `[[agents]]`
    /// entry id — an agent named `fast` says nothing about which CLI
    /// it drives, and the MCP harness needs the vendor to know how to
    /// read the session back.
    pub provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Profile forces a non-interactive launch. Surfaced because such a
    /// profile cannot be driven interactively, so a caller picking one
    /// must supply a prompt.
    pub headless: bool,
    /// Whether `mcp harness` may drive this profile. Read from the
    /// PATCHED profile — a `$match`ed patch is how a family opts in.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub harness_enabled: bool,
    /// How many MCP servers and skills this profile resolves to — a
    /// cheap "how equipped is this agent" signal for a caller choosing
    /// between profiles.
    pub mcp_count: usize,
    pub skills_count: usize,
    /// Profile-scoped cwd hint — the resolved launch cwd for this
    /// profile. Optional because not every profile sets one; the
    /// interactive picker surfaces it so the captain sees the launch
    /// target before committing. The actual launch cwd still follows
    /// the `--cwd` > profile/agent > current-dir precedence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub is_default: bool,
    /// Set when patch resolution failed for this profile. The listing
    /// then shows the UNPATCHED base values (model / cwd), so surface
    /// the failure — a `!` marker + this message in the table/picker —
    /// rather than passing stale data off as the resolved shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Project the patched profile's `mcps` field onto the resolved
/// runtime shape. Reads ONLY from the patched profile — root
/// `[[patches]]` were folded onto it in `resolve_effective_profile`
/// upstream.
pub(crate) fn effective_mcp_files_with(profile: &ProfileConfig) -> Vec<crate::config::ResolvedMcpFile> {
    profile
        .mcps
        .as_deref()
        .map(|files| files.iter().map(crate::config::ResolvedMcpFile::from_entry).collect())
        .unwrap_or_default()
}

/// Build the per-launch MCP registry from the patched profile.
///
/// Prepends an **auto-injected** entry for the in-tree `hyprpilot mcp
/// serve` server when the resolved `[mcp]` block has `enabled = true`
/// AND the per-launch skills registry (after applying the optional
/// slug whitelist) is non-empty. The resolved skill set rides through
/// to the agent vendor as a stdio MCP server it spawns itself.
/// Auto-inject is independent of user-declared `mcps` —
/// `mcps = []` does not suppress the in-tree server (that's what
/// `mcp.enabled = false` is for).
pub(crate) fn build_mcp_registry_with(
    profile: &ProfileConfig,
    skills: Option<&Arc<crate::mcp::skills::SkillsRegistry>>,
) -> Vec<crate::mcp::MCPDefinition> {
    let mcp_cfg = effective_mcp_with(profile);
    let files = effective_mcp_files_with(profile);
    let mut defs = crate::mcp::loader::load_files(&files);
    apply_mcp_glob_defaults(&mut defs, &mcp_cfg);

    // `[mcp].enabled` is the master gate over every in-tree server;
    // each server then decides for itself. Source is a synthetic path
    // so the "which file owns this server" surface stays recognisable.
    let mut auto_injected: Vec<&str> = Vec::new();
    if mcp_cfg.enabled() {
        if let Some(tools) = crate::mcp::auto_inject::build_tools_definition(
            &mcp_cfg,
            std::path::PathBuf::from("<auto-injected:hyprpilot mcp serve>"),
        ) {
            prepend_auto_mcp_definition(&mut defs, tools);
            auto_injected.push("serve");
        }
        if let Some(harness) = crate::mcp::auto_inject::build_harness_definition(
            &mcp_cfg,
            std::path::PathBuf::from("<auto-injected:hyprpilot mcp harness>"),
        ) {
            prepend_auto_mcp_definition(&mut defs, harness);
            auto_injected.push("harness");
        }
        // Skills last so it lands first in the list.
        if let Some(auto) = skills.and_then(|skills_arc| {
            crate::mcp::auto_inject::build_auto_inject_definition(
                skills_arc,
                &mcp_cfg,
                std::path::PathBuf::from("<auto-injected:hyprpilot mcp skills>"),
            )
        }) {
            prepend_auto_mcp_definition(&mut defs, auto);
            auto_injected.push("skills");
        }
    }

    if defs.is_empty() {
        return defs;
    }
    // `loader::load_files` + `prepend_auto_mcp_definition` already
    // apply file-iteration order and dedupe the reserved name, so the
    // returned list is collision-free and ordered — the spawn path
    // projects it onto the vendor CLI as-is.
    tracing::info!(
        servers = ?defs.iter().map(|def| def.name.as_str()).collect::<Vec<_>>(),
        ?auto_injected,
        "resolve: mcp registry built"
    );
    for def in &defs {
        tracing::debug!(server = %def.name, source = %def.source.display(), "resolve: mcp server source");
    }
    defs
}

pub(crate) fn prepend_auto_mcp_definition(defs: &mut Vec<crate::mcp::MCPDefinition>, auto: crate::mcp::MCPDefinition) {
    let reserved_name = auto.name.clone();
    let before = defs.len();

    defs.retain(|def| def.name != reserved_name);
    if defs.len() != before {
        tracing::warn!(
            server = %reserved_name,
            "resolve: replacing configured MCP server with reserved auto-injected server"
        );
    }
    defs.insert(0, auto);
}

pub(crate) fn apply_mcp_glob_defaults(defs: &mut [crate::mcp::MCPDefinition], cfg: &crate::config::McpConfig) {
    for def in defs {
        if def.hyprpilot.auto_accept_tools.is_empty() {
            def.hyprpilot.auto_accept_tools = cfg.auto_accept_tools().to_vec();
        }

        if def.hyprpilot.auto_reject_tools.is_empty() {
            def.hyprpilot.auto_reject_tools = cfg.auto_reject_tools().to_vec();
        }
    }
}

/// Resolved `[mcp]` block for a launch — reads ONLY from the patched
/// profile, backfilled onto `McpConfig::default()`.
///
/// The patched profile's `mcp` (root `[[patches]]` already folded
/// upstream) overlays the typed default per-leaf via `Merge`
/// (`overwrite_some`): a `Some` leaf wins, an unset leaf inherits the
/// default. This guarantees the value leaves the `.expect()`
/// accessors read (`enabled` / `autoAcceptTools` / `autoRejectTools`)
/// are always populated even when a patch replaced `mcp` wholesale
/// with a partial block or cleared it, while the seeded skills dir —
/// carried on the patched profile — still wins. No profile `mcp` at
/// all → the bare default (enabled, `["*"]`, no skills).
pub(crate) fn effective_mcp_with(profile: &ProfileConfig) -> crate::config::McpConfig {
    let mut cfg = crate::config::McpConfig::default();
    if let Some(profile_mcp) = profile.mcp.clone() {
        cfg.merge(profile_mcp);
    }
    cfg
}

/// Skills slugs the auto-injected `hyprpilot` MCP server should
/// expose for this launch. Reads from the patched profile's
/// `mcp.skills` (root `[[patches]]` already folded upstream).
fn effective_skills_with(profile: &ProfileConfig) -> Vec<crate::config::ResolvedSkillEntry> {
    effective_mcp_with(profile).resolved_skills()
}

/// Build the per-launch skills registry from the patched profile.
pub(crate) fn build_skills_registry_with(profile: &ProfileConfig) -> Arc<crate::mcp::skills::SkillsRegistry> {
    let entries = effective_skills_with(profile);
    let dir_count = entries.len();
    let registry = Arc::new(crate::mcp::skills::SkillsRegistry::new(entries));
    if let Err(err) = registry.reload() {
        tracing::warn!(%err, "resolve: skills initial reload failed");
    }
    let skills = registry.list();
    tracing::info!(
        skills = skills.len(),
        dirs = dir_count,
        "resolve: skills registry built"
    );
    tracing::debug!(
        slugs = ?skills.iter().map(|skill| skill.slug.as_str()).collect::<Vec<_>>(),
        "resolve: skills registry slugs"
    );
    registry
}

/// Count the `patches` that fold onto `profile_id`, tracing each
/// patch's `$match` decision at `debug`. Pure diagnostics for the
/// "profile resolved" info line — the authoritative fold happens in
/// [`resolve_effective_profile`]; `kind` labels the surface
/// (`"root"` / `"external"`).
pub(crate) fn count_matching_patches(profile_id: &str, patches: &[Value], kind: &str) -> usize {
    let ctx = crate::config::patch::PatchMatchContext::new(profile_id);
    let mut matched = 0;
    for (index, patch) in patches.iter().enumerate() {
        let applies = crate::config::patch::patch_applies(patch, ctx);
        if applies {
            matched += 1;
        }
        tracing::debug!(
            kind,
            index,
            applies,
            match_filter = ?patch.get("$match"),
            "resolve: patch $match evaluated"
        );
    }
    matched
}

/// Single source of truth for the captain-intended `ProfileConfig`
/// at spawn time. Every consumer that needs to ask "what does the
/// captain want?" — `ResolvedProfile` builder, MCP registry
/// builder, skills registry builder, session-info shape — calls
/// this and reads from the returned profile.
///
/// Resolution order:
///   1. Pick base profile via `base_profile_for_patches` (errors
///      when neither the positional `[PROFILE]` id nor `[profile] default`
///      addresses a real `[[profiles]]` entry).
///   2. Fold root `[[patches]]` from the captain's on-disk config,
///      filtered by each patch's optional `$match.profile` glob.
///   3. Fold `external_patches` in declaration order (the
///      `--with-config` per-invocation overrides) with the same
///      match context. Empty slice is a no-op.
///   4. Deserialize back to `ProfileConfig` + re-run garde
///      validation against the post-merge shape.
pub(crate) fn resolve_effective_profile(
    cfg: &Config,
    profile_id: Option<&str>,
    external_patches: &[Value],
) -> anyhow::Result<ProfileConfig> {
    let base = base_profile_for_patches(cfg, profile_id)?;
    let base_value = serde_json::to_value(&base).context("profile serialize failed")?;
    let match_context = crate::config::patch::PatchMatchContext::new(&base.id);

    let with_root = match cfg.patches.as_deref() {
        Some(rp) if !rp.is_empty() => {
            crate::config::patch::apply_profile_patches_with_context(base_value, rp, match_context)
        }
        _ => base_value,
    };

    let merged = if external_patches.is_empty() {
        with_root
    } else {
        // Per-invocation `withConfig` patches use the same
        // profile-shaped patch vocabulary as root `[[patches]]`,
        // including top-level `$match`. Strip/filter that directive
        // here before deserialising back into `ProfileConfig`; if it
        // leaks through serde reports `unknown field "$match"`.
        crate::config::patch::apply_profile_patches_with_context(with_root, external_patches, match_context)
    };

    let patched: ProfileConfig =
        serde_json::from_value(merged).context("profile resolution: invalid shape after patches")?;
    garde::Validate::validate(&patched).context("profile resolution: validation failed")?;
    Ok(patched)
}

/// Pick the base `ProfileConfig` patches will fold onto. Resolves
/// the positional `[PROFILE]` id first, then `[profile] default`. Errors when
/// neither addresses a real `[[profiles]]` entry — every spawn
/// flows through a profile (no bare-agent fallback). Validation at
/// config-load already rejects an empty `[[profiles]]` list, so the
/// captain's setup mistake surfaces at config-load rather than per
/// `--with-config` invocation.
fn base_profile_for_patches(cfg: &Config, profile_id: Option<&str>) -> anyhow::Result<ProfileConfig> {
    if let Some(id) = profile_id {
        if let Some(p) = cfg.profiles.iter().find(|p| p.id == id) {
            return Ok(p.clone());
        }
        anyhow::bail!("profile '{id}' not found in [[profiles]] registry");
    }
    if let Some(default_id) = cfg.profile.default.as_deref() {
        if let Some(p) = cfg.profiles.iter().find(|p| p.id == default_id) {
            return Ok(p.clone());
        }
        anyhow::bail!("[profile] default = '{default_id}' but no matching [[profiles]] entry exists");
    }
    anyhow::bail!(
        "no profile addressed and no `[profile] default` configured — every spawn requires a `[[profiles]]` entry. \
         Pass the profile as the positional `hyprpilot <id>` argument or set `[profile] default = '<id>'`."
    )
}

/// One-stop launch-time resolver: pick + patch the profile, project
/// onto a `ResolvedProfile`, return both. The patched
/// `ProfileConfig` is the single source the MCP registry, skills
/// registry, and per-launch context downstream all read from.
///
/// `external_patches` is empty for plain launch paths; the
/// `--with-config` path supplies non-empty patches that fold on top
/// of root `[[patches]]`. Every knob — agent, model, mode — comes
/// from the resolved profile; there is no per-launch agent/model
/// override (`--with-config` is the ad-hoc escape hatch).
pub(crate) fn resolve_into_instance_and_profile(
    cfg: &Config,
    profile_id: Option<&str>,
    external_patches: &[Value],
) -> anyhow::Result<(ResolvedProfile, ProfileConfig)> {
    let patched = resolve_effective_profile(cfg, profile_id, external_patches)?;
    let resolved = ResolvedProfile::from_profile_explicit(&patched, cfg)?;
    Ok((resolved, patched))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::McpConfig;
    use crate::mcp::{HyprpilotExtension, MCPDefinition};

    fn bare_profile() -> ProfileConfig {
        serde_json::from_value(json!({ "id": "p", "agent": "a" })).expect("minimal profile deserializes")
    }

    /// `[mcp].enabled = false` is the one switch that must silence
    /// every in-tree server at once — per-server `enabled` flags do not
    /// get a vote above it.
    #[test]
    fn master_gate_off_injects_nothing() {
        let mut profile = bare_profile();
        profile.mcp = Some(McpConfig {
            enabled: Some(false),
            harness: Some(crate::config::mcp::HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            serve: Some(crate::config::mcp::ToolsServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        });

        assert!(build_mcp_registry_with(&profile, None).is_empty());
    }

    /// With no `mcp` block at all the typed defaults apply: general
    /// tools on, harness off.
    #[test]
    fn defaults_inject_tools_but_not_harness() {
        let names: Vec<_> = build_mcp_registry_with(&bare_profile(), None)
            .into_iter()
            .map(|d| d.name)
            .collect();

        assert_eq!(names, vec![crate::config::mcp::DEFAULT_TOOLS_SERVER_NAME.to_string()]);
    }

    fn cfg_with_globs(accept: &[&str], reject: &[&str]) -> McpConfig {
        McpConfig {
            enabled: Some(true),
            auto_accept_tools: Some(accept.iter().map(|s| s.to_string()).collect()),
            auto_reject_tools: Some(reject.iter().map(|s| s.to_string()).collect()),
            ..McpConfig::default()
        }
    }

    fn def(accept: Vec<String>, reject: Vec<String>) -> MCPDefinition {
        MCPDefinition {
            name: "srv".into(),
            raw: json!({ "command": "srv" }),
            hyprpilot: HyprpilotExtension {
                include_tools: None,
                exclude_tools: Vec::new(),
                auto_accept_tools: accept,
                auto_reject_tools: reject,
            },
            source: "<test>".into(),
        }
    }

    #[test]
    fn glob_defaults_copied_onto_defs_without_per_server_policy() {
        let cfg = cfg_with_globs(&["read_*"], &["delete_*"]);
        let mut defs = vec![def(Vec::new(), Vec::new())];
        apply_mcp_glob_defaults(&mut defs, &cfg);

        assert_eq!(defs[0].hyprpilot.auto_accept_tools, vec!["read_*".to_string()]);
        assert_eq!(defs[0].hyprpilot.auto_reject_tools, vec!["delete_*".to_string()]);
    }

    #[test]
    fn glob_defaults_preserve_existing_per_server_overrides() {
        let cfg = cfg_with_globs(&["read_*"], &["delete_*"]);
        let mut defs = vec![def(vec!["custom_*".into()], vec!["nuke_*".into()])];
        apply_mcp_glob_defaults(&mut defs, &cfg);

        assert_eq!(defs[0].hyprpilot.auto_accept_tools, vec!["custom_*".to_string()]);
        assert_eq!(defs[0].hyprpilot.auto_reject_tools, vec!["nuke_*".to_string()]);
    }

    #[test]
    fn glob_defaults_fill_only_the_empty_axis() {
        // Accept already set, reject empty → only reject gets the
        // default; the set accept axis is left untouched.
        let cfg = cfg_with_globs(&["read_*"], &["delete_*"]);
        let mut defs = vec![def(vec!["custom_*".into()], Vec::new())];
        apply_mcp_glob_defaults(&mut defs, &cfg);

        assert_eq!(defs[0].hyprpilot.auto_accept_tools, vec!["custom_*".to_string()]);
        assert_eq!(defs[0].hyprpilot.auto_reject_tools, vec!["delete_*".to_string()]);
    }

    #[test]
    fn count_matching_patches_counts_only_applicable_patches() {
        let patches = vec![
            json!({ "model": "a" }),
            json!({ "$match": { "profile": "work/*" }, "model": "b" }),
            json!({ "$match": { "profile": "personal/*" }, "model": "c" }),
        ];

        assert_eq!(count_matching_patches("personal/claude/opus", &patches, "test"), 2);
        assert_eq!(count_matching_patches("work/claude/opus", &patches, "test"), 2);
        assert_eq!(count_matching_patches("other/id", &patches, "test"), 1);
    }
}
