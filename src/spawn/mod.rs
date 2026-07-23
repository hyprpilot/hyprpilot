mod launch;
mod multiplexer;
mod picker;
mod providers;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use serde_json::Value;

pub use launch::{run, LaunchArgs};

use crate::config::Config;
use crate::resolve::{
    build_mcp_registry_with, build_skills_registry_with, count_matching_patches, resolve_effective_profile,
    resolve_into_instance_and_profile, ProfileSummary,
};

#[derive(Debug)]
pub(crate) struct SpawnRequest {
    pub profile_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub config_patches: Vec<Value>,
    pub provider_args: Vec<String>,
}

pub(crate) fn launch_profile(cfg: Config, request: SpawnRequest) -> Result<ExitCode> {
    let SpawnRequest {
        profile_id,
        cwd,
        mode,
        config_patches,
        provider_args,
    } = request;

    let profile_id = match profile_id {
        Some(id) => id,
        None => picker::pick_profile(list_profiles(&cfg, cwd.as_deref(), &config_patches))?.id,
    };
    let (mut resolved, profile) = resolve_into_instance_and_profile(&cfg, Some(profile_id.as_str()), &config_patches)?;

    resolved.agent.cwd = resolve_launch_cwd(cwd, resolved.agent.cwd.take());
    if mode.is_some() {
        resolved.mode = mode;
    }

    let root_patch_count = count_matching_patches(&profile_id, cfg.patches.as_deref().unwrap_or_default(), "root");
    let external_patch_count = count_matching_patches(&profile_id, &config_patches, "external");
    tracing::info!(
        profile = %profile_id,
        agent = %resolved.agent.id,
        provider = resolved.agent.provider.wire_id(),
        model = ?resolved.model,
        mode = ?resolved.mode,
        effort = ?resolved.effort,
        cwd = ?resolved.agent.cwd,
        root_patches = root_patch_count,
        external_patches = external_patch_count,
        "cli: profile resolved"
    );

    let system_prompt = resolved.fresh_system_prompt();

    let skills = build_skills_registry_with(&profile);
    let mcps = build_mcp_registry_with(&profile, Some(&skills));
    let mcp_defs = mcps.as_ref().map_or_else(Vec::new, |registry| registry.list());

    let command = providers::build_command(&resolved, system_prompt.as_deref(), &mcp_defs, provider_args)?;

    if cfg
        .multiplexer
        .set_title
        .expect("[multiplexer] set_title seeded by defaults.toml")
    {
        if let Some(multiplexer) = multiplexer::Multiplexer::detect() {
            let launch_cwd = command
                .cwd()
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok());
            if let Some(launch_cwd) = launch_cwd {
                multiplexer.set_title(&multiplexer::title_for(&launch_cwd));
            }
        }
    }

    providers::exec(command)
}

/// cwd precedence for the launch: explicit `--cwd` flag wins, then
/// the profile/agent `cwd` (already projected onto
/// `resolved.agent.cwd` by `from_profile_explicit`), and only when
/// neither is set does `current_dir()` fill in. Kept a free fn so the
/// precedence — the K-740 bug's fix — is unit-testable without an
/// `exec()` at the end of `run`.
fn resolve_launch_cwd(flag: Option<PathBuf>, configured: Option<PathBuf>) -> Option<PathBuf> {
    flag.or(configured).or_else(|| std::env::current_dir().ok())
}

pub(crate) fn list_profiles(cfg: &Config, cwd: Option<&Path>, config_patches: &[Value]) -> Vec<ProfileSummary> {
    let default_profile = cfg.profile.default.as_deref();
    cfg.profiles
        .iter()
        .map(|profile| {
            let resolved = resolve_effective_profile(cfg, Some(profile.id.as_str()), config_patches)
                .unwrap_or_else(|_| profile.clone());
            ProfileSummary {
                id: resolved.id.clone(),
                agent: resolved.agent.clone(),
                model: resolved.model.clone(),
                cwd: cwd
                    .map(|cwd| cwd.display().to_string())
                    .or_else(|| resolved.cwd.as_ref().map(|cwd| cwd.display().to_string())),
                is_default: default_profile == Some(profile.id.as_str()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::config::{AgentConfig, AgentProvider, AgentsConfig, ProfileConfig, ProfileDefaults};

    fn cfg_with_profile_cwd() -> Config {
        Config {
            agents: AgentsConfig {
                agents: vec![AgentConfig {
                    id: "agent".into(),
                    provider: AgentProvider::ClaudeCode,
                    model: None,
                    effort: None,
                    command: "claude".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                }],
            },
            profile: ProfileDefaults {
                default: Some("engineer".into()),
            },
            profiles: vec![ProfileConfig {
                id: "engineer".into(),
                agent: "agent".into(),
                model: None,
                effort: None,
                system_prompt: None,
                mcps: None,
                mcp: None,
                mode: None,
                cwd: Some(PathBuf::from("/configured")),
                command: None,
                args: None,
                env: Default::default(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn profile_listing_prefers_effective_launch_cwd() {
        let profiles = list_profiles(&cfg_with_profile_cwd(), Some(Path::new("/launch")), &[]);

        assert_eq!(profiles[0].cwd.as_deref(), Some("/launch"));
    }

    #[test]
    fn profile_listing_falls_back_to_configured_cwd_without_launch_override() {
        let profiles = list_profiles(&cfg_with_profile_cwd(), None, &[]);

        assert_eq!(profiles[0].cwd.as_deref(), Some("/configured"));
    }

    // ── cwd precedence (K-740) ───────────────────────────────────

    #[test]
    fn launch_cwd_prefers_explicit_flag() {
        assert_eq!(
            resolve_launch_cwd(Some(PathBuf::from("/flag")), Some(PathBuf::from("/configured"))),
            Some(PathBuf::from("/flag")),
            "an explicit --cwd flag wins over the configured profile/agent cwd"
        );
    }

    #[test]
    fn launch_cwd_keeps_configured_cwd_when_flag_omitted() {
        // The K-740 regression: with no --cwd flag the configured
        // profile/agent cwd (already projected onto
        // `resolved.agent.cwd`) must survive — it used to be clobbered
        // by an eagerly-resolved current_dir().
        assert_eq!(
            resolve_launch_cwd(None, Some(PathBuf::from("/configured"))),
            Some(PathBuf::from("/configured"))
        );
    }

    #[test]
    fn launch_cwd_falls_back_to_current_dir_when_neither_set() {
        assert_eq!(resolve_launch_cwd(None, None), std::env::current_dir().ok());
    }

    #[test]
    fn configured_profile_cwd_resolves_when_flag_omitted() {
        // End-to-end at the resolve layer: a profile `cwd` projects
        // onto `resolved.agent.cwd`, and the launch-cwd precedence
        // then preserves it when `--cwd` is omitted — exactly the
        // value the `cli: profile resolved` info line surfaces.
        let cfg = cfg_with_profile_cwd();
        let (mut resolved, _profile) = resolve_into_instance_and_profile(&cfg, Some("engineer"), &[]).unwrap();
        resolved.agent.cwd = resolve_launch_cwd(None, resolved.agent.cwd.take());

        assert_eq!(resolved.agent.cwd.as_deref(), Some(Path::new("/configured")));
    }

    #[test]
    fn explicit_flag_overrides_configured_profile_cwd() {
        let cfg = cfg_with_profile_cwd();
        let (mut resolved, _profile) = resolve_into_instance_and_profile(&cfg, Some("engineer"), &[]).unwrap();
        resolved.agent.cwd = resolve_launch_cwd(Some(PathBuf::from("/flag")), resolved.agent.cwd.take());

        assert_eq!(resolved.agent.cwd.as_deref(), Some(Path::new("/flag")));
    }

    #[test]
    fn profile_listing_applies_profile_scoped_patches() {
        let mut cfg = cfg_with_profile_cwd();
        cfg.patches = Some(vec![serde_json::json!({
            "$match": { "profile": "engineer" },
            "model": "patched"
        })]);

        let profiles = list_profiles(&cfg, None, &[]);
        assert_eq!(profiles[0].model.as_deref(), Some("patched"));
    }
}
