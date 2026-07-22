//! Pure config→resolution core: profile resolution, root/withConfig
//! patch folding, and per-instance MCP/skills registry construction.
//! No process spawning, no ACP wire types, no daemon/RPC state — just
//! `Config` + `ProfileConfig` in, a resolved shape out.
//!
//! Consumed by the launcher (`adapters::cli`); returns
//! `anyhow::Error` so nothing here depends on a transport-specific
//! error type.

use std::sync::Arc;

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::adapters::profile::ResolvedInstance;
use crate::config::{Config, ProfileConfig};

/// Wire shape for `profiles_list` / `config/profiles` entries.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Profile-scoped cwd hint — the daemon's resolved cwd for spawns
    /// under this profile. Optional because not every profile sets
    /// one; consumers (palette `instance · new`) use it to pre-seed
    /// the chrome header's cwd pill so the captain sees the spawn
    /// target before the actor's `session/new` lands. The header
    /// later updates from `MetaSnapshot.cwd` (authoritative) when
    /// the spawn completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub is_default: bool,
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

/// Build the per-instance MCP registry from the patched profile.
///
/// Prepends an **auto-injected** entry for the in-tree `hyprpilot mcp
/// serve` server when the resolved `[mcp]` block has `enabled = true`
/// AND the per-instance skills registry (after applying the optional
/// slug whitelist) is non-empty. The daemon's resolved skill set
/// rides through to the agent vendor as a stdio MCP server it spawns
/// itself. Auto-inject is independent of user-declared `mcps` —
/// `mcps = []` does not suppress the in-tree server (that's what
/// `mcp.enabled = false` is for).
pub(crate) fn build_mcp_registry_with(
    profile: &ProfileConfig,
    skills: Option<&Arc<crate::skills::SkillsRegistry>>,
) -> Option<Arc<crate::mcp::MCPsRegistry>> {
    let mcp_cfg = effective_mcp_with(profile);
    let files = effective_mcp_files_with(profile);
    let mut defs = crate::mcp::loader::load_files(&files);
    apply_mcp_glob_defaults(&mut defs, &mcp_cfg);

    // Auto-inject only when the effective [mcp] block opts in AND
    // there's a non-empty skills registry to project. Source is a
    // synthetic path so the UI's "which file owns this server"
    // surfaces a recognisable label.
    if let Some(skills_arc) = skills {
        if mcp_cfg.enabled() {
            if let Some(auto) = crate::mcp::auto_inject::build_auto_inject_definition(
                skills_arc,
                &mcp_cfg,
                std::path::PathBuf::from("<auto-injected:hyprpilot mcp serve>"),
            ) {
                prepend_auto_mcp_definition(&mut defs, auto);
            }
        }
    }

    if defs.is_empty() {
        return None;
    }
    Some(Arc::new(crate::mcp::MCPsRegistry::new(defs)))
}

pub(crate) fn prepend_auto_mcp_definition(defs: &mut Vec<crate::mcp::MCPDefinition>, auto: crate::mcp::MCPDefinition) {
    let reserved_name = auto.name.clone();
    let before = defs.len();

    defs.retain(|def| def.name != reserved_name);
    if defs.len() != before {
        tracing::warn!(
            server = %reserved_name,
            "acp::adapter: replacing configured MCP server with reserved auto-injected server"
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

/// Resolved `[mcp]` block for an instance — reads ONLY from the
/// patched profile. Falls back to the typed `Default::default()`
/// when the profile has no `mcp` block (enabled=true,
/// autoAcceptTools=["*"], no skills).
pub(crate) fn effective_mcp_with(profile: &ProfileConfig) -> crate::config::McpConfig {
    profile.mcp.clone().unwrap_or_default()
}

/// Skills slugs the auto-injected `hyprpilot` MCP server should
/// expose for this instance. Reads from the patched profile's
/// `mcp.skills` (root `[[patches]]` already folded upstream).
fn effective_skills_with(profile: &ProfileConfig) -> Vec<crate::config::ResolvedSkillEntry> {
    effective_mcp_with(profile).resolved_skills()
}

/// Build the per-instance skills registry from the patched profile.
pub(crate) fn build_skills_registry_with(profile: &ProfileConfig) -> Arc<crate::skills::SkillsRegistry> {
    let entries = effective_skills_with(profile);
    let registry = Arc::new(crate::skills::SkillsRegistry::new(entries));
    if let Err(err) = registry.reload() {
        tracing::warn!(%err, "acp::adapter: per-instance skills initial reload failed");
    }
    registry
}

/// Single source of truth for the captain-intended `ProfileConfig`
/// at spawn time. Every consumer that needs to ask "what does the
/// captain want?" — `ResolvedInstance` builder, MCP registry
/// builder, skills registry builder, session-info shape — calls
/// this and reads from the returned profile.
///
/// Resolution order:
///   1. Pick base profile via `base_profile_for_patches` (errors
///      when neither `--profile <id>` nor `[profile] default`
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
            crate::config::patch::apply_root_patches_to_profile_with_context(base_value, rp, match_context)
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
/// `--profile <id>` first, then `[profile] default`. Errors when
/// neither addresses a real `[[profiles]]` entry — every spawn
/// flows through a profile (no bare-agent fallback). Validation at
/// config-load already rejects an empty `[[profiles]]` list, so the
/// captain's setup mistake surfaces at daemon boot rather than per
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
         Pass `--profile <id>` or set `[profile] default = '<id>'`."
    )
}

/// One-stop spawn-time resolver: pick + patch the profile, project
/// onto a `ResolvedInstance`, return both. The patched
/// `ProfileConfig` is the single source the MCP registry, skills
/// registry, and per-instance context downstream all read from.
///
/// `external_patches` is empty for plain spawn paths; the
/// `--with-config` path supplies non-empty patches that fold on top
/// of root `[[patches]]`.
///
/// Explicit `agent_id` wins over whatever agent the patched profile
/// names — captain intent for "run THIS profile but on a different
/// vendor binary".
pub(crate) fn resolve_into_instance_and_profile(
    cfg: &Config,
    agent_id: Option<&str>,
    profile_id: Option<&str>,
    external_patches: &[Value],
) -> anyhow::Result<(ResolvedInstance, ProfileConfig)> {
    let patched = resolve_effective_profile(cfg, profile_id, external_patches)?;
    let mut resolved = ResolvedInstance::from_profile_explicit(&patched, cfg)?;

    if let Some(wanted) = agent_id {
        let agent = cfg
            .agents
            .agents
            .iter()
            .find(|a| a.id == wanted)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("agent '{wanted}' not found in [[agents]] registry"))?;
        if resolved.model.is_none() || resolved.agent.id != agent.id {
            resolved.model = resolved.model.or_else(|| agent.model.clone());
        }
        resolved.agent = agent;
    }

    if resolved.agent.id.is_empty() {
        anyhow::bail!("no agent resolved — add a [[agents]] entry or pass agent_id / profile_id");
    }

    Ok((resolved, patched))
}
