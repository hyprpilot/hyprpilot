mod picker;
mod providers;
mod sessions;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use serde_json::Value;

use crate::adapters::acp::instances::{
    build_mcp_registry_with, build_skills_registry_with, resolve_effective_profile, resolve_into_instance_and_profile,
};
use crate::adapters::Bootstrap;
use crate::adapters::ProfileSummary;
use crate::config::Config;

#[derive(Debug)]
pub(crate) struct SpawnRequest {
    pub profile_id: String,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub restore: bool,
    pub session_id: Option<String>,
    pub all: bool,
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
        restore,
        session_id,
        all,
        config_patches,
        provider_args,
    } = request;

    let (mut resolved, profile) =
        resolve_into_instance_and_profile(&cfg, agent_id.as_deref(), Some(profile_id.as_str()), &config_patches)
            .map_err(|err| anyhow::anyhow!(err.message))?;

    if let Some(cwd) = cwd {
        resolved.agent.cwd = Some(cwd);
    }
    if model.is_some() {
        resolved.model = model;
    }
    if mode.is_some() {
        resolved.mode = mode;
    }

    let session_id = match session_id {
        Some(id) => Some(id),
        None if restore => {
            let sessions = sessions::list_restorable_sessions(&resolved, all)?;
            Some(picker::pick_session(sessions)?.id)
        }
        None => None,
    };
    let bootstrap = session_id
        .as_ref()
        .map_or(Bootstrap::Fresh, |id| Bootstrap::Resume(id.clone()));
    let system_prompt = resolved.system_prompt_for(&bootstrap);

    let skills = build_skills_registry_with(&profile);
    let mcps = build_mcp_registry_with(&profile, Some(&skills));
    let mcp_defs = mcps.as_ref().map_or_else(Vec::new, |registry| registry.list());

    let command = providers::build_command(
        &resolved,
        &bootstrap,
        system_prompt.as_deref(),
        &mcp_defs,
        provider_args,
    )?;

    providers::exec(command)
}

pub(crate) fn list_profiles(cfg: &Config) -> Vec<ProfileSummary> {
    let default_profile = cfg.profile.default.as_deref();
    cfg.profiles
        .iter()
        .map(|profile| {
            let resolved =
                resolve_effective_profile(cfg, Some(profile.id.as_str()), &[]).unwrap_or_else(|_| profile.clone());
            ProfileSummary {
                id: resolved.id.clone(),
                agent: resolved.agent.clone(),
                model: resolved.model.clone(),
                cwd: resolved.cwd.as_ref().map(|cwd| cwd.display().to_string()),
                is_default: default_profile == Some(profile.id.as_str()),
            }
        })
        .collect()
}
