mod client;

use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use tracing::{debug, error};

use crate::config::Config;
use crate::ctl::client::CtlConnection;
use crate::paths;
use crate::rpc::protocol::Outcome;

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

impl CtlCommand {
    /// Map a CLI subcommand to its wire `(method, params)` pair. `status`
    /// is handled separately in `run` — it never reaches this path.
    fn wire(self) -> (&'static str, Value) {
        match self {
            CtlCommand::Submit { text } => ("submit", json!({ "text": text.join(" ") })),
            CtlCommand::Cancel => ("cancel", Value::Null),
            CtlCommand::Toggle => ("toggle", Value::Null),
            CtlCommand::Kill => ("kill", Value::Null),
            CtlCommand::SessionInfo => ("session-info", Value::Null),
            CtlCommand::Status { .. } => unreachable!("status is dispatched before wire()"),
        }
    }
}

/// Drive one `ctl` subcommand. Success prints the `result` payload as
/// pretty JSON on stdout; an RPC error writes the message to stderr and
/// calls `std::process::exit(1)` so the caller sees a non-zero exit —
/// keeping `main()`'s `Result<()>` signature untouched.
pub fn run(cfg: Config, args: CtlArgs) -> Result<()> {
    let socket = cfg.daemon.socket.clone().unwrap_or_else(paths::socket_path);

    // `status` is handled separately: it may be long-running (`--watch`) and
    // has its own offline-fallback + formatting logic.
    if let CtlCommand::Status { watch } = args.command {
        return client::run_status(&socket, watch);
    }

    debug!(socket = %socket.display(), "ctl: connecting");
    let (method, params) = args.command.wire();
    let outcome = match CtlConnection::connect(&socket).and_then(|mut c| c.call_outcome(method, params)) {
        Ok(o) => o,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    match outcome {
        Outcome::Success { result } => {
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Outcome::Error { error } => {
            error!(code = error.code, message = %error.message, "ctl: rpc error");
            eprintln!("rpc error {}: {}", error.code, error.message);
            std::process::exit(1);
        }
    }
}
