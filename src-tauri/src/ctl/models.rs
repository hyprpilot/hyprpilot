//! `ctl models *` — ACP `session/set_session_model` passthrough.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum ModelsSubcommand {
    /// List models the addressed instance advertised.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
    /// Set the addressed instance's current model.
    Set {
        #[arg(long = "instance")]
        instance_id: String,
        #[arg(long = "model")]
        model_id: String,
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
    model_id: String,
}

impl CtlDispatch for ModelsSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            ModelsSubcommand::List { instance_id } => list(client, instance_id),
            ModelsSubcommand::Set { instance_id, model_id } => set(client, instance_id, model_id),
        }
    }
}

fn list(client: &CtlClient, instance_id: String) -> Result<()> {
    emit(client, "models/list", &ListParams { instance_id })
}

fn set(client: &CtlClient, instance_id: String, model_id: String) -> Result<()> {
    emit(client, "models/set", &SetParams { instance_id, model_id })
}
