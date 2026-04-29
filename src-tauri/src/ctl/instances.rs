//! `ctl instances *` — live process management for scripting.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum InstancesSubcommand {
    /// List live instances.
    List,
    /// Focus an instance (by uuid).
    Focus {
        #[arg(long)]
        id: String,
    },
    /// Spawn a new instance against a profile / agent.
    Spawn {
        #[arg(long = "profile")]
        profile_id: Option<String>,
        #[arg(long = "agent")]
        agent_id: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Restart an instance, optionally with a new cwd.
    Restart {
        #[arg(long)]
        id: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// Shut one instance down.
    Shutdown {
        #[arg(long)]
        id: String,
    },
    /// Fetch one instance's projection.
    Info {
        #[arg(long)]
        id: String,
    },
}

#[derive(Serialize)]
struct IdParams {
    id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpawnParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Serialize)]
struct RestartParams {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
}

impl CtlDispatch for InstancesSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            InstancesSubcommand::List => emit(client, "instances/list", &Value::Null),
            InstancesSubcommand::Focus { id } => emit(client, "instances/focus", &IdParams { id }),
            InstancesSubcommand::Spawn {
                profile_id,
                agent_id,
                cwd,
                mode,
                model,
            } => emit(
                client,
                "instances/spawn",
                &SpawnParams {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                },
            ),
            InstancesSubcommand::Restart { id, cwd } => emit(client, "instances/restart", &RestartParams { id, cwd }),
            InstancesSubcommand::Shutdown { id } => emit(client, "instances/shutdown", &IdParams { id }),
            InstancesSubcommand::Info { id } => emit(client, "instances/info", &IdParams { id }),
        }
    }
}
