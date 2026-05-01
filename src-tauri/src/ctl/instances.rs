//! `ctl instances *` — live process management for scripting.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, request_value, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum InstancesSubcommand {
    /// List live instances.
    List,
    /// Focus an instance. `--id` accepts UUID or captain-set name;
    /// omitted falls back to the focused pointer (no-op).
    Focus {
        #[arg(long)]
        id: Option<String>,
    },
    /// Spawn a new instance against a profile / agent. Optional
    /// `--name` applies a captain-set name post-spawn.
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
        /// Captain-set name to apply post-spawn (slug, ≤16 chars).
        #[arg(long)]
        name: Option<String>,
    },
    /// Restart an instance. `--id` accepts UUID or name; omitted
    /// falls back to focused.
    Restart {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// Shut one instance down. `--id` accepts UUID or name; omitted
    /// falls back to focused.
    Shutdown {
        #[arg(long)]
        id: Option<String>,
    },
    /// Fetch one instance's projection. `--id` accepts UUID or name;
    /// omitted falls back to focused.
    Info {
        #[arg(long)]
        id: Option<String>,
    },
    /// Rename a live instance. `--id` accepts UUID or current name;
    /// omitted falls back to focused. Pass an empty `--name ""` to
    /// clear the name.
    Rename {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Serialize, Default)]
struct IdParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
}

#[derive(Serialize)]
struct RenameParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
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
                name,
            } => {
                let spawn_params = SpawnParams {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                };
                if let Some(n) = name {
                    // Two-step composition when --name is supplied:
                    // spawn (capture minted id) → rename. Single-step
                    // path stays a plain `emit` for the common case
                    // where the captain doesn't bother with naming.
                    let v = request_value(client, "instances/spawn", &spawn_params)?;
                    println!("{}", serde_json::to_string_pretty(&v)?);
                    let key = v.get("id").and_then(Value::as_str).map(str::to_string);
                    emit(client, "instances/rename", &RenameParams { id: key, name: Some(n) })
                } else {
                    emit(client, "instances/spawn", &spawn_params)
                }
            }
            InstancesSubcommand::Restart { id, cwd } => emit(client, "instances/restart", &RestartParams { id, cwd }),
            InstancesSubcommand::Shutdown { id } => emit(client, "instances/shutdown", &IdParams { id }),
            InstancesSubcommand::Info { id } => emit(client, "instances/info", &IdParams { id }),
            InstancesSubcommand::Rename { id, name } => emit(client, "instances/rename", &RenameParams { id, name }),
        }
    }
}
