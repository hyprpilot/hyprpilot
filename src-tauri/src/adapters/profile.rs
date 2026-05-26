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

use anyhow::{Context, Result};

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
    pub effort: Option<String>,
    /// Resolved per-entry system-prompt list. Each entry carries its
    /// own pre-read body + inject toggles; the actor filters the
    /// list against the bootstrap variant (Fresh vs Resume) at spawn
    /// time and concatenates the surviving entries. `Some(vec![])` /
    /// `None` both mean "no prompt".
    pub system_prompt: Vec<ResolvedSystemPromptEntry>,
    /// Per-instance mode override. Populated from `SpawnSpec::mode`
    /// at resolve time. Generic layer just carries it; ACP's runtime
    /// passes it into `AcpInstance` and surfaces it via `InstanceInfo`.
    /// Vendor-specific interpretation (e.g. claude-code's `plan` /
    /// `edit`) happens inside the vendor agent impl.
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
    /// Pick a profile (addressed id, then `[profile] default`),
    /// fold root `[[patches]]`, and project the result through
    /// `from_profile_explicit`. Errors when neither addresses a
    /// real profile — there is no bare-agent fallback.
    ///
    /// Production callers go through `adapters::acp::instances::
    /// resolve_into_instance_and_profile` which returns the patched
    /// `ProfileConfig` alongside, so downstream MCP / skills
    /// registries read from the same shape. This entry point is
    /// the thin test-only surface — production paths must use the
    /// production helper so the MCP / skills registries get the
    /// same patched view, not a re-derived one that could drift.
    #[cfg(test)]
    pub fn from_config(config: &Config, profile_id: Option<&str>) -> Result<Self> {
        let picked_id = profile_id
            .map(str::to_string)
            .or_else(|| config.profile.default.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no profile addressed and no `[profile] default` configured — \
                 every spawn requires a `[[profiles]]` entry. \
                 Pass `--profile <id>` or set `[profile] default = '<id>'`."
                )
            })?;
        let base = config
            .profiles
            .iter()
            .find(|p| p.id == picked_id)
            .cloned()
            .with_context(|| format!("profile '{picked_id}' not found in [[profiles]] registry"))?;
        let patched = apply_root_patches(config, base)?;
        Self::from_profile_explicit(&patched, config)
    }

    /// Resolve against an already-materialised `ProfileConfig` —
    /// the result of `resolve_effective_profile` (which picks the
    /// addressed profile, folds root `[[patches]]`, folds any
    /// `--with-config` overlays, and re-validates the shape).
    /// The agent referenced by `profile.agent` must still exist in
    /// `config.agents.agents`.
    ///
    /// Production callers go through `adapters::acp::instances::
    /// resolve_into_instance_and_profile` (or `resolve_with_patches`
    /// for the `withConfig` path) — both return the patched
    /// `ProfileConfig` alongside the `ResolvedInstance` so downstream
    /// MCP / skills registry builders read from the same shape.
    pub fn from_profile_explicit(profile: &ProfileConfig, config: &Config) -> Result<Self> {
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
        let effort = profile.effort.clone().or_else(|| agent.effort.clone());
        let system_prompt = Self::load_system_prompt(profile)?;

        // Project profile-level overrides onto a clone of the agent
        // so the spawn path (which iterates `entry.env` + reads
        // `entry.cwd`) sees them. Profile entries are the more
        // specific scope and win on collision. `${VAR}` interpolation
        // against the daemon's process env happens later in
        // `agents/mod.rs::expand_value`.
        let mut agent = agent.clone();

        for (k, v) in profile.env.iter() {
            agent.env.insert(k.clone(), v.clone());
        }

        // `profile.cwd` overrides `agent.cwd` when set. This is the
        // channel through which root `[[patches]]` `cwd` reaches
        // the spawn — patches land on `ProfileConfig.cwd`, then
        // this line projects it onto `AgentConfig.cwd`, which
        // `AcpAgent::base_command` reads at spawn time. Without
        // this line, captains writing `cwd: ~/notes` in a patch
        // saw the agent spawn from `$HOME` (the daemon's inherited
        // cwd) and the patch silently no-op'd.
        if let Some(profile_cwd) = profile.cwd.as_ref() {
            agent.cwd = Some(profile_cwd.clone());
        }

        Ok(Self {
            agent,
            profile_id: Some(profile.id.clone()),
            model,
            effort,
            system_prompt,
            mode: profile.mode.clone(),
        })
    }

    /// Resolve the system prompt for a profile. Each entry's body is
    /// read in list order at resolve time; the actor filters the list
    /// against the bootstrap variant at spawn time and concatenates
    /// the surviving bodies with a blank-line separator. `system_prompt
    /// = []` is the explicit off-switch and resolves to an empty list.
    /// `None` resolves to an empty list too — there is no root-level
    /// fallback anymore (use `[[patches]]` for shared prompts).
    fn load_system_prompt(profile: &ProfileConfig) -> Result<Vec<ResolvedSystemPromptEntry>> {
        match &profile.system_prompt {
            Some(entries) => read_prompt_entries(entries, &format!("profile '{}'", profile.id)),
            None => Ok(Vec::new()),
        }
    }
}

/// Fold root-level `[[patches]]` into the captain-picked profile.
/// Each patch may carry an optional `$match: { profile: "<glob>" }`
/// directive — stripped before merging, skips non-matching entries.
/// The result deserializes back through `ProfileConfig` so garde
/// re-validates the post-merge shape.
///
/// Test-only — production paths use `adapters::acp::instances::
/// resolve_effective_profile` (which composes this same logic +
/// `--with-config` overlays) so MCP / skills registries read from
/// the same patched view.
#[cfg(test)]
fn apply_root_patches(config: &Config, profile: ProfileConfig) -> Result<ProfileConfig> {
    let Some(patches) = config.patches.as_deref() else {
        return Ok(profile);
    };
    if patches.is_empty() {
        return Ok(profile);
    }

    let profile_id = profile.id.clone();
    let base = serde_json::to_value(&profile).context("apply_root_patches: profile serialize failed")?;
    let merged = crate::config::patch::apply_root_patches_to_profile(base, patches, &profile_id);
    let patched: ProfileConfig = serde_json::from_value(merged)
        .with_context(|| format!("apply_root_patches: profile '{profile_id}' invalid after patch merge"))?;
    garde::Validate::validate(&patched)
        .with_context(|| format!("apply_root_patches: profile '{profile_id}' failed validation after patch merge"))?;
    Ok(patched)
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
            effort: None,
            command: "/bin/false".into(),
            args: vec![],
            cwd: None,
            env: Default::default(),
        }
    }

    fn profile(id: &str, agent: &str, model: Option<&str>, prompt_files: Option<Vec<PathBuf>>) -> ProfileConfig {
        ProfileConfig {
            id: id.into(),
            agent: agent.into(),
            model: model.map(|s| s.to_string()),
            effort: None,
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
            mcp: None,
            mode: None,
            cwd: None,
            env: Default::default(),
        }
    }

    fn write_prompt(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();

        write!(f, "{body}").unwrap();
        path
    }

    /// Helper: wrap a list of file paths as `SystemPromptEntry`s
    #[test]
    fn profile_model_overrides_agent_model() {
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", Some("sonnet"))],
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
        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
            },
            // Empty Vec is the explicit "no prompt" off-switch.
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
    fn falls_back_to_default_profile_then_errors() {
        let mut cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", Some("sonnet"))],
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

        // With `[profile] default` cleared AND no `--profile`,
        // resolution errors — there is no bare-agent fallback.
        cfg.profile.default = None;
        let err = ResolvedInstance::from_config(&cfg, None).expect_err("must error without a profile");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("every spawn requires a") || msg.contains("no profile addressed"),
            "expected captain-facing error, got: {msg}"
        );
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
            agents: AgentsConfig { agents: vec![a] },
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
            },
            profiles: vec![],
            ..Default::default()
        };
        let err = ResolvedInstance::from_config(&cfg, Some("ghost")).expect_err("unknown profile");
        assert!(err.to_string().contains("profile 'ghost' not found"));
    }

    // ── root `[[patches]]` apply to the picked profile ────────────

    #[test]
    fn root_patch_without_match_injects_system_prompt_into_profile() {
        // Captain's typical shape: one shared system_prompt across
        // every profile. `[[patches]]` with no `$match` filter
        // overlays the prompt list onto whichever profile was picked.
        let dir = tempfile::tempdir().unwrap();
        let base = write_prompt(&dir, "base.md", "shared base prompt");

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
            },
            profiles: vec![profile("personal/claude/opus", "cc", None, None)],
            patches: Some(vec![serde_json::json!({
                "system_prompt": [{ "file": base.to_string_lossy() }],
            })]),
            ..Default::default()
        };

        let r = ResolvedInstance::from_config(&cfg, Some("personal/claude/opus")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("shared base prompt"),
            "root patch's system_prompt must reach the resolved profile"
        );
    }

    #[test]
    fn root_patch_with_match_only_applies_to_glob_matching_profile() {
        let dir = tempfile::tempdir().unwrap();
        let personal = write_prompt(&dir, "personal.md", "personal-only prompt");

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
            },
            profiles: vec![
                profile("personal/claude/opus", "cc", None, None),
                profile("work/claude/opus", "cc", None, None),
            ],
            patches: Some(vec![serde_json::json!({
                "$match": { "profile": "personal/*" },
                "system_prompt": [{ "file": personal.to_string_lossy() }],
            })]),
            ..Default::default()
        };

        let personal_resolved = ResolvedInstance::from_config(&cfg, Some("personal/claude/opus")).unwrap();
        assert_eq!(
            personal_resolved
                .system_prompt_for(&crate::adapters::Bootstrap::Fresh)
                .as_deref(),
            Some("personal-only prompt")
        );

        let work_resolved = ResolvedInstance::from_config(&cfg, Some("work/claude/opus")).unwrap();
        assert!(
            work_resolved
                .system_prompt_for(&crate::adapters::Bootstrap::Fresh)
                .is_none(),
            "personal/* glob must not reach work/* profile"
        );
    }

    #[test]
    fn root_patches_fold_in_declaration_order() {
        // Two patches both setting system_prompt — later wins (right
        // wins on scalar / non-keyed field collision).
        let dir = tempfile::tempdir().unwrap();
        let first = write_prompt(&dir, "first.md", "first");
        let second = write_prompt(&dir, "second.md", "second");

        let cfg = Config {
            agents: AgentsConfig {
                agents: vec![agent("cc", None)],
            },
            profiles: vec![profile("ask", "cc", None, None)],
            patches: Some(vec![
                serde_json::json!({ "system_prompt": [{ "file": first.to_string_lossy() }] }),
                serde_json::json!({ "system_prompt": [{ "file": second.to_string_lossy() }] }),
            ]),
            ..Default::default()
        };

        let r = ResolvedInstance::from_config(&cfg, Some("ask")).unwrap();
        assert_eq!(
            r.system_prompt_for(&crate::adapters::Bootstrap::Fresh).as_deref(),
            Some("first\n\nsecond"),
            "system_prompt is a keyed-by-`file` array → both patches' entries land in order"
        );
    }
}
