mod config;
mod logging;
mod mcp;
mod paths;
mod profile;
mod profiles;
mod resolve;
mod skills;
mod spawn;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "hyprpilot",
    version,
    about = "Hyprpilot: resolve a profile from layered config and launch the vendor's native CLI.",
    long_about = None,
)]
struct Cli {
    /// Path to the global config.toml (overrides the XDG default).
    #[arg(long, global = true, env = "HYPRPILOT_CONFIG")]
    config: Option<PathBuf>,

    /// Name of a config-layer profile (resolved to
    /// `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.toml`). Distinct
    /// from the session `[[profiles]]` registry driving agent +
    /// system-prompt overlays — session profiles are addressed per
    /// call via the positional `[PROFILE]` argument.
    #[arg(long = "config-profile", global = true, env = "HYPRPILOT_CONFIG_PROFILE")]
    config_profile: Option<String>,

    /// Override the tracing filter (otherwise `RUST_LOG` + defaults apply).
    #[arg(long, global = true, value_enum, env = "HYPRPILOT_LOG_LEVEL")]
    log_level: Option<logging::LogLevel>,

    #[command(flatten)]
    launch: spawn::LaunchArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run an in-tree MCP server (e.g. `mcp serve`) for an agent vendor to
    /// spawn via stdio. The launcher auto-injects entries when the resolved
    /// skill registry for a spawn is non-empty.
    Mcp(mcp::server::McpArgs),

    /// List configured session profiles.
    Profiles(profiles::ProfilesArgs),
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();

    let log_reload = logging::init(cli.log_level)?;

    let cfg = config::load(cli.config.as_deref(), cli.config_profile.as_deref())?;
    cfg.validate()?;
    logging::apply_config_level(&log_reload, cli.log_level, cfg.logging.level)?;

    match cli.command {
        // Bare `hyprpilot [PROFILE]` IS the launch: pick the profile
        // interactively when none is given, then exec into the
        // resolved vendor CLI.
        None => spawn::run(cfg, cli.launch),
        Some(Command::Profiles(args)) => profiles::run(cfg, args),
        Some(Command::Mcp(args)) => {
            // The MCP sidecar owns stdin/stdout for its protocol;
            // a dedicated tokio runtime keeps the sidecar self-contained.
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(args.run())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
