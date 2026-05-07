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

pub use crate::config::{AgentConfig, ProfileConfig};

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
    /// Resolved per-entry system-prompt list. Each entry carries its
    /// own pre-read body + inject toggles; the actor filters the
    /// list against the bootstrap variant (Fresh vs Resume) at spawn
    /// time and concatenates the surviving entries. `Some(vec![])` /
    /// `None` both mean "no prompt".
    pub system_prompt: Vec<ResolvedSystemPromptEntry>,
    /// Per-instance mode override. Populated from `SpawnSpec::mode`
    /// at resolve time (future K-275 will also let a `[[profiles]]`
    /// entry set a default). Generic layer just carries it; ACP's
    /// runtime passes it into `AcpInstance` and surfaces it via
    /// `InstanceInfo`. Vendor-specific interpretation (e.g.
    /// claude-code's `plan` / `edit`) happens inside the vendor
    /// agent impl.
    pub mode: Option<String>,
}

/// One pre-read system-prompt entry — body content + the per-entry
/// inject toggles the daemon honours per bootstrap path. The actor
/// filters its `Vec<Self>` against the live bootstrap variant
/// (Fresh / Resume) and concatenates the surviving bodies.
#[derive(Debug, Clone)]
pub struct ResolvedSystemPromptEntry {
    pub body: String,
    pub file: std::path::PathBuf,
    pub inject: crate::config::SystemPromptInject,
}

impl ResolvedInstance {
    /// Filter `system_prompt` entries against the bootstrap variant
    /// and concatenate the surviving bodies with a blank-line
    /// separator. Returns `None` when no entry qualifies (no entries
    /// configured, or every entry's inject toggle is off for this
    /// path). Production path: the actor calls this at spawn time
    /// so the per-entry inject toggles actually gate injection.
    pub fn system_prompt_for(&self, bootstrap: &crate::adapters::Bootstrap) -> Option<String> {
        use crate::adapters::Bootstrap;
        let bodies: Vec<&str> = self
            .system_prompt
            .iter()
            .filter(|e| match bootstrap {
                Bootstrap::Fresh => e.inject.on_create,
                Bootstrap::Resume(_) => e.inject.on_update,
                Bootstrap::ListOnly => false,
            })
            .map(|e| e.body.as_str())
            .collect();
        if bodies.is_empty() {
            None
        } else {
            Some(bodies.join("\n\n"))
        }
    }

    /// Files whose inject toggle qualifies for the given bootstrap
    /// path — the captain-facing list the "system prompt attached"
    /// banner reads. Mirrors `system_prompt_for`'s filter.
    pub fn system_prompt_files_for(&self, bootstrap: &crate::adapters::Bootstrap) -> Vec<std::path::PathBuf> {
        use crate::adapters::Bootstrap;
        self.system_prompt
            .iter()
            .filter(|e| match bootstrap {
                Bootstrap::Fresh => e.inject.on_create,
                Bootstrap::Resume(_) => e.inject.on_update,
                Bootstrap::ListOnly => false,
            })
            .map(|e| e.file.clone())
            .collect()
    }
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
        let system_prompt = Self::load_system_prompt(profile, config)?;

        // Merge profile.env onto a clone of the agent so the spawn
        // path (which iterates `entry.env`) sees both. Profile entries
        // override agent entries on key collision — profile is the
        // more specific scope. `${VAR}` interpolation against the
        // daemon's process env happens later in `agents/mod.rs::expand_value`.
        let mut agent = agent.clone();

        for (k, v) in profile.env.iter() {
            agent.env.insert(k.clone(), v.clone());
        }

        let budget = profile.thinking_budget_tokens.or(agent.thinking_budget_tokens);

        apply_thinking_budget(&mut agent, budget);

        Ok(Self {
            agent,
            profile_id: Some(profile.id.clone()),
            model,
            system_prompt,
            mode: profile.mode.clone(),
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
        let mut agent = agent.clone();
        let model = agent.model.clone();
        let budget = agent.thinking_budget_tokens;

        apply_thinking_budget(&mut agent, budget);
        Ok(Self {
            agent,
            profile_id: None,
            model,
            system_prompt: load_root_system_prompt(config)?,
            mode: None,
        })
    }

    /// Resolve the system prompt for a profile. Profile-level
    /// `system_prompt` wholesale-replaces the root `system_prompt`;
    /// both `None` means no prompt. Each entry's body is read in
    /// list order at resolve time; the actor filters the list
    /// against the bootstrap variant at spawn time and concatenates
    /// the surviving bodies with a blank-line separator. `system_prompt
    /// = []` is the explicit off-switch and resolves to an empty list.
    fn load_system_prompt(profile: &ProfileConfig, config: &Config) -> Result<Vec<ResolvedSystemPromptEntry>> {
        if let Some(entries) = &profile.system_prompt {
            return read_prompt_entries(entries, &format!("profile '{}'", profile.id));
        }
        load_root_system_prompt(config)
    }
}

/// Translate `thinking_budget_tokens` onto the spawned process env.
/// Provider-specific dispatch (only `acp-claude-code` consumes this
/// today) keeps the knob ergonomic at the config layer instead of
/// forcing captains to memorise the magic env-var name.
///
/// Default for `acp-claude-code` when the captain leaves it unset:
/// `MAX_THINKING_TOKENS=10000`. Per Anthropic's docs, Claude Opus
/// 4.7+ default `thinking.display = "omitted"` — thinking is BILLED
/// either way, but chunks arrive with empty text unless an explicit
/// budget tells the SDK to pass `display: "showing"`. Defaulting to
/// visible costs nothing and surfaces the reasoning the captain is
/// already paying for.
///
/// `Some(0)` is the explicit "no thinking" off-switch — clears any
/// existing env entry so the SDK falls back to its own default.
///
/// User-provided `MAX_THINKING_TOKENS` in `[profiles.env]` /
/// `[agents.env]` always wins — we only fill the slot when the
/// captain hasn't already set it.
fn apply_thinking_budget(agent: &mut AgentConfig, budget: Option<u32>) {
    use crate::config::AgentProvider;
    if !matches!(agent.provider, AgentProvider::AcpClaudeCode) {
        return;
    }
    if agent.env.contains_key("MAX_THINKING_TOKENS") {
        return;
    }
    let resolved = match budget {
        Some(0) => return,
        Some(n) => n,
        None => 10_000,
    };
    agent.env.insert("MAX_THINKING_TOKENS".into(), resolved.to_string());
}

fn load_root_system_prompt(config: &Config) -> Result<Vec<ResolvedSystemPromptEntry>> {
    match &config.system_prompt {
        Some(entries) => read_prompt_entries(entries, "root"),
        None => Ok(Vec::new()),
    }
}

/// Read every entry's file body and pair it with the entry's
/// inject toggles. Each path is `~`/env-expanded; missing files
/// surface as readable errors stamped with `ctx_label`. Empty list
/// returns an empty Vec (the explicit off-switch shape).
fn read_prompt_entries(
    entries: &[crate::config::SystemPromptEntry],
    ctx_label: &str,
) -> Result<Vec<ResolvedSystemPromptEntry>> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let expanded = crate::paths::resolve_user(&entry.file.to_string_lossy());
        let body = std::fs::read_to_string(&expanded)
            .with_context(|| format!("{ctx_label}: failed to read system_prompt {}", expanded.display()))?;
        out.push(ResolvedSystemPromptEntry {
            body,
            file: expanded,
            inject: entry.inject.clone(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;
    use crate::config::{AgentProvider, AgentsConfig};

    fn agent(id: &str, model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: id.into(),
            provider: AgentProvider::AcpClaudeCode,
            model: model.map(|s| s.to_string()),
            command: "/bin/false".into(),
            args: vec![],
            cwd: None,
            env: Default::default(),
            thinking_budget_tokens: None,
        }
    }

    fn profile(id: &str, agent: &str, model: Option<&str>, prompt_files: Option<Vec<PathBuf>>) -> ProfileConfig {
        ProfileConfig {
            id: id.into(),
            agent: agent.into(),
            model: model.map(|s| s.to_string()),
            system_prompt: prompt_files.map(|files| {
                files
                    .into_iter()
                    .map(|file| crate::config::SystemPromptEntry {
                        file,
                        inject: crate::config::SystemPromptInject::default(),
                    })
                    .collect()
            }),
            mcps: None,
            skills: None,
            mode: None,
            cwd: None,
            env: Default::default(),
            thinking_budget_tokens: None,
        }
    }

    fn write_prompt(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();

        write!(f, "{body}").unwrap();
        path
    }

    /// Helper: wrap a list of file paths as `SystemPromptEntry`s
    /// using the default inject toggles (on_create=true,
    /// on_update=false). Tests assert against `system_prompt_for`
    /// with `Bootstrap::Fresh` (which honours on_create=true) so
    /// the body matches the previous flat-vector shape.
    fn prompt_entries(files: Vec<PathBuf>) -> Vec<crate::config::SystemPromptEntry> {
        files
            .into_iter()
            .map(|file| crate::config::SystemPromptEntry {
                file,
                inject: crate::config::SystemPromptInject::default(),
            })
            .collect()
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
    fn profile_system_prompt_read_at_resolve_time() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = write_prompt(&dir, "plan.md", "You are a planner.");

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("plan", "cc", None, Some(vec![prompt_path]))],
            ..Default::default()
        };

        let r = ResolvedInstance::from_config(&cfg, Some("plan")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("You are a planner.")
        );
    }

    #[test]
    fn profile_system_prompt_concatenates_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = write_prompt(&dir, "base.md", "You are an agent.");
        let project = write_prompt(&dir, "project.md", "Working on hyprpilot.");

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("layered", "cc", None, Some(vec![base, project]))],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("layered")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("You are an agent.\n\nWorking on hyprpilot.")
        );
    }

    #[test]
    fn profile_system_prompt_empty_array_is_explicit_off_switch() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = write_prompt(&dir, "root.md", "should not apply");

        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![root_path])),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            // Empty Vec wholesale-replaces root with "no prompt".
            profiles: vec![profile("silent", "cc", None, Some(vec![]))],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("silent")).unwrap();
        assert!(r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).is_none());
    }

    #[test]
    fn profile_system_prompt_missing_file_errors() {
        let p = profile(
            "plan",
            "cc",
            None,
            Some(vec![PathBuf::from("/nonexistent/hyprpilot-test-never.md")]),
        );
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
        assert!(msg.contains("system_prompt"), "{msg}");
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
        assert!(r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).is_none());
    }

    #[test]
    fn profile_env_merges_onto_agent_env_at_resolve() {
        // Profile-level env entries flow through to the spawned
        // process. Profile values override agent values on key
        // collision (profile is the more specific scope); keys only
        // on the agent side survive untouched.
        let mut a = agent("cc", None);

        a.env.insert("AGENT_ONLY".into(), "from-agent".into());
        a.env.insert("OVERRIDDEN".into(), "agent-value".into());
        let mut p = profile("ask", "cc", None, None);

        p.env.insert("OVERRIDDEN".into(), "profile-value".into());
        p.env.insert("PROFILE_ONLY".into(), "from-profile".into());
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![a],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();

        assert_eq!(r.agent.env.get("AGENT_ONLY").map(String::as_str), Some("from-agent"));
        assert_eq!(r.agent.env.get("OVERRIDDEN").map(String::as_str), Some("profile-value"));
        assert_eq!(
            r.agent.env.get("PROFILE_ONLY").map(String::as_str),
            Some("from-profile")
        );
    }

    #[test]
    fn profile_mode_propagates_to_resolved_instance() {
        let mut p = profile("ask", "cc", None, None);

        p.mode = Some("plan".into());
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();

        assert_eq!(r.mode.as_deref(), Some("plan"));
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
    fn root_system_prompt_falls_back_when_profile_unset() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = write_prompt(&dir, "root.md", "global fallback prompt");

        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![root_path])),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("global fallback prompt")
        );
    }

    #[test]
    fn root_system_prompt_concatenates_multiple_files_with_blank_line() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_prompt(&dir, "a.md", "first");
        let b = write_prompt(&dir, "b.md", "second");
        let c = write_prompt(&dir, "c.md", "third");

        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![a, b, c])),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("first\n\nsecond\n\nthird")
        );
    }

    #[test]
    fn profile_system_prompt_wins_over_root() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = write_prompt(&dir, "root.md", "global fallback");
        let profile_path = write_prompt(&dir, "profile.md", "profile-specific");

        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![root_path])),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, Some(vec![profile_path]))],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("profile-specific")
        );
    }

    #[test]
    fn root_system_prompt_carries_through_bare_resolution() {
        // No profile → bare-agent path. Root system_prompt still
        // applies so unprofiled submits get the global default.
        let dir = tempfile::tempdir().unwrap();
        let root_path = write_prompt(&dir, "bare.md", "bare-path fallback");

        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![root_path])),
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
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("bare-path fallback")
        );
    }

    #[test]
    fn root_system_prompt_missing_file_errors_with_root_context() {
        let cfg = Config {
            system_prompt: Some(prompt_entries(vec![PathBuf::from(
                "/nonexistent/hyprpilot-root-prompt.md",
            )])),
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let err = ResolvedInstance::from_config(&cfg, Some("ask")).expect_err("missing root file fails");
        let msg = format!("{err:#}");
        assert!(msg.contains("root"), "{msg}");
        assert!(msg.contains("system_prompt"), "{msg}");
    }

    #[test]
    fn thinking_budget_default_visible_for_acp_claude_code() {
        // Per Anthropic's docs, Claude Opus 4.7+ default
        // `thinking.display = "omitted"` ships thinking blocks with
        // empty text. Captains pay for thinking either way; defaulting
        // visible surfaces it for free. Sentinel: env carries
        // MAX_THINKING_TOKENS=10000 when neither agent nor profile
        // set the field.
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![profile("ask", "cc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.agent.env.get("MAX_THINKING_TOKENS").map(String::as_str),
            Some("10000")
        );
    }

    #[test]
    fn thinking_budget_profile_wins_over_agent() {
        let mut a = agent("cc", None);

        a.thinking_budget_tokens = Some(2000);
        let mut p = profile("ask", "cc", None, None);

        p.thinking_budget_tokens = Some(50000);
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![a],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();

        assert_eq!(
            r.agent.env.get("MAX_THINKING_TOKENS").map(String::as_str),
            Some("50000")
        );
    }

    #[test]
    fn thinking_budget_zero_is_explicit_off_switch() {
        let mut p = profile("ask", "cc", None, None);

        p.thinking_budget_tokens = Some(0);
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();

        // Some(0) → don't inject. SDK falls back to its own default.
        assert!(!r.agent.env.contains_key("MAX_THINKING_TOKENS"));
    }

    #[test]
    fn thinking_budget_user_env_wins_over_default() {
        // Captain who explicitly sets MAX_THINKING_TOKENS in
        // [profiles.env] keeps their value — we only fill the slot
        // when nobody else has.
        let mut p = profile("ask", "cc", None, None);

        p.env.insert("MAX_THINKING_TOKENS".into(), "32000".into());
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
                ..Default::default()
            },
            profiles: vec![p],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.agent.env.get("MAX_THINKING_TOKENS").map(String::as_str),
            Some("32000")
        );
    }

    #[test]
    fn thinking_budget_skips_non_claude_code_providers() {
        let mut a = agent("oc", None);

        a.provider = AgentProvider::AcpOpenCode;
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![a],
                ..Default::default()
            },
            profiles: vec![profile("ask", "oc", None, None)],
            ..Default::default()
        };
        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();

        assert!(!r.agent.env.contains_key("MAX_THINKING_TOKENS"));
    }
}
