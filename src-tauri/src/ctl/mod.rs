mod client;
mod daemon;
mod diag;
mod instances;
mod overlay;
mod permissions;
mod prompts;
mod status;

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

/// Top-level `ctl` subcommand tree. Webview consumers go through
/// Tauri commands (in `adapters/commands.rs`); this is the operator,
/// scripting, and waybar surface. Routing happens once in
/// [`CtlDispatch::dispatch`]: shortcut variants call into the
/// namespace fns directly; namespaced variants delegate to the
/// inner sub-enum's own `CtlDispatch` impl.
#[derive(Subcommand, Debug, Clone)]
pub enum CtlCommand {
    /// Kill the running daemon.
    Kill,

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

    /// Send / cancel single-shot prompts addressed to a specific
    /// instance. Requires a live `--instance <id>`.
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

    /// Live process management for scripting.
    Instances {
        #[command(subcommand)]
        command: instances::InstancesSubcommand,
    },

    /// Daemon introspection + lifecycle. Distinct from the top-level
    /// `kill` shortcut: `daemon shutdown` is the graceful surface
    /// (with a busy check), `daemon kill` is the hard-stop.
    Daemon {
        #[command(subcommand)]
        command: daemon::DaemonSubcommand,
    },

    /// Operator diagnostics — read-only structural snapshot.
    Diag {
        #[command(subcommand)]
        command: diag::DiagSubcommand,
    },
}

/// Every command-holder in this module implements `CtlDispatch`. The
/// top-level [`CtlCommand`] is the hub: its impl either handles a
/// top-level shortcut variant inline (`Kill`, `Status`) or delegates
/// to the inner sub-enum via the same trait method.
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

/// Fire an `overlay/show` after a state-mutating call. Threaded
/// through the `--show` flag on `prompts send` / `instances spawn` /
/// `instances focus` / `instances restart` so captains can map a
/// keybind to "spawn + show" without chaining a second `ctl overlay
/// show` call. `instance_id` is forwarded verbatim — the daemon's
/// overlay handler accepts a UUID, captain-set name, or omits to
/// just present without a focus change. Soft-fails: if `overlay/show`
/// errors (no main window, e.g. headless tests), the caller's
/// success path stays intact and the `--show` failure logs a warn.
pub(super) fn show_after(client: &CtlClient, instance_id: Option<String>) -> Result<()> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ShowParams {
        #[serde(skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
    }
    let params = ShowParams { instance_id };
    if let Err(err) = request_value(client, "overlay/show", &params) {
        tracing::warn!(%err, "ctl: --show: overlay/show after main call failed");
    }
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

/// The hub. Top-level shortcut variants call into the matching
/// namespace fn directly; namespaced variants delegate to the inner
/// sub-enum's `CtlDispatch` impl. `Status` is the documented
/// exception — `status::run` swallows transport / RPC errors via the
/// offline sentinel so waybar's `exec` contract holds when the
/// daemon is down.
impl CtlDispatch for CtlCommand {
    fn dispatch(self, client: &CtlClient) -> Result<()> {
        match self {
            CtlCommand::Kill => daemon::kill(client),
            CtlCommand::Status { watch } => status::run(client, watch),
            CtlCommand::Prompts { command } => command.dispatch(client),
            CtlCommand::Permissions { command } => command.dispatch(client),
            CtlCommand::Overlay { command } => command.dispatch(client),
            CtlCommand::Instances { command } => command.dispatch(client),
            CtlCommand::Daemon { command } => command.dispatch(client),
            CtlCommand::Diag { command } => command.dispatch(client),
        }
    }
}
