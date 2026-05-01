mod adapters;
mod completion;
mod config;
mod ctl;
mod daemon;
mod logging;
mod mcp;
mod paths;
mod rpc;
mod skills;
mod tools;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "hyprpilot",
    version,
    about = "Hyprpilot: Assistant in overlay disguise.",
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
    /// call, this one is a config-layering alias.
    #[arg(long = "config-profile", global = true, env = "HYPRPILOT_CONFIG_PROFILE")]
    config_profile: Option<String>,

    /// Override the tracing filter (otherwise `RUST_LOG` + defaults apply).
    #[arg(long, global = true, value_enum, env = "HYPRPILOT_LOG_LEVEL")]
    log_level: Option<logging::LogLevel>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the Tauri overlay + unix-socket server (default when invoked without a subcommand).
    Daemon(daemon::DaemonArgs),

    /// Dispatch a command to the running daemon via the unix socket.
    Ctl(ctl::CtlArgs),
}

fn main() -> Result<ExitCode> {
    // HACK: webkit2gtk's default DMABUF renderer triggers `Gdk-Message: Error 71
    // (Protocol error) dispatching to Wayland display` on NVIDIA + Hyprland /
    // Sway sessions. Force the legacy shared-memory path so the daemon boots
    // cleanly on those machines. Export `WEBKIT_DISABLE_DMABUF_RENDERER=0` to
    // opt out.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: runs before any thread spawns, so no data race on the env block.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    let cli = Cli::parse();

    // Bind the `WorkerGuard` to a named local so it lives until `main`
    // returns — dropping it earlier flushes file logs and silently
    // truncates tail events on crash.
    let _log_guard = logging::init(cli.log_level)?;

    let cfg = config::load(cli.config.as_deref(), cli.config_profile.as_deref())?;
    cfg.validate()?;

    match cli.command {
        None => daemon::run(cfg, daemon::DaemonArgs::default()).map(|()| ExitCode::SUCCESS),
        Some(Command::Daemon(args)) => daemon::run(cfg, args).map(|()| ExitCode::SUCCESS),
        Some(Command::Ctl(args)) => ctl::run(cfg, args),
    }
}
