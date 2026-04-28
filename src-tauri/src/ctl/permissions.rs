//! `ctl permissions *` — inspect / resolve pending permission prompts.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum PermissionsCommand {
    /// List pending permission requests, optionally filtered by
    /// instance.
    Pending {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Resolve a pending permission request by id.
    Respond {
        #[arg(long = "request")]
        request_id: String,
        #[arg(long = "option")]
        option_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RespondParams {
    request_id: String,
    option_id: String,
}

impl CtlDispatch for PermissionsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            PermissionsCommand::Pending { instance_id } => pending(client, instance_id),
            PermissionsCommand::Respond { request_id, option_id } => respond(client, request_id, option_id),
        }
    }
}

fn pending(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    emit(client, "permissions/pending", &PendingParams { instance_id })
}

fn respond(client: &CtlClient, request_id: String, option_id: String) -> Result<()> {
    emit(client, "permissions/respond", &RespondParams { request_id, option_id })
}
