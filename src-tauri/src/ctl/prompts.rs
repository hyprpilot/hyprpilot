//! `ctl prompts *` — seamlessly-scriptable prompt surface. `prompts
//! send` resolves the target through `--instance` (UUID or
//! captain-set name) → focused → auto-spawn-with-defaults, so
//! `echo "build" | ctl prompts send` Just Works against an empty
//! daemon. `prompts cancel` shares the same resolve-or-focused
//! fallback minus the spawn.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::ctl::client::CtlClient;
use crate::ctl::{emit, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum PromptsCommand {
    /// Send a prompt. Targets `--instance` (UUID or captain-set
    /// name) when supplied; falls back to the focused instance;
    /// auto-spawns with the supplied flags when neither resolves.
    /// Reads stdin when no positional text and stdin is not a tty.
    Send {
        /// Target instance — UUID or captain-set name.
        #[arg(long = "instance")]
        instance_id: Option<String>,

        /// Captain-set name to apply post-resolve / post-spawn.
        /// Validated as a slug (lowercase, ≤16 chars).
        #[arg(long)]
        id: Option<String>,

        /// Spawn-flag bag. Used only when no instance resolves.
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        model: Option<String>,

        /// Prompt text. Optional — when omitted, reads stdin to EOF
        /// (provided stdin is not a tty). Pass `-` to force stdin.
        #[arg(trailing_var_arg = true)]
        text: Vec<String>,
    },
    /// Cancel the in-flight turn. Falls back to focused when
    /// `--instance` is omitted.
    Cancel {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SendParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

impl CtlDispatch for PromptsCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            PromptsCommand::Send {
                instance_id,
                id,
                agent,
                profile,
                cwd,
                mode,
                model,
                text,
            } => send(
                client,
                SendArgs {
                    instance_id,
                    id,
                    agent,
                    profile,
                    cwd,
                    mode,
                    model,
                    text,
                },
            ),
            PromptsCommand::Cancel { instance_id } => cancel(client, instance_id),
        }
    }
}

struct SendArgs {
    instance_id: Option<String>,
    id: Option<String>,
    agent: Option<String>,
    profile: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
    text: Vec<String>,
}

fn send(client: &CtlClient, args: SendArgs) -> Result<()> {
    // Text resolution: explicit `-` reads stdin; empty positional
    // reads stdin only when stdin is not a tty (script piping case);
    // tty + no positional errors so the captain doesn't sit waiting
    // on a never-arriving stdin.
    let joined = args.text.join(" ");
    let resolved = if joined.trim() == "-" {
        read_stdin()?
    } else if joined.trim().is_empty() {
        if std::io::stdin().is_terminal() {
            anyhow::bail!("prompts send: no text supplied (positional argument or stdin pipe required)");
        }
        read_stdin()?
    } else {
        joined
    };
    emit(
        client,
        "prompts/send",
        &SendParams {
            instance_id: args.instance_id,
            text: resolved,
            id: args.id,
            agent_id: args.agent,
            profile_id: args.profile,
            cwd: args.cwd,
            mode: args.mode,
            model: args.model,
        },
    )
}

// Swap to the `clap-stdin` crate's `MaybeStdin<T>` value-parser if a
// second subcommand picks up the same stdin-or-positional shape — at
// that point the four-line helper stops paying for the dep cost.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).context("read stdin")?;
    Ok(buf)
}

fn cancel(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    emit(client, "prompts/cancel", &CancelParams { instance_id })
}
