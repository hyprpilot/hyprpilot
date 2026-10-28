mod completion;
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
use clap::{CommandFactory, Parser, Subcommand, ValueHint};
use clap_complete::{ArgValueCandidates, CompleteEnv};
use path_absolutize::Absolutize;

#[derive(Parser, Debug)]
#[command(
    name = "hyprpilot",
    version,
    about = "Hyprpilot: resolve a profile from layered config and launch the vendor's native CLI.",
    long_about = None,
)]
struct Cli {
    /// Path to the global config.toml (overrides the XDG default).
    #[arg(long, global = true, env = "HYPRPILOT_CONFIG", value_hint = ValueHint::FilePath)]
    config: Option<PathBuf>,

    /// Config-layer overlay to fold after the root config. Repeatable.
    ///
    /// Resolved to `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.<ext>`.
    /// Overlays fold in the given order, so a later one wins by id. The
    /// env var takes a comma-separated list. Distinct from the session
    /// `[[profiles]]` registry, addressed per call via the positional
    /// `[PROFILE]` argument.
    #[arg(
        long = "config-profile",
        value_name = "NAME",
        global = true,
        env = "HYPRPILOT_CONFIG_PROFILE",
        value_delimiter = ',',
        add = ArgValueCandidates::new(completion::config_profiles)
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

    /// Print the shell completion registration script, e.g.
    /// `eval "$(hyprpilot completion zsh)"`. Same script as
    /// `COMPLETE=zsh hyprpilot`.
    Completion(completion::CompletionArgs),
}

impl Cli {
    /// Load the config QUIETLY (before any subscriber is installed) so
    /// its `[logging] level` can feed the one-and-only tracing filter —
    /// no early-init/late-reload dance. `init` then installs the
    /// subscriber once with the fully-resolved level, and the
    /// "config: loaded" line is emitted AFTER, so it honours that level
    /// (e.g. `--log-level error` / `[logging] level = "error"` suppress
    /// it along with every other info line).
    fn load_config(&self) -> Result<config::Config> {
        let cfg = config::load(self.config.as_deref(), &self.config_profiles)?;
        logging::init(self.log_level, cfg.logging.level)?;
        tracing::info!(
            config = ?self.config,
            config_profiles = ?self.config_profiles,
            agents = cfg.agents.agents.len(),
            profiles = cfg.profiles.len(),
            default_profile = ?cfg.profile.default,
            "config: loaded"
        );

        Ok(cfg)
    }

    /// The layer selection, for the sidecars this launch injects.
    /// Absolute, because the injected harness re-reads it from the
    /// launch's `--cwd`, not from here.
    fn config_source(&self) -> Result<mcp::server::ConfigSource> {
        Ok(mcp::server::ConfigSource {
            path: self
                .config
                .as_deref()
                .map(|path| path.absolutize().map(|p| p.into_owned()))
                .transpose()?,
            profiles: self.config_profiles.clone(),
        })
    }
}

fn main() -> Result<ExitCode> {
    CompleteEnv::with_factory(Cli::command)
        .var(completion::VAR)
        .shells(completion::SHELLS)
        .complete();
    let mut cli = Cli::parse();

    match cli.command.take() {
        // Bare `hyprpilot [PROFILE]` IS the launch: pick the profile
        // interactively when none is given, then exec into the
        // resolved vendor CLI.
        None => {
            let cfg = cli.load_config()?;
            cfg.validate()?;
            let source = cli.config_source()?;
            spawn::run(cfg, cli.launch, source)
        }
        Some(Command::Profiles(args)) => {
            // A subcommand is not a launch: launch-only args (positional
            // profile, `--cwd`, `--mode`, trailing `-- <args>`) can't
            // apply, so reject them rather than silently dropping the
            // launch intent. `--with-config` is the one launch flag the
            // listing honors — the table reflects the same overlay a
            // launch would fold.
            cli.launch.reject_launch_only_args("profiles", true)?;
            let cfg = cli.load_config()?;
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
            cli.load_config()?;
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(args.run(cli.config_source()?))?;
            Ok(ExitCode::SUCCESS)
        }
        // Printing the script must not depend on a valid config.
        Some(Command::Completion(args)) => {
            cli.launch.reject_launch_only_args("completion", false)?;
            completion::run(&args)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
