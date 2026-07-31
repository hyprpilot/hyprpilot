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

    // Load the config QUIETLY (before any subscriber is installed) so
    // its `[logging] level` can feed the one-and-only tracing filter —
    // no early-init/late-reload dance. `init` then installs the
    // subscriber once with the fully-resolved level, and the
    // "config: loaded" line is emitted AFTER, so it honours that level
    // (e.g. `--log-level error` / `[logging] level = "error"` suppress
    // it along with every other info line).
    let cfg = config::load(cli.config.as_deref(), cli.config_profile.as_deref())?;
    logging::init(cli.log_level, cfg.logging.level)?;
    tracing::info!(
        config = ?cli.config,
        config_profile = ?cli.config_profile,
        agents = cfg.agents.agents.len(),
        profiles = cfg.profiles.len(),
        default_profile = ?cfg.profile.default,
        "config: loaded"
    );

    match cli.command {
        // Bare `hyprpilot [PROFILE]` IS the launch: pick the profile
        // interactively when none is given, then exec into the
        // resolved vendor CLI.
        None => {
            cfg.validate()?;
            spawn::run(cfg, cli.launch)
        }
        Some(Command::Profiles(args)) => {
            // A subcommand is not a launch: launch-only args (positional
            // profile, `--cwd`, `--mode`, trailing `-- <args>`) can't
            // apply, so reject them rather than silently dropping the
            // launch intent. `--with-config` is the one launch flag the
            // listing honors — the table reflects the same overlay a
            // launch would fold.
            cli.launch.reject_launch_only_args("profiles", true)?;
            cfg.validate()?;
            let patches = cli.launch.into_config_patches()?;
            profiles::run(cfg, patches, args)
        }
        Some(Command::Mcp(args)) => {
            // The sidecar honors none of the launch flags — reject
            // `--with-config` too (`allow_with_config = false`).
            cli.launch.reject_launch_only_args("mcp", false)?;
            // The MCP sidecar consumes only its own `--skill-dir` args —
            // it never touches the launch/profile config — so
            // `validate()` is skipped deliberately: an invalid launch
            // config (e.g. an empty `[[profiles]]` list) must NOT kill
            // the skills sidecar the vendor respawns over stdio.
            // The harness tools DO need a validated config, so the
            // config source rides through — they load it lazily and
            // report a failure as a tool error, preserving the
            // "invalid config must not kill the skills sidecar"
            // invariant above.
            let source = mcp::server::ConfigSource {
                path: cli.config.clone(),
                profile: cli.config_profile.clone(),
            };
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(args.run(source))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
