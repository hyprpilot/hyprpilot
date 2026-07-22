//! Garde predicates for the `config::*` derive surface. Everything
//! here is `pub(super)`; the outside API is `Config::validate()`.

use super::{AgentConfig, ProfileConfig, ProfileDefaults};

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

/// Every spawn flows through a `[[profiles]]` entry — there is no
/// bare-agent fallback. So the registry must carry at least one
/// profile, and validation rejects an empty list at config-load.
pub(super) fn validate_profiles_non_empty(profiles: &[ProfileConfig], _ctx: &()) -> garde::Result {
    if profiles.is_empty() {
        return Err(garde::Error::new(
            "configure at least one [[profiles]] entry — spawn requires a profile (set `--profile <id>` \
             or `[profile] default = '<id>'`); there is no bare-agent fallback",
        ));
    }
    Ok(())
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

/// `[profile] default` (when set) must name a real `[[profiles]]`
/// entry.
pub(super) fn validate_default_profile_id<'a>(
    profiles: &'a [ProfileConfig],
) -> impl FnOnce(&ProfileDefaults, &()) -> garde::Result + 'a {
    move |defaults, _ctx| {
        let Some(wanted) = defaults.default.as_deref() else {
            return Ok(());
        };
        if profiles.iter().any(|p| p.id == wanted) {
            return Ok(());
        }
        Err(garde::Error::new(format!(
            "[profile] default = '{wanted}' but no matching [[profiles]] entry exists. \
             Configured ids: [{}]",
            profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")
        )))
    }
}
