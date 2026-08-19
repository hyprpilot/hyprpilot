use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
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
    /// Inline prompt that runs the launch headless (non-interactive) —
    /// the non-pipe alternative to `echo … | hyprpilot <profile>`.
    #[arg(short = 'p', long = "prompt", value_name = "PROMPT", conflicts_with = "file")]
    prompt: Option<String>,
    /// Read the headless prompt from a file (`~` / `$VAR` / relative
    /// paths expanded). Mutually exclusive with `--prompt`.
    #[arg(short = 'f', long = "file", value_name = "PATH")]
    file: Option<PathBuf>,
    /// Working directory for the provider process. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    cwd: Option<PathBuf>,
    /// Provider-specific mode override mapped to the direct CLI where supported.
    #[arg(long)]
    mode: Option<String>,
    /// Continue a previous conversation: bare opens the vendor's session
    /// picker, `--resume=<session-id>` continues that one. opencode has
    /// no picker, and no vendor offers one headless.
    #[arg(long, value_name = "SESSION", num_args = 0..=1, require_equals = true, conflicts_with = "resume_last")]
    resume: Option<Option<String>>,
    /// Continue the most recent conversation without a picker — the one
    /// resume shape every vendor supports.
    #[arg(long = "resume-last")]
    resume_last: bool,
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
        if self.prompt.is_some() {
            offenders.push("--prompt");
        }
        if self.file.is_some() {
            offenders.push("--file");
        }
        if self.cwd.is_some() {
            offenders.push("--cwd");
        }
        if self.mode.is_some() {
            offenders.push("--mode");
        }
        if self.resume.is_some() {
            offenders.push("--resume");
        }
        if self.resume_last {
            offenders.push("--resume-last");
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

    /// Which conversation `--resume` / `--resume-last` continue. clap
    /// `conflicts_with` guarantees at most one is set, and a bare
    /// `--resume` arrives as `Some(None)` — the picker.
    fn resume(&self) -> Option<super::providers::Resume> {
        if self.resume_last {
            return Some(super::providers::Resume::Last);
        }
        match self.resume.as_ref()? {
            Some(session) => Some(super::providers::Resume::Session(session.clone())),
            None => Some(super::providers::Resume::Picker),
        }
    }

    /// The explicit headless prompt from `--prompt` (inline) or
    /// `--file` (contents). clap `conflicts_with` guarantees at most one
    /// is set. `None` when neither is given — the launch then falls back
    /// to piped stdin. A `--file` read error is surfaced cleanly, not a
    /// panic.
    fn prompt_override(&self) -> Result<Option<String>> {
        if let Some(prompt) = &self.prompt {
            return Ok(Some(prompt.clone()));
        }
        if let Some(file) = &self.file {
            let path = crate::paths::resolve_user(&file.to_string_lossy());
            let body =
                std::fs::read_to_string(&path).with_context(|| format!("could not read --file {}", path.display()))?;
            return Ok(Some(body));
        }
        Ok(None)
    }
}

pub fn run(cfg: Config, args: LaunchArgs) -> Result<ExitCode> {
    // Resolve the explicit `--prompt` / `--file` override BEFORE
    // `into_patches()` moves `args.with_config`.
    let prompt = args.prompt_override()?;
    let resume = args.resume();
    // Whether `--with-config -` will drain stdin — captured BEFORE
    // `into_patches()` consumes it, so `launch_profile` knows stdin is
    // no longer available as a headless prompt source.
    let stdin_consumed = args.with_config.consumes_stdin();
    let config_patches = args.with_config.into_patches()?;

    super::launch_profile(
        cfg,
        SpawnRequest {
            profile_id: args.profile_id,
            prompt,
            // Only the EXPLICIT `--cwd` flag rides through here. The
            // `current_dir()` fallback is applied last, inside
            // `launch_profile`, so a configured profile/agent `cwd` is
            // not clobbered when the flag is omitted.
            cwd: args.cwd,
            mode: args.mode,
            config_patches,
            provider_args: args.provider_args,
            stdin_consumed,
            // A CLI launch inherits its depth rather than being told
            // one: `hyprpilot <profile>` run from inside a delegate's
            // own shell is still at that delegate's depth, so it must
            // not hand itself a harness the delegate was denied.
            spawn_depth: std::env::var(crate::mcp::server::harness::DEPTH_ENV)
                .ok()
                .and_then(|raw| raw.parse::<usize>().ok())
                .unwrap_or(0),
            // No launching harness on this path to speak for delegates.
            mcp_overlay: None,
            resume,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `require_equals` is what keeps `hyprpilot --resume engineer` from
    /// reading the profile as a session id — the positional and the
    /// optional value would otherwise compete for the same token.
    #[test]
    fn resume_flags_parse_into_their_intents() {
        use clap::Parser;

        #[derive(Parser)]
        struct Harness {
            #[command(flatten)]
            launch: LaunchArgs,
        }

        let parse = |argv: &[&str]| Harness::try_parse_from(argv).map(|parsed| parsed.launch);

        assert_eq!(parse(&["hyprpilot"]).unwrap().resume(), None);
        assert_eq!(
            parse(&["hyprpilot", "--resume"]).unwrap().resume(),
            Some(super::super::providers::Resume::Picker)
        );
        assert_eq!(
            parse(&["hyprpilot", "--resume=abc123"]).unwrap().resume(),
            Some(super::super::providers::Resume::Session("abc123".into()))
        );
        assert_eq!(
            parse(&["hyprpilot", "--resume-last"]).unwrap().resume(),
            Some(super::super::providers::Resume::Last)
        );

        let launch = parse(&["hyprpilot", "--resume", "engineer"]).expect("the profile stays positional");
        assert_eq!(launch.profile_id.as_deref(), Some("engineer"));
        assert_eq!(launch.resume(), Some(super::super::providers::Resume::Picker));

        parse(&["hyprpilot", "--resume", "--resume-last"]).expect_err("the two intents conflict");
    }

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
    fn subcommand_rejects_prompt_and_file() {
        let args = LaunchArgs {
            prompt: Some("do it".into()),
            ..Default::default()
        };
        let msg = args
            .reject_launch_only_args("profiles", true)
            .expect_err("--prompt + subcommand must error")
            .to_string();
        assert!(msg.contains("--prompt"), "{msg}");

        let args = LaunchArgs {
            file: Some(PathBuf::from("/tmp/prompt.md")),
            ..Default::default()
        };
        let msg = args
            .reject_launch_only_args("profiles", true)
            .expect_err("--file + subcommand must error")
            .to_string();
        assert!(msg.contains("--file"), "{msg}");
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
