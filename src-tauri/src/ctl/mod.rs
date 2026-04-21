mod client;
mod handlers;

use anyhow::Result;
use clap::{Args, Subcommand};
use tracing::debug;

use crate::config::Config;
use crate::ctl::client::CtlClient;
use crate::ctl::handlers::{
    CancelHandler, CtlHandler, KillHandler, SessionInfoHandler, StatusHandler, SubmitHandler, ToggleHandler,
};
use crate::paths;

#[derive(Args, Debug)]
pub struct CtlArgs {
    #[command(subcommand)]
    pub command: CtlCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CtlCommand {
    /// Submit a prompt to the primary session.
    Submit {
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
}

/// Dispatch one `ctl` subcommand. Each arm builds the matching
/// `CtlHandler` (from `ctl/handlers.rs`) and hands it a `CtlClient`
/// (from `ctl/client.rs`) — a connection factory pointed at the
/// resolved socket. The handler decides how many connections to open
/// and how transport failure should affect its exit semantics; `run`
/// just wires clap to the handler registry.
pub fn run(cfg: Config, args: CtlArgs) -> Result<()> {
    let socket = cfg.daemon.socket.clone().unwrap_or_else(paths::socket_path);
    debug!(socket = %socket.display(), "ctl: dispatching");
    let client = CtlClient::new(socket);

    match args.command {
        CtlCommand::Submit { text } => SubmitHandler { text: text.join(" ") }.run(&client),
        CtlCommand::Cancel => CancelHandler.run(&client),
        CtlCommand::Toggle => ToggleHandler.run(&client),
        CtlCommand::Kill => KillHandler.run(&client),
        CtlCommand::SessionInfo => SessionInfoHandler.run(&client),
        CtlCommand::Status { watch } => StatusHandler { watch }.run(&client),
    }
}
