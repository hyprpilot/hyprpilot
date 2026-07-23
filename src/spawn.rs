use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;

use crate::adapters::cli::SpawnRequest;
use crate::config::with_config::WithConfigArgs;
use crate::config::Config;

#[derive(Args, Debug, Default)]
pub struct LaunchArgs {
    /// Session profile id to resolve and launch directly in the provider TUI. Omit to pick interactively.
    #[arg(short = 'p', long = "profile", value_name = "PROFILE")]
    profile_id: Option<String>,
    /// Override the profile's agent entry.
    #[arg(long = "agent")]
    agent_id: Option<String>,
    /// Working directory for the provider process. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    cwd: Option<PathBuf>,
    /// Provider-specific mode override mapped to the direct CLI where supported.
    #[arg(long)]
    mode: Option<String>,
    /// Model override mapped to the direct CLI where supported.
    #[arg(long)]
    model: Option<String>,
    #[command(flatten)]
    with_config: WithConfigArgs,
    /// Extra arguments forwarded verbatim to the spawned provider CLI.
    #[arg(last = true, allow_hyphen_values = true, value_name = "ARG")]
    provider_args: Vec<String>,
}

pub fn run(cfg: Config, args: LaunchArgs) -> Result<ExitCode> {
    let config_patches = args.with_config.into_patches()?;

    crate::adapters::cli::run(
        cfg,
        SpawnRequest {
            profile_id: args.profile_id,
            agent_id: args.agent_id,
            // Only the EXPLICIT `--cwd` flag rides through here. The
            // `current_dir()` fallback is applied last, inside
            // `cli::run`, so a configured profile/agent `cwd` is not
            // clobbered when the flag is omitted.
            cwd: args.cwd,
            mode: args.mode,
            model: args.model,
            config_patches,
            provider_args: args.provider_args,
        },
    )
}
