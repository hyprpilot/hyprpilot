//! Garde predicates for the `config::*` derive surface. Everything
//! here is `pub(super)`; the outside API is `Config::validate()`.

use globset::Glob;

use super::{AgentConfig, AgentDefaults, AgentsConfig, ProfileConfig};

pub(super) fn validate_agents_ids(agents: &[AgentConfig], _ctx: &()) -> garde::Result {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for a in agents {
        if !seen.insert(a.id.as_str()) {
            return Err(garde::Error::new(format!(
                "duplicate agent id '{}' — each [[agents]] entry must have a unique id",
                a.id
            )));
        }
    }

    Ok(())
}

/// Higher-order custom validator: closes over `&self.agents` so the
/// `custom(...)` attribute on `AgentsConfig.agent` runs a cross-field
/// check inside the garde tree walk.
pub(super) fn validate_agent_default_id<'a>(
    agents: &'a [AgentConfig],
) -> impl FnOnce(&AgentDefaults, &()) -> garde::Result + 'a {
    move |defaults, _ctx| {
        let Some(active) = defaults.default.as_deref() else {
            return Ok(());
        };
        if agents.iter().any(|a| a.id == active) {
            return Ok(());
        }
        Err(garde::Error::new(format!(
            "default = '{active}' but no matching [[agents]] entry exists. \
             Configured ids: [{}]",
            agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
        )))
    }
}

pub(super) fn validate_profiles_ids(profiles: &[ProfileConfig], _ctx: &()) -> garde::Result {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for p in profiles {
        if !seen.insert(p.id.as_str()) {
            return Err(garde::Error::new(format!(
                "duplicate profile id '{}' — each [[profiles]] entry must have a unique id",
                p.id
            )));
        }
    }
    Ok(())
}

/// Every profile's `agent` must name a real `[[agents]]` entry. Mirror
/// of `validate_agent_default_id` but scoped across the profile list.
pub(super) fn validate_profile_agent_references<'a>(
    agents: &'a [AgentConfig],
) -> impl FnOnce(&Vec<ProfileConfig>, &()) -> garde::Result + 'a {
    move |profiles, _ctx| {
        for p in profiles {
            if !agents.iter().any(|a| a.id == p.agent) {
                return Err(garde::Error::new(format!(
                    "profile '{}' references agent '{}' but no matching [[agents]] entry exists. \
                     Configured ids: [{}]",
                    p.id,
                    p.agent,
                    agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
        }
        Ok(())
    }
}

/// Glob-pattern validator for `auto_accept_tools` / `auto_reject_tools`.
/// Empty strings + invalid globs reject with the profile id + offending
/// pattern. Closes over the profile id so the error names the entry.
pub(super) fn validate_profile_tool_globs<'a>(
    profile_id: &'a str,
) -> impl FnOnce(&Vec<String>, &()) -> garde::Result + 'a {
    move |patterns, _ctx| {
        for p in patterns {
            if p.is_empty() {
                return Err(garde::Error::new(format!(
                    "profile '{profile_id}': empty string is not a valid tool glob pattern"
                )));
            }
            if let Err(err) = Glob::new(p) {
                return Err(garde::Error::new(format!(
                    "profile '{profile_id}': invalid tool glob pattern '{p}': {err}"
                )));
            }
        }
        Ok(())
    }
}

/// `[agent] default_profile` (when set) must name a real
/// `[[profiles]]` entry.
pub(super) fn validate_default_profile_id<'a>(
    profiles: &'a [ProfileConfig],
) -> impl FnOnce(&AgentsConfig, &()) -> garde::Result + 'a {
    move |agents, _ctx| {
        let Some(wanted) = agents.agent.default_profile.as_deref() else {
            return Ok(());
        };
        if profiles.iter().any(|p| p.id == wanted) {
            return Ok(());
        }
        Err(garde::Error::new(format!(
            "default_profile = '{wanted}' but no matching [[profiles]] entry exists. \
             Configured ids: [{}]",
            profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")
        )))
    }
}
