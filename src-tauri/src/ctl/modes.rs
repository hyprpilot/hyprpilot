//! `ctl modes *` — ACP `session/set_session_mode` passthrough.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum ModesSubcommand {
    /// List session modes the addressed instance advertised.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
    /// Set the addressed instance's current mode.
    Set {
        #[arg(long = "instance")]
        instance_id: String,
        #[arg(long = "mode")]
        mode_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    instance_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetParams {
    instance_id: String,
    mode_id: String,
}

impl CtlDispatch for ModesSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            ModesSubcommand::List { instance_id } => list(client, instance_id),
            ModesSubcommand::Set { instance_id, mode_id } => set(client, instance_id, mode_id),
        }
    }
}

fn list(client: &CtlClient, instance_id: String) -> Result<()> {
    emit(client, "modes/list", &ListParams { instance_id })
}

fn set(client: &CtlClient, instance_id: String, mode_id: String) -> Result<()> {
    emit(client, "modes/set", &SetParams { instance_id, mode_id })
}
