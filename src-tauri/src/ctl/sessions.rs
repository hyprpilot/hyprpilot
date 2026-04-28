//! `ctl sessions *` — operations on persisted on-disk session
//! transcripts. Distinct from `submit` / `prompts` (per-instance ACP
//! wire ops) and instance lifecycle (`spawn`, `restart`, `shutdown`).

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum SessionsCommand {
    /// List the agent's persisted sessions.
    List {
        #[arg(long = "instance")]
        instance_id: Option<String>,
        #[arg(long = "agent")]
        agent_id: Option<String>,
        #[arg(long = "profile")]
        profile_id: Option<String>,
        #[arg(long = "cwd")]
        cwd: Option<PathBuf>,
    },
    /// Delete a persisted session transcript by id. Idempotent on
    /// the wire shape; today the daemon panics with `unimplemented!`
    /// because ACP 0.12 doesn't expose a delete verb — `ctl` mirrors
    /// the panic on the client side rather than round-tripping.
    Forget {
        #[arg(long)]
        id: String,
    },
    /// Fetch one session's projection by id.
    Info {
        #[arg(long)]
        id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
}

#[derive(Serialize)]
struct InfoParams {
    id: String,
}

impl CtlDispatch for SessionsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            SessionsCommand::List {
                instance_id,
                agent_id,
                profile_id,
                cwd,
            } => list(client, instance_id, agent_id, profile_id, cwd),
            SessionsCommand::Forget { id } => forget(id),
            SessionsCommand::Info { id } => info(client, id),
        }
    }
}

fn list(
    client: &CtlClient,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
) -> Result<()> {
    emit(
        client,
        "sessions/list",
        &ListParams {
            instance_id,
            agent_id,
            profile_id,
            cwd,
        },
    )
}

/// Stubbed per CLAUDE.md "stubs panic, don't pretend". ACP 0.12 has
/// no session-delete verb; the daemon would also panic. Flip to a
/// real `emit(...)` call when ACP lands the underlying verb.
fn forget(id: String) -> Result<()> {
    unimplemented!("ctl sessions forget '{id}': ACP 0.12 does not expose a session-delete verb (track upstream)")
}

fn info(client: &CtlClient, id: String) -> Result<()> {
    emit(client, "sessions/info", &InfoParams { id })
}
