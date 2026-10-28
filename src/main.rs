mod config;
mod logging;
mod mcp;
mod paths;
mod profiles;
mod resolve;
mod spawn;
mod watch;

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

    /// Name of a config-layer overlay (resolved to
    /// `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.{toml,json,yaml,yml}`).
    /// Repeatable; overlays fold in the given order after the root
    /// config, so a later one wins by id. The env var takes a
    /// comma-separated list. Distinct from the session `[[profiles]]`
    /// registry driving agent + system-prompt overlays — session
    /// profiles are addressed per call via the positional `[PROFILE]`
    /// argument.
    #[arg(
        long = "config-profile",
        value_name = "NAME",
        global = true,
        env = "HYPRPILOT_CONFIG_PROFILE",
        value_delimiter = ','
    )]
    config_profiles: Vec<String>,

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
    /// Run one of the in-tree MCP servers for an agent vendor to spawn
    /// via stdio. Each server is its own subcommand; the launcher
    /// auto-injects the ones the resolved `[mcp]` block enables.
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
    let cfg = config::load(cli.config.as_deref(), &cli.config_profiles)?;
    logging::init(cli.log_level, cfg.logging.level)?;
    tracing::info!(
        config = ?cli.config,
        config_profiles = ?cli.config_profiles,
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
            // The MCP sidecar consumes only its own argv (`--skill-dir`,
            // `--prompt-dir`, `--prompt-file`, …) —
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
                profiles: cli.config_profiles.clone(),
            };
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(args.run(source))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, so the `HYPRPILOT_CONFIG_PROFILE` mutation cannot race
    /// a sibling under `cargo test`'s shared process.
    #[test]
    fn config_profiles_keep_command_line_order_and_split_the_env_list() {
        let parse = |argv: &[&str]| Cli::try_parse_from(argv).expect("parses").config_profiles;
        let saved = std::env::var_os("HYPRPILOT_CONFIG_PROFILE");

        std::env::remove_var("HYPRPILOT_CONFIG_PROFILE");
        assert!(parse(&["hyprpilot"]).is_empty());
        assert_eq!(
            parse(&["hyprpilot", "--config-profile", "personal", "--config-profile=work"]),
            ["personal", "work"]
        );
        assert_eq!(
            parse(&["hyprpilot", "mcp", "harness", "--config-profile", "work"]),
            ["work"],
            "global, so it parses after a subcommand too"
        );

        std::env::set_var("HYPRPILOT_CONFIG_PROFILE", "personal,work");
        assert_eq!(parse(&["hyprpilot"]), ["personal", "work"]);
        assert_eq!(
            parse(&["hyprpilot", "--config-profile", "work"]),
            ["work"],
            "an explicit flag replaces the env list rather than extending it"
        );

        match saved {
            Some(value) => std::env::set_var("HYPRPILOT_CONFIG_PROFILE", value),
            None => std::env::remove_var("HYPRPILOT_CONFIG_PROFILE"),
        }
    }
}
