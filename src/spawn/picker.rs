use std::fmt;

use anyhow::{bail, Result};
use nucleo_picker::{render::DisplayRenderer, PickerOptions};

use crate::resolve::ProfileSummary;

#[derive(Clone)]
struct ProfileChoice(ProfileSummary);

impl fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = &self.0;
        write!(
            f,
            "{}  {}  {}  {}  {}",
            if profile.is_default { "*" } else { " " },
            profile.id.as_str(),
            profile.agent.as_str(),
            profile.model.as_deref().unwrap_or("-"),
            profile.cwd.as_deref().unwrap_or("-")
        )
    }
}

pub(super) fn pick_profile(profiles: Vec<ProfileSummary>) -> Result<ProfileSummary> {
    if profiles.is_empty() {
        bail!("no profiles configured");
    }

    // nucleo-picker 0.11 has no public initial-cursor setter — `pick()`
    // always starts the cursor on match index 0. With an empty query
    // that's the first-inserted item, so hoisting the `[profile] default`
    // entry to the front pre-selects it under the cursor (Enter launches
    // it). The `*` marker rides along on the row. Stable within each
    // partition so config order is otherwise preserved.
    let choices = default_first(profiles).into_iter().map(ProfileChoice).collect();
    pick_display(
        choices,
        "no profiles configured",
        "profile selection cancelled",
        "profile picker requires an interactive terminal",
    )
    .map(|choice| choice.0)
}

/// Hoist the `is_default` entry to the front, preserving the relative
/// order of every other entry. The picker's cursor starts on index 0,
/// so the default lands under it. No default (or none marked) → the
/// list is returned untouched.
fn default_first(profiles: Vec<ProfileSummary>) -> Vec<ProfileSummary> {
    let Some(index) = profiles.iter().position(|p| p.is_default) else {
        return profiles;
    };
    let mut profiles = profiles;
    let default = profiles.remove(index);
    profiles.insert(0, default);
    profiles
}

fn pick_display<T: Clone + fmt::Display + Send + Sync + 'static>(
    items: Vec<T>,
    empty_message: &str,
    cancelled_message: &str,
    non_interactive_message: &str,
) -> Result<T> {
    if items.is_empty() {
        bail!("{empty_message}");
    }

    let mut picker = PickerOptions::new().picker(DisplayRenderer);
    picker.extend(items);
    match picker.pick() {
        Ok(Some(item)) => Ok(item.clone()),
        Ok(None) => bail!("{cancelled_message}"),
        Err(nucleo_picker::error::PickError::NotInteractive) => {
            bail!("{non_interactive_message}")
        }
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, is_default: bool) -> ProfileSummary {
        ProfileSummary {
            id: id.into(),
            agent: "cc".into(),
            model: None,
            cwd: None,
            is_default,
        }
    }

    #[test]
    fn default_first_hoists_default_to_front_preserving_order() {
        let ordered = default_first(vec![
            summary("a", false),
            summary("b", false),
            summary("d", true),
            summary("c", false),
        ]);
        let ids: Vec<&str> = ordered.iter().map(|p| p.id.as_str()).collect();

        // Default lands at index 0 (under the picker cursor); the rest
        // keep their relative order.
        assert_eq!(ids, vec!["d", "a", "b", "c"]);
    }

    #[test]
    fn default_first_leaves_list_untouched_without_a_default() {
        let ordered = default_first(vec![summary("a", false), summary("b", false)]);
        let ids: Vec<&str> = ordered.iter().map(|p| p.id.as_str()).collect();

        assert_eq!(ids, vec!["a", "b"]);
    }
}
