mod acp;
mod config;
mod ctl;
mod daemon;
mod logging;
mod paths;
mod rpc;

use std::path::PathBuf;

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

    /// Active profile name (resolved to `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.toml`).
    #[arg(long, global = true, env = "HYPRPILOT_PROFILE")]
    profile: Option<String>,

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

fn main() -> Result<()> {
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

    let _log_guard = logging::init(cli.log_level)?;

    let cfg = config::load(cli.config.as_deref(), cli.profile.as_deref())?;
    cfg.validate()?;

    match cli.command {
        None => daemon::run(cfg, daemon::DaemonArgs::default()),
        Some(Command::Daemon(args)) => daemon::run(cfg, args),
        Some(Command::Ctl(args)) => ctl::run(cfg, args),
    }
}
