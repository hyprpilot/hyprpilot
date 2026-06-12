use anyhow::{bail, Result};
use nucleo_picker::{render::DisplayRenderer, PickerOptions};

use super::sessions::RestoreSession;

pub(super) fn pick_session(sessions: Vec<RestoreSession>) -> Result<RestoreSession> {
    if sessions.is_empty() {
        bail!("no restorable provider sessions found for the resolved cwd; pass --all to show every cwd");
    }

    let mut picker = PickerOptions::new().picker(DisplayRenderer);
    picker.extend(sessions);
    match picker.pick() {
        Ok(Some(session)) => Ok(session.clone()),
        Ok(None) => bail!("restore cancelled"),
        Err(nucleo_picker::error::PickError::NotInteractive) => {
            bail!("restore picker requires an interactive terminal")
        }
        Err(err) => Err(err.into()),
    }
}
