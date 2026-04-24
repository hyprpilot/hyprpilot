mod client;
mod handlers;

use anyhow::Result;
use clap::{Args, Subcommand};
use tracing::debug;

use crate::config::Config;
use crate::ctl::client::CtlClient;
use crate::ctl::handlers::{
    AgentsListHandler, CancelHandler, CommandsListHandler, CtlHandler, KillHandler, ModelsListHandler,
    ModelsSetHandler, ModesListHandler, ModesSetHandler, SessionInfoHandler, StatusHandler, SubmitHandler,
    ToggleHandler,
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
        command: AgentsSubcommand,
    },

    /// ACP `session/available_commands` passthrough — per-instance.
    Commands {
        #[command(subcommand)]
        command: CommandsSubcommand,
    },

    /// ACP `session/set_session_mode` passthrough — per-instance.
    Modes {
        #[command(subcommand)]
        command: ModesSubcommand,
    },

    /// ACP `session/set_session_model` passthrough — per-instance.
    Models {
        #[command(subcommand)]
        command: ModelsSubcommand,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AgentsSubcommand {
    /// List configured agents.
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CommandsSubcommand {
    /// List available commands for the addressed instance.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ModesSubcommand {
    /// List session modes the addressed instance advertised.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
    /// Set the addressed instance's current mode.
    Set {
        #[arg(long = "instance")]
        instance_id: String,
        #[arg(long = "mode")]
        mode_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ModelsSubcommand {
    /// List models the addressed instance advertised.
    List {
        #[arg(long = "instance")]
        instance_id: String,
    },
    /// Set the addressed instance's current model.
    Set {
        #[arg(long = "instance")]
        instance_id: String,
        #[arg(long = "model")]
        model_id: String,
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
        CtlCommand::Submit { agent, profile, text } => SubmitHandler {
            text: text.join(" "),
            agent_id: agent,
            profile_id: profile,
        }
        .run(&client),
        CtlCommand::Cancel => CancelHandler.run(&client),
        CtlCommand::Toggle => ToggleHandler.run(&client),
        CtlCommand::Kill => KillHandler.run(&client),
        CtlCommand::SessionInfo => SessionInfoHandler.run(&client),
        CtlCommand::Status { watch } => StatusHandler { watch }.run(&client),
        CtlCommand::Agents { command } => match command {
            AgentsSubcommand::List => AgentsListHandler.run(&client),
        },
        CtlCommand::Commands { command } => match command {
            CommandsSubcommand::List { instance_id } => CommandsListHandler { instance_id }.run(&client),
        },
        CtlCommand::Modes { command } => match command {
            ModesSubcommand::List { instance_id } => ModesListHandler { instance_id }.run(&client),
            ModesSubcommand::Set { instance_id, mode_id } => ModesSetHandler { instance_id, mode_id }.run(&client),
        },
        CtlCommand::Models { command } => match command {
            ModelsSubcommand::List { instance_id } => ModelsListHandler { instance_id }.run(&client),
            ModelsSubcommand::Set { instance_id, model_id } => ModelsSetHandler { instance_id, model_id }.run(&client),
        },
    }
}
