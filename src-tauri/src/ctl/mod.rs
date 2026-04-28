mod agents;
mod client;
mod commands;
mod daemon;
mod diag;
mod events;
mod mcps;
mod models;
mod modes;
mod overlay;
mod permissions;
mod prompts;
mod session;
mod sessions;
mod skills;
mod status;
mod window;

use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::Serialize;
use serde_json::Value;
use tracing::{debug, error};

use crate::config::Config;
use crate::ctl::client::CtlClient;
use crate::paths;
use crate::rpc::protocol::Outcome;

#[derive(Args, Debug)]
pub struct CtlArgs {
    #[command(subcommand)]
    pub command: CtlCommand,
}

/// Top-level `ctl` subcommand tree. Each variant either:
///
/// - is a top-level shortcut (`Submit`, `Cancel`, `Toggle`, `Kill`,
///   `SessionInfo`) that maps directly to a single wire method, or
/// - holds a namespaced sub-enum owned by the matching `ctl/<ns>.rs`
///   submodule (`Agents { command: AgentsSubcommand }`,
///   `Sessions { command: SessionsCommand }`, …).
///
/// Routing happens once in [`dispatch`]: shortcut variants call into
/// the namespace fns directly (`session::submit(...)`); namespaced
/// variants delegate to the namespace's own `dispatch` fn
/// (`agents::dispatch(client, command)`).
#[derive(Subcommand, Debug, Clone)]
pub enum CtlCommand {
    /// Submit a prompt to the primary session.
    Submit {
        /// Agent id override (defaults to `[agent] default` / the
        /// profile's agent).
        #[arg(long)]
        agent: Option<String>,

        /// Profile id — applies the configured model + system prompt
        /// overlay. Defaults to `[agent] default_profile`.
        #[arg(long)]
        profile: Option<String>,

        /// Prompt text to submit. Joined with spaces if supplied as multiple args.
        #[arg(trailing_var_arg = true)]
        text: Vec<String>,
    },

    /// Cancel the in-flight request on the active session.
    Cancel,

    /// Toggle the overlay window.
    Toggle,

    /// Kill the running daemon.
    Kill,

    /// Print the active session id + profile info.
    SessionInfo,

    /// Print the daemon / session status as JSON.
    ///
    /// Always emits a waybar-shaped JSON object (`{text, class, tooltip,
    /// alt}`) per line — waybar's `return-type: "json"` contract. One-shot
    /// by default: connects, fetches a snapshot, exits 0. When the daemon
    /// is not running, prints an offline payload and exits 0 — safe for
    /// waybar's `exec` field.
    ///
    /// With `--watch`, long-running: calls `status/subscribe` and streams
    /// one JSON object per state change. Reconnects with back-off on
    /// socket loss.
    Status {
        /// Stream status changes continuously (waybar mode).
        #[arg(long, default_value_t = false)]
        watch: bool,
    },

    /// Read-only operations over the `[[agents]]` registry.
    Agents {
        #[command(subcommand)]
        command: agents::AgentsSubcommand,
    },

    /// ACP `session/available_commands` passthrough — per-instance.
    Commands {
        #[command(subcommand)]
        command: commands::CommandsSubcommand,
    },

    /// ACP `session/set_session_mode` passthrough — per-instance.
    Modes {
        #[command(subcommand)]
        command: modes::ModesSubcommand,
    },

    /// ACP `session/set_session_model` passthrough — per-instance.
    Models {
        #[command(subcommand)]
        command: models::ModelsSubcommand,
    },

    /// Skill catalogue operations.
    Skills {
        #[command(subcommand)]
        command: skills::SkillsCommand,
    },

    /// MCP catalogue + per-instance enabled-set operations.
    Mcps {
        #[command(subcommand)]
        command: mcps::MCPsCommand,
    },

    /// Send / cancel single-shot prompts addressed to a specific
    /// instance. Distinct from `submit` — `submit` resolves through
    /// `(agent, profile)` and may auto-spawn; `prompts send` requires
    /// a live `--instance <id>`.
    Prompts {
        #[command(subcommand)]
        command: prompts::PromptsCommand,
    },

    /// Inspect / resolve pending permission prompts.
    Permissions {
        #[command(subcommand)]
        command: permissions::PermissionsCommand,
    },

    /// Overlay window control — hyprland-bind surface.
    ///
    /// Recommended hyprland binding:
    /// `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`.
    Overlay {
        #[command(subcommand)]
        command: overlay::OverlaySubcommand,
    },

    /// Daemon introspection + lifecycle. Distinct from the legacy
    /// top-level `kill` subcommand: `daemon shutdown` is the graceful
    /// surface (with a busy check), `daemon kill` would be the
    /// hard-stop.
    Daemon {
        #[command(subcommand)]
        command: daemon::DaemonSubcommand,
    },

    /// Operator diagnostics — read-only structural snapshot.
    Diag {
        #[command(subcommand)]
        command: diag::DiagSubcommand,
    },

    /// Connection-scoped event subscription. Streams every
    /// `events/notify` notification the daemon emits as one JSON line
    /// per event. Live-only — no replay, no reconnect; Ctrl-C exits.
    Events {
        #[command(subcommand)]
        command: events::EventsSubcommand,
    },

    /// Operations on persisted on-disk session transcripts. Distinct
    /// from `submit` / `prompts` (per-instance ACP wire ops) and
    /// instance lifecycle (`spawn`, `restart`, `shutdown`).
    Sessions {
        #[command(subcommand)]
        command: sessions::SessionsCommand,
    },
}

/// Every command-holder in this module implements `CtlDispatch`. The
/// top-level [`CtlCommand`] is the hub: its impl either handles a
/// top-level shortcut variant inline (`Submit`, `Cancel`, …) or
/// delegates to the inner sub-enum via the same trait method
/// (`agents::AgentsSubcommand` etc. all impl `CtlDispatch`). Each
/// namespace file owns one impl over its own sub-enum.
///
/// Adding a new namespace = new file + new sub-enum + `impl
/// CtlDispatch for NewSubcommand` + new variant on `CtlCommand` + one
/// match arm in `CtlCommand`'s impl that delegates `command.dispatch(client)`.
pub(super) trait CtlDispatch {
    fn dispatch(self, client: &CtlClient) -> Result<()>;
}

/// Connect, send `method` + `params`, pretty-print the success result
/// to stdout. RPC errors and transport failures bubble as `anyhow::Error`
/// so the top-level `run` catches once and prints + exits.
pub(super) fn emit<P: Serialize>(client: &CtlClient, method: &str, params: &P) -> Result<()> {
    let value = request_value(client, method, params)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Connect, send `method` + `params`, return the raw `Value` from the
/// success branch. RPC errors carry the JSON-RPC code in the error
/// message; transport errors bubble as-is. Used by namespaces whose
/// success branch needs post-processing beyond pretty-printing
/// (`diag::snapshot` writes to a file).
pub(super) fn request_value<P: Serialize>(client: &CtlClient, method: &str, params: &P) -> Result<Value> {
    let mut conn = client.connect()?;
    let params_value = serde_json::to_value(params).context("serialize params")?;
    match conn.call(method, params_value)? {
        Outcome::Success { result } => Ok(result),
        Outcome::Error { error } => {
            error!(code = error.code, message = %error.message, "ctl: rpc error");
            bail!("rpc error {}: {}", error.code, error.message)
        }
    }
}

/// Top-level `ctl` entry point. Builds the `CtlClient` from config,
/// then asks the parsed [`CtlCommand`] to dispatch itself via
/// [`CtlDispatch`]. Returns an [`ExitCode`] so `main` can defer to
/// the OS. Errors short-circuit with `?` inside the trait impls and
/// land here, where they're printed as a friendly stderr line.
pub fn run(cfg: Config, args: CtlArgs) -> Result<ExitCode> {
    let socket = cfg.daemon.socket.clone().unwrap_or_else(paths::socket_path);
    debug!(socket = %socket.display(), "ctl: dispatching");
    let client = CtlClient::new(socket);

    match args.command.dispatch(&client) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(err) => {
            eprintln!("{err}");
            Ok(ExitCode::from(1))
        }
    }
}

/// The hub. Top-level shortcut variants (`Submit`, `Cancel`, …) call
/// into the matching namespace fn directly; namespaced variants
/// delegate to the inner sub-enum's `CtlDispatch` impl. `Status` is
/// the documented exception — `status::run` swallows transport / RPC
/// errors via the offline sentinel so waybar's `exec` contract holds
/// when the daemon is down.
impl CtlDispatch for CtlCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            CtlCommand::Submit { agent, profile, text } => session::submit(client, text, agent, profile),
            CtlCommand::Cancel => session::cancel(client),
            CtlCommand::SessionInfo => session::info(client),
            CtlCommand::Toggle => window::toggle(client),
            CtlCommand::Kill => daemon::kill(client),
            CtlCommand::Status { watch } => status::run(client, watch),

            CtlCommand::Agents { command } => command.dispatch(client),
            CtlCommand::Commands { command } => command.dispatch(client),
            CtlCommand::Modes { command } => command.dispatch(client),
            CtlCommand::Models { command } => command.dispatch(client),
            CtlCommand::Skills { command } => command.dispatch(client),
            CtlCommand::Mcps { command } => command.dispatch(client),
            CtlCommand::Prompts { command } => command.dispatch(client),
            CtlCommand::Permissions { command } => command.dispatch(client),
            CtlCommand::Overlay { command } => command.dispatch(client),
            CtlCommand::Daemon { command } => command.dispatch(client),
            CtlCommand::Diag { command } => command.dispatch(client),
            CtlCommand::Events { command } => command.dispatch(client),
            CtlCommand::Sessions { command } => command.dispatch(client),
        }
    }
}
