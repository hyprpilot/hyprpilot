use std::fmt;

use anyhow::{bail, Result};
use nucleo_picker::{render::DisplayRenderer, PickerOptions};

use crate::adapters::ProfileSummary;

use super::sessions::RestoreSession;

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

    let choices = profiles.into_iter().map(ProfileChoice).collect();
    pick_display(
        choices,
        "no profiles configured",
        "profile selection cancelled",
        "profile picker requires an interactive terminal",
    )
    .map(|choice| choice.0)
}

pub(super) fn pick_session(sessions: Vec<RestoreSession>) -> Result<RestoreSession> {
    if sessions.is_empty() {
        bail!("no restorable provider sessions found for the resolved cwd; pass --all to show every cwd");
    }

    pick_display(
        sessions,
        "no restorable provider sessions found for the resolved cwd; pass --all to show every cwd",
        "restore cancelled",
        "restore picker requires an interactive terminal",
    )
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
