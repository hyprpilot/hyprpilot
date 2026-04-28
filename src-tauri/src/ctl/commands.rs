//! `ctl commands *` — ACP `session/available_commands` passthrough.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum CommandsSubcommand {
    /// List available commands for the addressed instance.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    instance_id: String,
}

impl CtlDispatch for CommandsSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            CommandsSubcommand::List { instance_id } => list(client, instance_id),
        }
    }
}

fn list(client: &CtlClient, instance_id: String) -> Result<()> {
    emit(client, "commands/list", &ListParams { instance_id })
}
