//! `ctl daemon *` — daemon introspection + lifecycle. Includes the
//! top-level `kill` shortcut alias.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonSubcommand {
    /// Print daemon pid, uptime, version, instance count.
    Status,
    /// Print daemon version (+ commit / build date when wired).
    Version,
    /// Graceful shutdown. Refuses with `-32603` when any instance has
    /// an in-flight turn unless `--force` is set.
    Shutdown {
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Serialize)]
struct ShutdownParams {
    force: bool,
}

impl CtlDispatch for DaemonSubcommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            DaemonSubcommand::Status => status(client),
            DaemonSubcommand::Version => version(client),
            DaemonSubcommand::Shutdown { force } => shutdown(client, force),
        }
    }
}

pub(super) fn kill(client: &CtlClient) -> Result<()> {
    emit(client, "daemon/kill", &Value::Null)
}

fn status(client: &CtlClient) -> Result<()> {
    emit(client, "daemon/status", &Value::Null)
}

fn version(client: &CtlClient) -> Result<()> {
    emit(client, "daemon/version", &Value::Null)
}

fn shutdown(client: &CtlClient, force: bool) -> Result<()> {
    if force {
        emit(client, "daemon/shutdown", &ShutdownParams { force: true })
    } else {
        emit(client, "daemon/shutdown", &Value::Null)
    }
}
