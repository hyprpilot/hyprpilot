//! `ctl overlay *` — overlay window control (hyprland-bind surface).
//!
//! Recommended hyprland binding:
//! `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum OverlaySubcommand {
    /// Show + focus the overlay (no-op when already visible). With
    /// `--instance`, also focuses that instance after the present.
    Present {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Hide the overlay (no-op when already hidden). Webview stays warm.
    Hide,
    /// Flip the overlay's visibility. Race-safe across concurrent calls.
    Toggle,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresentParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

impl CtlDispatch for OverlaySubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            OverlaySubcommand::Present { instance_id } => present(client, instance_id),
            OverlaySubcommand::Hide => hide(client),
            OverlaySubcommand::Toggle => toggle(client),
        }
    }
}

fn present(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    emit(client, "overlay/present", &PresentParams { instance_id })
}

fn hide(client: &CtlClient) -> Result<()> {
    emit(client, "overlay/hide", &Value::Null)
}

fn toggle(client: &CtlClient) -> Result<()> {
    emit(client, "overlay/toggle", &Value::Null)
}
