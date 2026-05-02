//! Profile vocabulary — re-exports the config-side `AgentConfig` /
//! `ProfileConfig` / `AgentProvider` types the adapter layer consumes,
//! plus the flat `ResolvedInstance` view built by resolving a
//! `(Config, profile_id?)` pair.
//!
//! The types themselves stay declared in `config::` because the TOML
//! deserialize + garde-validate wiring belongs with the rest of the
//! config tree. Re-exports here keep the adapter surface symmetric —
//! callers reach for `adapters::profile::ProfileConfig`, never
//! `config::ProfileConfig`, when operating at the adapter layer.

pub use crate::config::{AgentConfig, AgentProvider, ProfileConfig};

use anyhow::{bail, Context, Result};

use crate::config::Config;

/// Flat, runtime-ready view of an agent + its profile overlay. The
/// adapter takes this (not a raw `Config`) so the actor body never
/// reaches back into the layered config tree.
///
/// Model precedence: profile > agent > vendor default (the vendor
/// default is applied lazily at spawn time when `model` is `None`).
/// The system prompt is read from disk at resolve time, not at spawn
/// time, so a missing file surfaces as a readable error on the
/// submit path rather than inside the actor.
#[derive(Debug, Clone)]
pub struct ResolvedInstance {
    pub agent: AgentConfig,
    pub profile_id: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    /// Per-instance mode override. Populated from `SpawnSpec::mode`
    /// at resolve time (future K-275 will also let a `[[profiles]]`
    /// entry set a default). Generic layer just carries it; ACP's
    /// runtime passes it into `AcpInstance` and surfaces it via
    /// `InstanceInfo`. Vendor-specific interpretation (e.g.
    /// claude-code's `plan` / `edit`) happens inside the vendor
    /// agent impl.
    pub mode: Option<String>,
}

impl ResolvedInstance {
    /// Resolve the active agent + profile overlay for a submit call.
    /// `profile_id` — when `Some` — must name a real profile; when
    /// `None`, falls back through `[profile] default` and finally
    /// to a bare-agent resolution.
    pub fn from_config(config: &Config, profile_id: Option<&str>) -> Result<Self> {
        if let Some(id) = profile_id {
            return Self::from_profile(config, id);
        }
        if let Some(id) = config.profile.default.as_deref() {
            return Self::from_profile(config, id);
        }
        Self::bare(config)
    }

    fn from_profile(config: &Config, profile_id: &str) -> Result<Self> {
        let profile = config
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .with_context(|| format!("profile '{profile_id}' not found in [[profiles]] registry"))?;

        let agent = config
            .agents
            .agents
            .iter()
            .find(|a| a.id == profile.agent)
            .with_context(|| {
                format!(
                    "profile '{}' references agent '{}' but no matching [[agents]] entry exists",
                    profile.id, profile.agent
                )
            })?;

        let model = profile.model.clone().or_else(|| agent.model.clone());
        let system_prompt = Self::load_system_prompt(profile, config.system_prompt.as_deref())?;

        Ok(Self {
            agent: agent.clone(),
            profile_id: Some(profile.id.clone()),
            model,
            system_prompt,
            mode: None,
        })
    }

    fn bare(config: &Config) -> Result<Self> {
        let agents = &config.agents.agents;
        if agents.is_empty() {
            bail!("no agents configured — add a [[agents]] entry or use --profile");
        }
        let agent = config
            .agents
            .agent
            .default
            .as_deref()
            .and_then(|wanted| agents.iter().find(|a| a.id == wanted))
            .unwrap_or(&agents[0]);
        Ok(Self {
            agent: agent.clone(),
            profile_id: None,
            model: agent.model.clone(),
            system_prompt: config.system_prompt.clone(),
            mode: None,
        })
    }

    /// Resolve the system prompt for a profile. Precedence:
    ///   1. `[[profiles]] system_prompt` (inline string)
    ///   2. `[[profiles]] system_prompt_file` (file path read at resolve time)
    ///   3. root `system_prompt` (global fallback, mirrors `mcps`)
    fn load_system_prompt(profile: &ProfileConfig, root_fallback: Option<&str>) -> Result<Option<String>> {
        if let Some(text) = &profile.system_prompt {
            return Ok(Some(text.clone()));
        }
        if let Some(path) = &profile.system_prompt_file {
            let expanded = shellexpand::tilde(&path.to_string_lossy()).into_owned();
            let contents = std::fs::read_to_string(&expanded).with_context(|| {
                format!(
                    "profile '{}': failed to read system_prompt_file {}",
                    profile.id, expanded
                )
            })?;
            return Ok(Some(contents));
        }
        Ok(root_fallback.map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;
    use crate::config::AgentsConfig;

    fn agent(id: &str, model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: id.into(),
            provider: AgentProvider::AcpClaudeCode,
            model: model.map(|s| s.to_string()),
            command: Some("/bin/false".into()),
            args: vec![],
            cwd: None,
            env: Default::default(),
        }
    }

    fn profile(id: &str, agent: &str, model: Option<&str>, prompt: Option<&str>) -> ProfileConfig {
        ProfileConfig {
            id: id.into(),
            agent: agent.into(),
            model: model.map(|s| s.to_string()),
            system_prompt: prompt.map(|s| s.to_string()),
            system_prompt_file: None,
            mcps: None,
            skills: None,
            mode: None,
            cwd: None,
            env: Default::default(),
        }
    }

    #[test]
    fn profile_model_overrides_agent_model() {
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", Some("sonnet"))],
                ..Default::default()
            },
            profiles: vec![profile("strict", "cc", Some("opus-4"), None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("strict")).unwrap();
        assert_eq!(r.agent.id, "cc");
        assert_eq!(r.model.as_deref(), Some("opus-4"));
    }

    #[test]
    fn profile_model_absent_uses_agent_model() {
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", Some("sonnet"))],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(r.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn system_prompt_file_read_at_resolve_time() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("plan.md");
        let mut f = std::fs::File::create(&prompt_path).unwrap();
        write!(f, "You are a planner.").unwrap();

        let mut p = profile("plan", "cc", None, None);
        p.system_prompt_file = Some(prompt_path.clone());

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };

        let r = ResolvedInstance::from_config(&cfg, Some("plan")).unwrap();
        assert_eq!(r.system_prompt.as_deref(), Some("You are a planner."));
    }

    #[test]
    fn system_prompt_file_missing_errors() {
        let mut p = profile("plan", "cc", None, None);
        p.system_prompt_file = Some(PathBuf::from("/nonexistent/hyprpilot-test-never.md"));
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let err = ResolvedInstance::from_config(&cfg, Some("plan")).expect_err("missing file fails");
        let msg = format!("{err:#}");
        assert!(msg.contains("plan"), "{msg}");
        assert!(msg.contains("system_prompt_file"), "{msg}");
    }

    #[test]
    fn falls_back_to_default_profile_then_bare_agent() {
        let mut cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", Some("sonnet"))],
                agent: crate::config::AgentDefaults {
                    default: Some("cc".into()),
                },
            },
            profile: crate::config::ProfileDefaults {
                default: Some("ask".into()),
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, None).unwrap();
        assert_eq!(r.profile_id.as_deref(), Some("ask"));
        assert_eq!(r.model.as_deref(), Some("sonnet"));

        cfg.profile.default = None;
        let r = ResolvedInstance::from_config(&cfg, None).unwrap();
        assert!(r.profile_id.is_none());
        assert_eq!(r.agent.id, "cc");
        assert_eq!(r.model.as_deref(), Some("sonnet"));
        assert!(r.system_prompt.is_none());
    }

    #[test]
    fn unknown_profile_id_errors() {
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![],
            ..Default::default()
        };
        let err = ResolvedInstance::from_config(&cfg, Some("ghost")).expect_err("unknown profile");
        assert!(err.to_string().contains("profile 'ghost' not found"));
    }

    #[test]
    fn root_system_prompt_falls_back_when_profile_has_neither() {
        // Profile has no inline `system_prompt` and no
        // `system_prompt_file` — falls back to the root
        // `system_prompt` declared at TOML root, mirroring how
        // `mcps` works.
        let cfg = Config {
            system_prompt: Some("global fallback prompt".into()),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(r.system_prompt.as_deref(), Some("global fallback prompt"));
    }

    #[test]
    fn profile_system_prompt_wins_over_root_fallback() {
        // Per-profile inline value beats the root fallback. Same
        // precedence as `mcps`.
        let cfg = Config {
            system_prompt: Some("global fallback".into()),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, Some("profile-specific"))],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(r.system_prompt.as_deref(), Some("profile-specific"));
    }

    #[test]
    fn root_system_prompt_carries_through_bare_resolution() {
        // No profile → bare-agent path. Root system_prompt still
        // applies so unprofiled submits get the global default.
        let cfg = Config {
            system_prompt: Some("bare-path fallback".into()),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                agent: crate::config::AgentDefaults {
                    default: Some("cc".into()),
                },
            },
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, None).unwrap();
        assert!(r.profile_id.is_none());
        assert_eq!(r.system_prompt.as_deref(), Some("bare-path fallback"));
    }
}
