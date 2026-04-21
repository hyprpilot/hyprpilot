mod client;

use anyhow::Result;
use clap::{Args, Subcommand};
use tracing::{debug, error};

use crate::config::Config;
use crate::paths;
use crate::rpc::protocol::{Call, Outcome};

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
}

impl CtlCommand {
    fn into_call(self) -> Call {
        match self {
            CtlCommand::Submit { text } => Call::Submit { text: text.join(" ") },
            CtlCommand::Cancel => Call::Cancel,
            CtlCommand::Toggle => Call::Toggle,
            CtlCommand::Kill => Call::Kill,
            CtlCommand::SessionInfo => Call::SessionInfo,
        }
    }
}

/// Drive one `ctl` subcommand. Success prints the `result` payload as
/// pretty JSON on stdout; an RPC error writes the message to stderr and
/// calls `std::process::exit(1)` so the caller sees a non-zero exit —
/// keeping `main()`'s `Result<()>` signature untouched.
pub fn run(cfg: Config, args: CtlArgs) -> Result<()> {
    let socket = cfg.daemon.socket.clone().unwrap_or_else(paths::socket_path);

    debug!(socket = %socket.display(), "ctl: connecting");
    let outcome = match client::call(&socket, args.command.into_call()) {
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
