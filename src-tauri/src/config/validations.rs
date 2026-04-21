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

use super::{AgentConfig, Config, Dimension};

/// `logging.level` must name a canonical tracing level. Case-insensitive
/// so user configs can write `Info` or `DEBUG` without friction.
pub(super) fn validate_log_level(value: &String, _ctx: &()) -> garde::Result {
    const ALLOWED: &[&str] = &["trace", "debug", "info", "warn", "error"];

    if !ALLOWED.contains(&value.to_lowercase().as_str()) {
        return Err(garde::Error::new(format!("must be one of {ALLOWED:?}, got '{value}'")));
    }

    Ok(())
}

/// Pixel dimensions must be in `1..=10_000`; percent dimensions must
/// be in `1..=100`. The upper cap on pixel values is a sanity guard
/// against TOML typos (`width = 1000000`) — 10 000 is beyond the
/// widest single-monitor setups we plausibly target.
pub(super) fn validate_dimension(value: &Dimension, _ctx: &()) -> garde::Result {
    match *value {
        Dimension::Pixels(0) => Err(garde::Error::new("pixel dimension must be >= 1")),
        Dimension::Pixels(px) if px > 10_000 => Err(garde::Error::new(format!(
            "pixel dimension {px} exceeds 10000 — refusing absurd size"
        ))),
        Dimension::Pixels(_) => Ok(()),
        Dimension::Percent(p) if (1..=100).contains(&p) => Ok(()),
        Dimension::Percent(p) => Err(garde::Error::new(format!("percent must be 1..=100, got {p}"))),
    }
}

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

/// Hex colour string: `#RRGGBB` or `#RRGGBBAA`. Used on every
/// `ThemeXxx` colour leaf. Rejects short form (`#RGB`) — the theme
/// emitter doesn't normalise, and readers downstream (CSS custom
/// properties) would accept it but cascade differently.
pub(super) fn validate_hex_color(value: &String, _ctx: &()) -> garde::Result {
    let is_valid =
        value.starts_with('#') && matches!(value.len(), 7 | 9) && value[1..].chars().all(|c| c.is_ascii_hexdigit());

    if !is_valid {
        return Err(garde::Error::new(format!(
            "must be a hex color (#RRGGBB or #RRGGBBAA), got '{value}'"
        )));
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
