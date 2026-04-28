//! `ctl prompts *` — single-shot prompts addressed to a specific
//! instance. Distinct from `ctl submit` (which resolves through
//! `(agent, profile)` and may auto-spawn) — `prompts send` requires
//! a live `--instance <id>`.

use std::io::Read;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum PromptsCommand {
    /// Send a prompt to a live instance. `text` is positional; pass
    /// `-` to read it from stdin.
    Send {
        #[arg(long = "instance")]
        instance_id: String,

        /// Prompt text. Use `-` to read from stdin.
        #[arg(trailing_var_arg = true)]
        text: Vec<String>,
    },
    /// Cancel the addressed instance's in-flight turn.
    Cancel {
        #[arg(long = "instance")]
        instance_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendParams {
    instance_id: String,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelParams {
    instance_id: String,
}

impl CtlDispatch for PromptsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            PromptsCommand::Send { instance_id, text } => send(client, instance_id, text),
            PromptsCommand::Cancel { instance_id } => cancel(client, instance_id),
        }
    }
}

fn send(client: &CtlClient, instance_id: String, text: Vec<String>) -> Result<()> {
    let joined = text.join(" ");
    let resolved = if joined.trim() == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).context("read stdin")?;
        buf
    } else {
        joined
    };
    emit(
        client,
        "prompts/send",
        &SendParams {
            instance_id,
            text: resolved,
        },
    )
}

fn cancel(client: &CtlClient, instance_id: String) -> Result<()> {
    emit(client, "prompts/cancel", &CancelParams { instance_id })
}
