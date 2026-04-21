//! Garde predicates for the `config::*` derive surface. Everything
//! here is `pub(super)`; the outside API is `Config::validate()`.

use super::{AgentConfig, AgentDefaults};

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
