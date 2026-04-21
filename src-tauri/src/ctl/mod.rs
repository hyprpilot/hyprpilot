use anyhow::Result;
use clap::{Args, Subcommand};
use tracing::info;

use crate::config::Config;

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

pub fn run(_cfg: Config, args: CtlArgs) -> Result<()> {
    match args.command {
        CtlCommand::Submit { text } => {
            let joined = text.join(" ");
            info!(prompt = %joined, "ctl submit — not implemented");
        }
        CtlCommand::Cancel => info!("ctl cancel — not implemented"),
        CtlCommand::Toggle => info!("ctl toggle — not implemented"),
        CtlCommand::Kill => info!("ctl kill — not implemented"),
        CtlCommand::SessionInfo => info!("ctl session-info — not implemented"),
    }

    Ok(())
}
