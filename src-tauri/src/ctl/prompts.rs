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
use crate::ctl::{emit, request_value, show_after, CtlDispatch};

#[derive(Subcommand, Debug, Clone)]
pub enum PromptsCommand {
    /// Send a prompt. `--instance` is overloaded:
    ///  - UUID or existing captain-set name → target that instance.
    ///  - Slug-shaped value (lowercase, `[a-z0-9][a-z0-9_-]*`,
    ///    ≤16 chars) that doesn't match any live instance →
    ///    auto-spawn a new instance and rename it to that slug.
    ///  - Anything else → error.
    ///
    /// When `--instance` is omitted, falls back to the focused
    /// instance; if none, auto-spawns under `--profile` (with
    /// optional `--cwd`) without a captain-set name.
    ///
    /// `--profile` carries `agent` (mandatory), `mode` (optional),
    /// and `model` (optional) — there's no separate `--agent` /
    /// `--mode` / `--model` here. Add a `[[profiles]]` entry instead.
    ///
    /// Reads stdin when no positional text and stdin is not a tty.
    Send {
        /// Target instance — UUID, existing captain-set name, or a
        /// slug to assign to a freshly spawned instance.
        #[arg(long = "instance")]
        instance_id: Option<String>,

        /// Profile id from `[[profiles]]`. Used only when no instance
        /// resolves; carries the agent + mode + model picks.
        #[arg(long)]
        profile: Option<String>,
        /// Working directory for the spawned instance. Used only when
        /// no instance resolves.
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Prompt text. Optional — when omitted, reads stdin to EOF
        /// (provided stdin is not a tty). Pass `-` to force stdin.
        #[arg(trailing_var_arg = true)]
        text: Vec<String>,

        /// Present the overlay focused on the resolved instance after
        /// the send lands. Maps a single keybind to "send + show"
        /// without chaining a second `ctl overlay show` call.
        #[arg(long, default_value_t = false)]
        show: bool,

        /// Append the text into the resolved instance's composer
        /// without dispatching it. The captain edits + submits at
        /// their own pace from the overlay. Existing composer text is
        /// preserved; a blank line separator slots between the prior
        /// content and the appended draft.
        #[arg(long, default_value_t = false)]
        draft: bool,
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
    profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    draft: bool,
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
                profile,
                cwd,
                text,
                show,
                draft,
            } => send(
                client,
                SendArgs {
                    instance_id,
                    profile,
                    cwd,
                    text,
                    show,
                    draft,
                },
            ),
            PromptsCommand::Cancel { instance_id } => cancel(client, instance_id),
        }
    }
}

struct SendArgs {
    instance_id: Option<String>,
    profile: Option<String>,
    cwd: Option<PathBuf>,
    text: Vec<String>,
    show: bool,
    draft: bool,
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
    let v = request_value(
        client,
        "prompts/send",
        &SendParams {
            instance_id: args.instance_id,
            text: resolved,
            profile_id: args.profile,
            cwd: args.cwd,
            draft: args.draft,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&v)?);

    if args.show {
        // Server returns the resolved instance id (`instanceId`). Pass
        // it to overlay/show so the captain lands focused on the
        // instance their prompt just dispatched against, not whichever
        // happens to be focused.
        let instance_id = v.get("instanceId").and_then(|v| v.as_str()).map(str::to_string);

        show_after(client, instance_id)?;
    }
    Ok(())
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
