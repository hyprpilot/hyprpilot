//! `ctl skills *` — skill catalogue operations.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum SkillsCommand {
    /// List every skill currently loaded by the daemon.
    List {
        /// Optional instance id — reserved for per-profile skill
        /// allowlists once K-275 lands. Passing it today surfaces
        /// the gap loudly via `unimplemented!` on the server side.
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Fetch one skill's full markdown body + references.
    Get {
        #[arg(long)]
        slug: String,
    },
    /// Force-reload the registry from disk.
    Reload,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

#[derive(Serialize)]
struct GetParams {
    slug: String,
}

impl CtlDispatch for SkillsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            SkillsCommand::List { instance_id } => list(client, instance_id),
            SkillsCommand::Get { slug } => get(client, slug),
            SkillsCommand::Reload => reload(client),
        }
    }
}

fn list(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    emit(client, "skills/list", &ListParams { instance_id })
}

fn get(client: &CtlClient, slug: String) -> Result<()> {
    emit(client, "skills/get", &GetParams { slug })
}

fn reload(client: &CtlClient) -> Result<()> {
    emit(client, "skills/reload", &Value::Null)
}
