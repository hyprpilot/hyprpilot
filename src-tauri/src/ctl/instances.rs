//! `ctl instances *` — live process management for scripting.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, request_value, show_after, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum InstancesSubcommand {
    /// List live instances.
    List,
    /// Focus an instance. `--instance` accepts UUID or captain-set
    /// name; omitted falls back to the focused pointer (no-op).
    ///
    /// `--ensure` flips to spawn-or-focus: when `--instance` names a
    /// slug that doesn't resolve to a live instance, spawn one (using
    /// `--profile` / `--agent` / `--cwd` / etc.) and rename it to
    /// that slug before focusing. Lets a single keybind act as
    /// "open this named conversation, creating it if needed".
    Focus {
        #[arg(long = "instance")]
        instance_id: Option<String>,
        /// Present the overlay focused on this instance after the
        /// focus call lands. Maps a single keybind to "focus + show"
        /// without chaining a second `ctl overlay show` call.
        #[arg(long, default_value_t = false)]
        show: bool,
        /// Spawn-and-rename when `--instance` is a slug with no live
        /// match. No-op when an instance already carries the slug.
        #[arg(long, default_value_t = false)]
        ensure: bool,
        /// Spawn flag — used only when `--ensure` triggers a spawn.
        #[arg(long = "profile")]
        profile_id: Option<String>,
        /// Spawn flag — used only when `--ensure` triggers a spawn.
        #[arg(long = "agent")]
        agent_id: Option<String>,
        /// Spawn flag — used only when `--ensure` triggers a spawn.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Spawn flag — used only when `--ensure` triggers a spawn.
        #[arg(long)]
        mode: Option<String>,
        /// Spawn flag — used only when `--ensure` triggers a spawn.
        #[arg(long)]
        model: Option<String>,
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
        /// Present the overlay focused on the freshly-spawned instance
        /// after spawn (and rename, when `--name` is supplied) lands.
        #[arg(long, default_value_t = false)]
        show: bool,
    },
    /// Restart an instance. `--instance` accepts UUID or name;
    /// omitted falls back to focused.
    Restart {
        #[arg(long = "instance")]
        instance_id: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Present the overlay focused on the restarted instance
        /// after the restart lands.
        #[arg(long, default_value_t = false)]
        show: bool,
    },
    /// Shut one instance down. `--instance` accepts UUID or name;
    /// omitted falls back to focused.
    Shutdown {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Fetch one instance's projection. `--instance` accepts UUID or
    /// name; omitted falls back to focused.
    Info {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Rename a live instance. `--instance` accepts UUID or current
    /// name; omitted falls back to focused. Pass an empty `--name ""`
    /// to clear the name.
    Rename {
        #[arg(long = "instance")]
        instance_id: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct InstanceParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct FocusParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ensure: bool,
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
#[serde(rename_all = "camelCase")]
struct RestartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenameParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl CtlDispatch for InstancesSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            InstancesSubcommand::List => emit(client, "instances/list", &Value::Null),
            InstancesSubcommand::Focus {
                instance_id,
                show,
                ensure,
                profile_id,
                agent_id,
                cwd,
                mode,
                model,
            } => {
                let v = request_value(
                    client,
                    "instances/focus",
                    &FocusParams {
                        instance_id: instance_id.clone(),
                        ensure,
                        profile_id,
                        agent_id,
                        cwd,
                        mode,
                        model,
                    },
                )?;
                println!("{}", serde_json::to_string_pretty(&v)?);

                if show {
                    // Pass through whatever the captain typed (UUID or
                    // captain-set name) — the overlay handler accepts
                    // either and falls back to the now-focused
                    // instance when omitted.
                    show_after(client, instance_id)?;
                }
                Ok(())
            }
            InstancesSubcommand::Spawn {
                profile_id,
                agent_id,
                cwd,
                mode,
                model,
                name,
                show,
            } => {
                let spawn_params = SpawnParams {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                };
                let v = request_value(client, "instances/spawn", &spawn_params)?;
                println!("{}", serde_json::to_string_pretty(&v)?);
                let minted = v.get("instanceId").and_then(Value::as_str).map(str::to_string);

                if let Some(n) = name {
                    // Two-step composition when --name is supplied:
                    // spawn (capture minted id) → rename. The
                    // single-step path stays unchanged for the common
                    // case where the captain doesn't bother with
                    // naming.
                    let rv = request_value(
                        client,
                        "instances/rename",
                        &RenameParams {
                            instance_id: minted.clone(),
                            name: Some(n),
                        },
                    )?;
                    println!("{}", serde_json::to_string_pretty(&rv)?);
                }

                if show {
                    show_after(client, minted)?;
                }
                Ok(())
            }
            InstancesSubcommand::Restart { instance_id, cwd, show } => {
                let v = request_value(
                    client,
                    "instances/restart",
                    &RestartParams {
                        instance_id: instance_id.clone(),
                        cwd,
                    },
                )?;
                println!("{}", serde_json::to_string_pretty(&v)?);

                if show {
                    show_after(client, instance_id)?;
                }
                Ok(())
            }
            InstancesSubcommand::Shutdown { instance_id } => {
                emit(client, "instances/shutdown", &InstanceParams { instance_id })
            }
            InstancesSubcommand::Info { instance_id } => {
                emit(client, "instances/info", &InstanceParams { instance_id })
            }
            InstancesSubcommand::Rename { instance_id, name } => {
                emit(client, "instances/rename", &RenameParams { instance_id, name })
            }
        }
    }
}
