//! Garde predicates for the `config::*` derive surface. Everything
//! here is `pub(super)`; the outside API is `Config::validate()`.

use std::path::PathBuf;

use super::{AgentConfig, AgentDefaults, KeymapsConfig, Modifier, ProfileConfig, ProfileDefaults};

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

/// Trait that lets `validate_unique_nonempty` reject empty entries +
/// duplicates over either `String` or `PathBuf` without two copies of
/// the validator. `is_blank` smooths over `String::is_empty` vs
/// `PathBuf::as_os_str().is_empty()`; `display_label` powers the error
/// message (paths use `.display()`).
pub(super) trait ListEntry: std::hash::Hash + Eq {
    fn is_blank(&self) -> bool;
    fn display_label(&self) -> String;
}

impl ListEntry for String {
    fn is_blank(&self) -> bool {
        self.is_empty()
    }
    fn display_label(&self) -> String {
        self.clone()
    }
}

impl ListEntry for PathBuf {
    fn is_blank(&self) -> bool {
        self.as_os_str().is_empty()
    }
    fn display_label(&self) -> String {
        self.display().to_string()
    }
}

/// Reject empty entries + duplicates inside an `Option<Vec<T>>`.
/// Generic over `String` / `PathBuf` (and anything else implementing
/// the local `ListEntry` trait). `~` expansion happens at consume
/// time so paths are compared in raw form.
pub(super) fn validate_unique_nonempty<T: ListEntry>(list: &Option<Vec<T>>, _ctx: &()) -> garde::Result {
    let Some(items) = list else {
        return Ok(());
    };
    let mut seen: std::collections::HashSet<&T> = std::collections::HashSet::new();
    for item in items {
        if item.is_blank() {
            return Err(garde::Error::new("empty entry is not valid"));
        }
        if !seen.insert(item) {
            return Err(garde::Error::new(format!("duplicate entry '{}'", item.display_label())));
        }
    }
    Ok(())
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

/// Per-binding modifier uniqueness check. Unknown modifier tokens
/// reject at TOML parse time via `Modifier`'s `Deserialize` (closed
/// enum with `rename_all = "lowercase"`); this predicate just catches
/// repeats like `modifiers = ["ctrl", "ctrl"]`.
pub(super) fn validate_unique_modifiers(mods: &Vec<Modifier>, _ctx: &()) -> garde::Result {
    let mut seen: std::collections::HashSet<Modifier> = std::collections::HashSet::new();
    for m in mods {
        if !seen.insert(*m) {
            return Err(garde::Error::new(format!("duplicate modifier '{m:?}' in binding")));
        }
    }
    Ok(())
}

/// Within-scope keymaps collision check. Garde-walk adapter — wraps
/// `keymaps::validate_collisions` (which returns `anyhow::Result`)
/// into `garde::Result` so the rule lives inside the derive walk
/// alongside every other cross-field validator.
pub(super) fn validate_keymaps_collisions(cfg: &KeymapsConfig, _ctx: &()) -> garde::Result {
    super::keymaps::validate_collisions(cfg).map_err(|e| garde::Error::new(format!("{e}")))
}

/// `system_prompt` ⊕ `system_prompt_file` — exclusive. Iterates the
/// profile list at validate time; folded into the garde walk so the
/// rule fires inside the derive tree alongside every other
/// cross-field validator instead of as a post-walk for-loop.
pub(super) fn validate_profile_prompt_sources(profiles: &Vec<ProfileConfig>, _ctx: &()) -> garde::Result {
    for p in profiles {
        if p.system_prompt.is_some() && p.system_prompt_file.is_some() {
            return Err(garde::Error::new(format!(
                "profile '{}' sets both system_prompt and system_prompt_file — pick one",
                p.id
            )));
        }
    }
    Ok(())
}
