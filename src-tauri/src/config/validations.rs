//! Validation predicates used by the `#[derive(Validate)]` surface
//! on `config::*` structs, plus cross-field rules that garde can't
//! express through its derive macros.
//!
//! garde's hooks fall into two buckets:
//!
//! - **Field-scoped** — the four `validate_*` functions take
//!   `(&value, &ctx)` and wire in via `#[garde(inner(custom(fn)))]`
//!   or `#[garde(custom(fn))]`. They run during
//!   `<Config as garde::Validate>::validate`'s tree walk.
//! - **Cross-field** — `validate_active_agent_reference` takes the
//!   assembled `Config` because it needs both `agents.active_agent`
//!   and `agents.agents[]` in one place. Called from
//!   `Config::validate()` after the derive pass so a report names
//!   the precise offender (`active_agent = 'x' but no matching
//!   [[agents]] entry exists`).
//!
//! Everything here is crate-private; callers outside `config`
//! consume validation only through `Config::validate()`.

use anyhow::{bail, Result};

use super::{AgentConfig, Config};

/// Per-layer uniqueness check for `[[agents]]`. Duplicate ids inside
/// a single layer are a user error; cross-layer duplicates are the
/// override mechanism (handled by `AgentsConfig::merge`) and not
/// seen here.
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

/// `agents.active_agent`, when set, must match a real `agents[].id`.
/// Reported outside the derive pass because garde has no
/// first-class "this field references that field" hook — the
/// assembled `Config` is the first place both sides of the
/// reference are in scope.
pub(super) fn validate_active_agent_reference(config: &Config) -> Result<()> {
    let Some(active) = config.agents.active_agent.as_deref() else {
        return Ok(());
    };

    let known = config.agents.agents.iter().any(|a| a.id == active);
    if !known {
        bail!(
            "agents.active_agent = '{active}' but no matching [[agents]] entry exists. \
             Configured ids: [{}]",
            config
                .agents
                .agents
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}
