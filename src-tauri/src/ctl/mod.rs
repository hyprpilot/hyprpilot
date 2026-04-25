mod client;
mod handlers;

use anyhow::Result;
use clap::{Args, Subcommand};
use tracing::debug;

use crate::config::Config;
use crate::ctl::client::CtlClient;
use crate::ctl::handlers::{
    AgentsListHandler, CancelHandler, CommandsListHandler, CtlHandler, KillHandler, ModelsListHandler,
    ModelsSetHandler, ModesListHandler, ModesSetHandler, OverlayHideHandler, OverlayPresentHandler,
    OverlayToggleHandler, PermissionsPendingHandler, PermissionsRespondHandler, PromptsCancelHandler,
    PromptsSendHandler, SessionInfoHandler, SessionsForgetHandler, SessionsInfoHandler, SessionsListHandler,
    SkillsGetHandler, SkillsListHandler, SkillsReloadHandler, StatusHandler, SubmitHandler, ToggleHandler,
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

    /// Skill catalogue operations.
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },

    /// Send / cancel single-shot prompts addressed to a specific
    /// instance. Distinct from `submit` — `submit` resolves through
    /// `(agent, profile)` and may auto-spawn; `prompts send` requires
    /// a live `--instance <id>`.
    Prompts {
        #[command(subcommand)]
        command: PromptsCommand,
    },

    /// Inspect / resolve pending permission prompts.
    Permissions {
        #[command(subcommand)]
        command: PermissionsCommand,
    },

    /// Overlay window control — hyprland-bind surface.
    ///
    /// Recommended hyprland binding:
    /// `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`.
    Overlay {
        #[command(subcommand)]
        command: OverlaySubcommand,
    },

    /// Operations on persisted on-disk session transcripts. Distinct
    /// from `submit` / `prompts` (per-instance ACP wire ops) and
    /// instance lifecycle (`spawn`, `restart`, `shutdown`).
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
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

#[derive(Subcommand, Debug, Clone)]
pub enum OverlaySubcommand {
    /// Show + focus the overlay (no-op when already visible). With
    /// `--instance`, also focuses that instance after the present.
    Present {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Hide the overlay (no-op when already hidden). Webview stays warm.
    Hide,
    /// Flip the overlay's visibility. Race-safe across concurrent calls.
    Toggle,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SkillsCommand {
    /// List every skill currently loaded by the daemon.
    List {
        /// Optional instance id — reserved for per-profile skill
        /// allowlists once K-275 lands. Passing it today surfaces
        /// the gap loudly via `unimplemented!` on the server side.
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Fetch one skill's full markdown body + references.
    Get {
        #[arg(long)]
        slug: String,
    },
    /// Force-reload the registry from disk.
    Reload,
}

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

#[derive(Subcommand, Debug, Clone)]
pub enum SessionsCommand {
    /// List the agent's persisted sessions.
    List {
        #[arg(long = "instance")]
        instance_id: Option<String>,
        #[arg(long = "agent")]
        agent_id: Option<String>,
        #[arg(long = "profile")]
        profile_id: Option<String>,
        #[arg(long = "cwd")]
        cwd: Option<std::path::PathBuf>,
    },
    /// Delete a persisted session transcript by id. Idempotent on
    /// the wire shape; today the daemon panics with `unimplemented!`
    /// because ACP 0.12 doesn't expose a delete verb — `ctl` mirrors
    /// the panic on the client side rather than round-tripping.
    Forget {
        #[arg(long)]
        id: String,
    },
    /// Fetch one session's projection by id.
    Info {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PermissionsCommand {
    /// List pending permission requests, optionally filtered by
    /// instance.
    Pending {
        #[arg(long = "instance")]
        instance_id: Option<String>,
    },
    /// Resolve a pending permission request by id.
    Respond {
        #[arg(long = "request")]
        request_id: String,
        #[arg(long = "option")]
        option_id: String,
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
        CtlCommand::Skills { command } => match command {
            SkillsCommand::List { instance_id } => SkillsListHandler { instance_id }.run(&client),
            SkillsCommand::Get { slug } => SkillsGetHandler { slug }.run(&client),
            SkillsCommand::Reload => SkillsReloadHandler.run(&client),
        },
        CtlCommand::Prompts { command } => match command {
            PromptsCommand::Send { instance_id, text } => PromptsSendHandler {
                instance_id,
                text: text.join(" "),
            }
            .run(&client),
            PromptsCommand::Cancel { instance_id } => PromptsCancelHandler { instance_id }.run(&client),
        },
        CtlCommand::Permissions { command } => match command {
            PermissionsCommand::Pending { instance_id } => PermissionsPendingHandler { instance_id }.run(&client),
            PermissionsCommand::Respond { request_id, option_id } => {
                PermissionsRespondHandler { request_id, option_id }.run(&client)
            }
        },
        CtlCommand::Overlay { command } => match command {
            OverlaySubcommand::Present { instance_id } => OverlayPresentHandler { instance_id }.run(&client),
            OverlaySubcommand::Hide => OverlayHideHandler.run(&client),
            OverlaySubcommand::Toggle => OverlayToggleHandler.run(&client),
        },
        CtlCommand::Sessions { command } => match command {
            SessionsCommand::List {
                instance_id,
                agent_id,
                profile_id,
                cwd,
            } => SessionsListHandler {
                instance_id,
                agent_id,
                profile_id,
                cwd,
            }
            .run(&client),
            SessionsCommand::Forget { id } => SessionsForgetHandler { id }.run(&client),
            SessionsCommand::Info { id } => SessionsInfoHandler { id }.run(&client),
        },
    }
}
