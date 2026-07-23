mod multiplexer;
mod picker;
mod providers;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use serde_json::Value;

use crate::adapters::ProfileSummary;
use crate::config::Config;
use crate::resolve::{
    build_mcp_registry_with, build_skills_registry_with, resolve_effective_profile, resolve_into_instance_and_profile,
};

#[derive(Debug)]
pub(crate) struct SpawnRequest {
    pub profile_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub config_patches: Vec<Value>,
    pub provider_args: Vec<String>,
}

pub(crate) fn run(cfg: Config, request: SpawnRequest) -> Result<ExitCode> {
    let SpawnRequest {
        profile_id,
        agent_id,
        cwd,
        mode,
        model,
        config_patches,
        provider_args,
    } = request;

    let profile_id = match profile_id {
        Some(id) => id,
        None => picker::pick_profile(list_profiles(&cfg, cwd.as_deref(), &config_patches))?.id,
    };
    let (mut resolved, profile) =
        resolve_into_instance_and_profile(&cfg, agent_id.as_deref(), Some(profile_id.as_str()), &config_patches)?;

    if let Some(cwd) = cwd {
        resolved.agent.cwd = Some(cwd);
    }
    if model.is_some() {
        resolved.model = model;
    }
    if mode.is_some() {
        resolved.mode = mode;
    }

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
