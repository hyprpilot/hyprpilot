use std::fmt;

use anyhow::{bail, Result};
use nucleo_picker::{render::DisplayRenderer, PickerOptions};

use crate::resolve::ProfileSummary;

#[derive(Clone)]
struct ProfileChoice(ProfileSummary);

impl fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = &self.0;
        // `!` (over the `*` default marker) flags a patch-resolution
        // failure, and the error replaces the model/cwd columns — the
        // row surfaces the broken patch instead of showing stale base
        // values as if they resolved cleanly (K-750 item 5).
        let marker = if profile.error.is_some() {
            "!"
        } else if profile.is_default {
            "*"
        } else {
            " "
        };
        if let Some(error) = profile.error.as_deref() {
            return write!(
                f,
                "{}  {}  {}  ! resolve failed: {}",
                marker,
                profile.id.as_str(),
                profile.agent.as_str(),
                error
            );
        }
        write!(
            f,
            "{}  {}  {}  {}  {}",
            marker,
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

    // nucleo-picker has no public initial-cursor setter — `pick()`
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

pub(super) fn pick_config_profile(names: Vec<String>, current: &[String]) -> Result<String> {
    pick_display(
        current_first(names, current),
        "no config profiles found under the config dir's `profiles/`",
        "config profile selection cancelled",
        "config profile picker requires an interactive terminal",
    )
}

/// Hoist the first overlay of the current selection to the front, so
/// Enter keeps it — same cursor trick as [`default_first`].
fn current_first(names: Vec<String>, current: &[String]) -> Vec<String> {
    let Some(index) = current.first().and_then(|c| names.iter().position(|n| n == c)) else {
        return names;
    };
    let mut names = names;
    let current = names.remove(index);
    names.insert(0, current);
    names
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
            error: None,
            ..Default::default()
        }
    }

    #[test]
    fn errored_row_shows_bang_marker_and_error() {
        let mut errored = summary("broken", true);
        errored.error = Some("invalid shape after patches".into());
        let line = ProfileChoice(errored).to_string();

        assert!(line.starts_with("!  broken"), "errored row leads with `!`: {line}");
        assert!(line.contains("resolve failed: invalid shape after patches"), "{line}");
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
    fn current_first_hoists_the_selected_overlay() {
        let names = vec!["a".to_string(), "personal".to_string(), "work".to_string()];

        assert_eq!(current_first(names, &["work".into()]), vec!["work", "a", "personal"]);
    }

    #[test]
    fn current_first_leaves_list_untouched_for_an_unknown_overlay() {
        let names = vec!["personal".to_string(), "work".to_string()];

        assert_eq!(current_first(names, &["gone".into()]), vec!["personal", "work"]);
    }

    #[test]
    fn default_first_leaves_list_untouched_without_a_default() {
        let ordered = default_first(vec![summary("a", false), summary("b", false)]);
        let ids: Vec<&str> = ordered.iter().map(|p| p.id.as_str()).collect();

        assert_eq!(ids, vec!["a", "b"]);
    }
}
