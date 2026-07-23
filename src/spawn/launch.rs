use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::Args;
use serde_json::Value;

use super::SpawnRequest;
use crate::config::with_config::WithConfigArgs;
use crate::config::Config;

#[derive(Args, Debug, Default)]
pub struct LaunchArgs {
    /// Session profile id to resolve and launch directly in the provider TUI. Omit to pick interactively.
    #[arg(value_name = "PROFILE")]
    profile_id: Option<String>,
    /// Working directory for the provider process. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    cwd: Option<PathBuf>,
    /// Provider-specific mode override mapped to the direct CLI where supported.
    #[arg(long)]
    mode: Option<String>,
    #[command(flatten)]
    with_config: WithConfigArgs,
    /// Extra arguments forwarded verbatim to the spawned provider CLI.
    #[arg(last = true, allow_hyphen_values = true, value_name = "ARG")]
    provider_args: Vec<String>,
}

impl LaunchArgs {
    /// Reject launch-only arguments that a subcommand can't honor.
    ///
    /// `LaunchArgs` is flattened at the CLI root, so clap happily parses
    /// `hyprpilot engineer profiles` (positional + subcommand) or
    /// `hyprpilot --cwd x profiles` (launch flag + subcommand) — the
    /// subcommand then wins the dispatch and the launch args used to be
    /// **silently dropped**. Surface them as a hard error instead.
    ///
    /// `--with-config` is the one launch flag some subcommands honor
    /// (`profiles` folds the same overlay a launch would); pass
    /// `allow_with_config = true` there and `false` where the subcommand
    /// ignores it (`mcp`), so an unhonored overlay never silently
    /// vanishes either.
    pub fn reject_launch_only_args(&self, subcommand: &str, allow_with_config: bool) -> Result<()> {
        let mut offenders = Vec::new();
        if self.profile_id.is_some() {
            offenders.push("positional <PROFILE>");
        }
        if self.cwd.is_some() {
            offenders.push("--cwd");
        }
        if self.mode.is_some() {
            offenders.push("--mode");
        }
        if !self.provider_args.is_empty() {
            offenders.push("trailing `-- <provider args>`");
        }
        if !allow_with_config && !self.with_config.with_config.is_empty() {
            offenders.push("--with-config");
        }
        if !offenders.is_empty() {
            bail!(
                "`{subcommand}` is a subcommand, not a launch — these launch-only arguments do not apply to it: {}. \
                 Run the launch and `{subcommand}` as separate invocations.",
                offenders.join(", ")
            );
        }
        Ok(())
    }

    /// The `--with-config` overlay patches — the one launch flag a
    /// subcommand (`profiles`) folds so its output mirrors a launch.
    pub fn into_config_patches(self) -> Result<Vec<Value>> {
        self.with_config.into_patches()
    }
}

pub fn run(cfg: Config, args: LaunchArgs) -> Result<ExitCode> {
    // Whether `--with-config -` will drain stdin — captured BEFORE
    // `into_patches()` consumes it, so `launch_profile` knows stdin is
    // no longer available as a headless prompt source.
    let stdin_consumed = args.with_config.consumes_stdin();
    let config_patches = args.with_config.into_patches()?;

    super::launch_profile(
        cfg,
        SpawnRequest {
            profile_id: args.profile_id,
            // Only the EXPLICIT `--cwd` flag rides through here. The
            // `current_dir()` fallback is applied last, inside
            // `launch_profile`, so a configured profile/agent `cwd` is
            // not clobbered when the flag is omitted.
            cwd: args.cwd,
            mode: args.mode,
            config_patches,
            provider_args: args.provider_args,
            stdin_consumed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcommand_accepts_launch_args_with_no_launch_only_flags() {
        LaunchArgs::default()
            .reject_launch_only_args("profiles", true)
            .expect("bare default carries no launch-only args");
    }

    #[test]
    fn subcommand_rejects_positional_profile() {
        let args = LaunchArgs {
            profile_id: Some("engineer".into()),
            ..Default::default()
        };
        let err = args
            .reject_launch_only_args("profiles", true)
            .expect_err("positional profile + subcommand must error");
        assert!(err.to_string().contains("positional <PROFILE>"), "{err}");
    }

    #[test]
    fn subcommand_rejects_cwd_mode_and_provider_args() {
        let args = LaunchArgs {
            cwd: Some(PathBuf::from("/tmp")),
            mode: Some("plan".into()),
            provider_args: vec!["--resume".into()],
            ..Default::default()
        };
        let msg = args
            .reject_launch_only_args("profiles", true)
            .expect_err("launch flags + subcommand must error")
            .to_string();
        assert!(msg.contains("--cwd"), "{msg}");
        assert!(msg.contains("--mode"), "{msg}");
        assert!(msg.contains("provider args"), "{msg}");
    }

    #[test]
    fn profiles_honors_with_config_but_mcp_rejects_it() {
        let mut args = LaunchArgs::default();
        args.with_config.with_config = vec!["@{}".into()];

        // `profiles` folds the overlay, so `--with-config` is allowed.
        args.reject_launch_only_args("profiles", true)
            .expect("profiles honors --with-config");

        // `mcp` ignores it — reject so it isn't silently dropped.
        let err = args
            .reject_launch_only_args("mcp", false)
            .expect_err("mcp must reject an unhonored --with-config");
        assert!(err.to_string().contains("--with-config"), "{err}");
    }
}
