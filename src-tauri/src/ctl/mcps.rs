//! `ctl mcps *` — MCP catalogue + per-instance enabled-set operations.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum MCPsCommand {
    /// List the global MCP catalogue. With `--instance`, every entry
    /// gets an `enabled` flag reflecting the per-instance override
    /// (or the resolved profile's `mcps` allowlist).
    List {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Install a per-instance MCP enabled-list override and restart
    /// the addressed instance. `--enabled` is comma-separated; pass
    /// an empty value (`--enabled=`) for the explicit "no MCPs"
    /// override.
    Set {
        #[arg(long = "instance")]
        instance_id: String,
        /// Comma-separated MCP names. Empty value installs `[]`.
        #[arg(long, value_delimiter = ',', default_value = "")]
        enabled: Vec<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetParams {
    instance_id: String,
    enabled: Vec<String>,
}

impl CtlDispatch for MCPsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            MCPsCommand::List { instance_id } => list(client, instance_id),
            MCPsCommand::Set { instance_id, enabled } => set(client, instance_id, enabled),
        }
    }
}

fn list(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    emit(client, "mcps/list", &ListParams { instance_id })
}

fn set(client: &CtlClient, instance_id: String, enabled: Vec<String>) -> Result<()> {
    // `--enabled=` produces `[""]` from clap; treat as empty.
    let enabled = enabled.into_iter().filter(|s| !s.is_empty()).collect();
    emit(client, "mcps/set", &SetParams { instance_id, enabled })
}
