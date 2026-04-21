//! Validation predicates used by the `#[derive(Validate)]` surface
//! on `config::*` structs.
//!
//! All predicates wire in through garde — no manual post-derive
//! pass remains on `Config::validate()`. Two shapes:
//!
//! - **Field-scoped** — `validate_agents_ids` takes the whole `&[AgentConfig]`
//!   (collection-level cross-element uniqueness check) and wires in
//!   via `#[garde(custom(validate_agents_ids))]` on the `agents` field.
//! - **Cross-field** — `agent_default_references_id` is a higher-order
//!   function: it closes over a sibling field (`&self.agents`) and
//!   returns the actual `FnOnce(&AgentDefaults, &()) -> garde::Result`
//!   garde calls. Wired as
//!   `#[garde(custom(agent_default_references_id(&self.agents)))]` on
//!   the `agent` field of `AgentsConfig`. This is the idiomatic
//!   garde pattern for a reference-across-siblings check, documented
//!   in the crate README as the "self access in rules" example.
//!
//! Everything here is crate-private; callers outside `config`
//! consume validation only through `Config::validate()`.

use super::{AgentConfig, AgentDefaults};

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

/// Higher-order custom validator: closes over `&self.agents` (a
/// sibling field on `AgentsConfig`) and returns the `FnOnce` garde
/// calls against the `agent: AgentDefaults` field. Runs during the
/// derive's tree walk, so its error lands in the unified garde
/// report rather than a separate post-pass.
///
/// Wired as `#[garde(custom(agent_default_references_id(&self.agents)))]`.
pub(super) fn agent_default_references_id<'a>(
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
