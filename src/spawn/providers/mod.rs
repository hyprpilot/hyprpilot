//! Per-vendor native-flag projection + `exec` handoff.
//!
//! `build_command` resolves a [`SpawnCommand`] — program, argv, env,
//! cwd — from the [`ResolvedProfile`] by dispatching on the agent
//! provider, then `exec`/spawn hands the process off to the vendor
//! CLI. The heavy per-vendor projection lives in the sibling modules:
//!
//! - [`claude`] — `claude` native flags (`--model`, `--mcp-config`,
//!   `--allowedTools`, …).
//! - [`codex`] — `codex` `-c` config overrides + `exec` subcommand.
//! - [`opencode`] — `opencode` `OPENCODE_CONFIG_CONTENT` /
//!   `OPENCODE_PERMISSION` env + `run` subcommand.
//! - [`argv`] — flag-detection predicates shared by all three.
//! - [`temp`] — the launch-scoped 0600 temp-config lifecycle + reaper
//!   backing claude's by-path `--mcp-config`.
//!
//! This module keeps the dispatch, the `exec` handoff (with its
//! secret-tiered tracing + argv redaction), the infallible
//! `base_command` projection of the agent's `command`/`args`/`env`/
//! `cwd`, and the pure helpers (`ensure_inline_size`, MCP leaf-pattern
//! matching) the vendor modules share.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use anyhow::{bail, Context, Result};

use crate::config::AgentProvider;
use crate::mcp::MCPDefinition;
use crate::profile::ResolvedProfile;

mod argv;
mod claude;
mod codex;
mod opencode;
mod temp;

const INLINE_CONFIG_LIMIT: usize = 256 * 1024;

/// Harness-only projection knobs. `None` for CLI launches, which keep
/// the vendor's human-readable output — a captain running
/// `hyprpilot x -p "…"` wants prose, not an NDJSON event stream.
#[derive(Debug, Clone, Default)]
pub(crate) struct HarnessProjection {
    /// Emit the vendor's structured JSON event stream. Load-bearing for
    /// the harness: turn boundaries, usage, and the vendor session id
    /// all arrive only through it.
    pub structured_output: bool,
    /// Vendor session id to continue. `None` starts a new conversation.
    ///
    /// All three vendors mint their own id and report it in the first
    /// turn's event stream (`session_id` / `thread_id` / `sessionID` —
    /// three different keys, all verified against the installed CLIs),
    /// so the harness reads it back uniformly. claude would also accept
    /// a caller-minted `--session-id <uuid>`, but taking that shortcut
    /// for one vendor would mean two code paths and a UUID dependency
    /// to save nothing.
    pub resume: Option<String>,
}

#[derive(Debug)]
pub(crate) struct SpawnCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) cwd: Option<PathBuf>,
    /// Headless prompt to write to the child's stdin. When `Some`, the
    /// launcher SPAWNS the vendor (not `exec()`) and pipes this prompt
    /// into its stdin, then waits — the delivery path claude/codex use
    /// so the prompt reaches the vendor's stdin reader instead of riding
    /// argv (where claude's variadic `--allowedTools`/`--disallowedTools`
    /// would swallow a trailing positional, and where a consumed pipe
    /// would leave codex reading EOF). `None` keeps the `exec()` handoff
    /// (interactive, and opencode's positional-prompt headless path).
    pub(crate) stdin_prompt: Option<String>,
}

impl SpawnCommand {
    /// The resolved cwd this command will `exec()` into — same
    /// `expand_value`-expanded value `base_command` derived from the
    /// profile's `agent.cwd`. Consumed by the multiplexer title so the
    /// rename tracks the launch's actual working directory rather than
    /// the invocation cwd.
    pub(crate) fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }
}

pub(crate) fn build_command(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
    harness: Option<&HarnessProjection>,
) -> Result<SpawnCommand> {
    match resolved.agent.provider {
        AgentProvider::ClaudeCode => {
            claude::build_claude(resolved, system_prompt, mcp_defs, provider_args, prompt, harness)
        }
        AgentProvider::Codex => codex::build_codex(resolved, system_prompt, mcp_defs, provider_args, prompt, harness),
        AgentProvider::OpenCode => {
            opencode::build_opencode(resolved, system_prompt, mcp_defs, provider_args, prompt, harness)
        }
    }
}

/// Test shim for the CLI-shaped call — `build_command` with no harness
/// projection. Keeps the vendor projection tests asserting what they
/// actually care about (argv shape) instead of restating `None` for a
/// knob none of them exercise.
#[cfg(test)]
pub(crate) fn build_command_cli(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
) -> Result<SpawnCommand> {
    build_command(resolved, system_prompt, mcp_defs, provider_args, prompt, None)
}

pub(crate) fn exec(command: SpawnCommand) -> Result<ExitCode> {
    // Secrets rule: argv values (codex `-c instructions=…`, claude
    // `--append-system-prompt`) and env values carry the system prompt
    // + bearer tokens. Claude's `--mcp-config` carries only a path to a
    // 0600 temp file (see `temp::write_launch_temp_config`), so the MCP
    // header secrets that file references never reach the
    // world-readable argv. `info` shows flag + env-key names only;
    // `debug` shows argv with every value payload elided to a size;
    // `trace` — and ONLY trace — dumps the raw argv + env values. Never
    // enable trace where the log sink is shared.
    let handoff = if command.stdin_prompt.is_some() {
        "cli: spawn handoff (prompt on stdin)"
    } else {
        "cli: exec handoff"
    };
    tracing::info!(
        program = %command.program,
        cwd = ?command.cwd,
        args = ?arg_flag_names(&command.args),
        env = ?command.env.keys().collect::<Vec<_>>(),
        stdin_prompt = command.stdin_prompt.is_some(),
        "{handoff}"
    );
    tracing::debug!(argv = ?redacted_argv(&command.args), "{handoff} argv (values elided)");
    tracing::trace!(argv = ?command.args, env = ?command.env, "{handoff} raw argv + env (secrets)");

    // Headless claude/codex: the prompt rides the child's stdin, so we
    // must SPAWN (not `exec()`) to keep a handle on the pipe, write the
    // prompt, close it (EOF), and propagate the exit code. stdout/stderr
    // stay inherited, so the child can always drain them and never
    // deadlocks against our blocking stdin write.
    if let Some(prompt) = command.stdin_prompt {
        return spawn_with_stdin_prompt(
            &command.program,
            &command.args,
            &command.env,
            command.cwd.as_deref(),
            &prompt,
        );
    }

    let mut cmd = std::process::Command::new(&command.program);
    cmd.args(&command.args)
        .envs(&command.env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(cwd) = command.cwd.as_ref() {
        cmd.current_dir(cwd);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = cmd.exec();
        Err(err).with_context(|| format!("exec {}", command.program))
    }

    #[cfg(not(unix))]
    {
        let status = cmd.status().with_context(|| format!("spawning {}", command.program))?;

        Ok(status
            .code()
            .map_or_else(|| ExitCode::from(1), |code| ExitCode::from(code as u8)))
    }
}

/// Spawn the vendor with the headless prompt written to its stdin, then
/// wait and propagate the exit code. Used for claude (`--print`) and
/// codex (`exec`) headless launches, which read the prompt from stdin
/// when no positional is given.
fn spawn_with_stdin_prompt(
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
    prompt: &str,
) -> Result<ExitCode> {
    use std::io::Write;

    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }

    let mut child = cmd.spawn().with_context(|| format!("spawning {program}"))?;
    {
        let mut stdin = child.stdin.take().context("child stdin pipe missing after spawn")?;
        stdin
            .write_all(prompt.as_bytes())
            .with_context(|| format!("writing headless prompt to {program} stdin"))?;
        // `stdin` drops here, closing the pipe so the child sees EOF.
    }
    let status = child.wait().with_context(|| format!("waiting on {program}"))?;

    Ok(status
        .code()
        .map_or_else(|| ExitCode::from(1), |code| ExitCode::from(code as u8)))
}

/// Project the agent's `command`/`args`/`env`/`cwd` into a
/// [`SpawnCommand`], applying `~` + `${VAR}` expansion to each. Always
/// succeeds — expansion failures warn and fall back to the raw value —
/// so the per-vendor builders call it without a `?`.
pub(super) fn base_command(resolved: &ResolvedProfile) -> SpawnCommand {
    let program = expand_value(&resolved.agent.command, "agents.command");
    let args = resolved
        .agent
        .args
        .iter()
        .map(|arg| expand_value(arg, "agents.args"))
        .collect();
    let env = resolved
        .agent
        .env
        .iter()
        .map(|(key, value)| (key.clone(), expand_value(value, "agents.env")))
        .collect();
    let cwd = resolved
        .agent
        .cwd
        .as_ref()
        .map(|path| PathBuf::from(expand_value(&path.to_string_lossy(), "agents.cwd")));

    SpawnCommand {
        program,
        args,
        env,
        cwd,
        stdin_prompt: None,
    }
}

fn expand_value(raw: &str, ctx: &str) -> String {
    crate::paths::expand_env_value(raw, ctx, |name| std::env::var(name).ok())
}

/// Strip the `mcp__<server>__` prefix from a tool pattern, returning
/// the server-relative leaf. `None` when the pattern names a different
/// server than `server`; the pattern verbatim when it carries no
/// prefix.
pub(super) fn mcp_leaf_pattern<'a>(server: &str, pattern: &'a str) -> Option<&'a str> {
    if let Some(rest) = pattern.strip_prefix("mcp__") {
        let (prefix, leaf) = rest.split_once("__")?;
        return (prefix == server).then_some(leaf);
    }

    Some(pattern)
}

/// Whether `pattern` is an exact tool name (no glob metacharacters).
pub(super) fn is_exact_tool_name(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[')
}

/// Flag tokens from argv with any `=payload` suffix stripped — the
/// `info`-level view. Value tokens (positionals, `-c key=value`
/// bodies, `--mcp-config` / `--append-system-prompt` payloads) do not
/// start with `-`, so they are dropped entirely and no secret
/// material reaches `info`.
fn arg_flag_names(args: &[String]) -> Vec<&str> {
    args.iter()
        .filter(|arg| arg.starts_with('-'))
        .map(|arg| arg.split('=').next().unwrap_or(arg.as_str()))
        .collect()
}

/// argv with every value token replaced by a `<size[ json]>`
/// placeholder — the `debug`-level view. Flag tokens survive; value
/// payloads never do, so bearer tokens / the system prompt stay out
/// of `debug` logs while argv shape stays legible.
///
/// Also the shape the MCP harness reports as session provenance: a
/// calling agent gets to see exactly which flags a session was launched
/// with, without the `--append-system-prompt` body or an MCP config's
/// bearer tokens riding along in the tool result.
pub(crate) fn redacted_argv(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg.starts_with('-') {
                arg.clone()
            } else {
                elide_value(arg)
            }
        })
        .collect()
}

fn elide_value(value: &str) -> String {
    let kind = if value.trim_start().starts_with(['{', '[']) {
        " json"
    } else {
        ""
    };
    format!("<{}{kind}>", bytesize::ByteSize(value.len() as u64))
}

pub(super) fn ensure_inline_size(label: &str, value: &str) -> Result<()> {
    if value.len() > INLINE_CONFIG_LIMIT {
        bail!(
            "{label} is too large for inline direct CLI injection ({} bytes > {} bytes); refusing to write unmanaged temp config",
            value.len(),
            INLINE_CONFIG_LIMIT
        );
    }

    Ok(())
}

#[cfg(test)]
pub(super) mod fixtures {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    use serde_json::json;

    use crate::config::{AgentConfig, AgentProvider};
    use crate::mcp::{HyprpilotExtension, MCPDefinition};
    use crate::profile::ResolvedProfile;

    /// Serialises the two tests that touch the real process env
    /// (`set_var` / `${HOME}` expansion) so a mutation in one can't
    /// race a read in the other under threaded `cargo test`.
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) fn env_test_guard() -> MutexGuard<'static, ()> {
        ENV_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn resolved(provider: AgentProvider) -> ResolvedProfile {
        ResolvedProfile {
            agent: AgentConfig {
                id: "agent".into(),
                provider,
                model: None,
                effort: None,
                command: "provider".into(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            },
            model: Some("model-a".into()),
            effort: Some("high".into()),
            system_prompt: Vec::new(),
            mode: Some("plan".into()),
            headless: false,
        }
    }

    pub(crate) fn resolved_with_mode(provider: AgentProvider, mode: Option<&str>) -> ResolvedProfile {
        let mut resolved = resolved(provider);
        resolved.mode = mode.map(str::to_string);

        resolved
    }

    pub(crate) fn resolved_with_cwd(provider: AgentProvider, cwd: &str) -> ResolvedProfile {
        let mut resolved = resolved(provider);
        resolved.agent.cwd = Some(PathBuf::from(cwd));
        resolved.mode = None;

        resolved
    }

    /// `resolved.agent.args` already carries the resolve-time
    /// profile `args` REPLACE merge by the time `build_command`
    /// runs — that merge happens in
    /// `profile::ResolvedProfile::from_profile_explicit`,
    /// not here (pinned separately by
    /// `flat_args_replace_base_agent_args_wholesale` in
    /// `profile.rs`). This helper stands in for that
    /// already-resolved shape, isolating `effort` / `mode` (both
    /// `None`) so the ordering / suppression assertions only have to
    /// reason about `--model`.
    pub(crate) fn resolved_with_agent_args(provider: AgentProvider, args: Vec<String>) -> ResolvedProfile {
        let mut resolved = resolved(provider);
        resolved.agent.args = args;
        resolved.effort = None;
        resolved.mode = None;

        resolved
    }

    pub(crate) fn mcp_def() -> MCPDefinition {
        MCPDefinition {
            name: "hyprpilot-nvim".into(),
            raw: json!({
                "command": "uvx",
                "args": ["hyprpilot-nvim-mcp"],
                "env": { "NVIM": "/tmp/nvim.sock" },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    pub(crate) fn mcp_def_with_home_env() -> MCPDefinition {
        MCPDefinition {
            name: "shell-env".into(),
            raw: json!({
                "command": "${HOME}/bin/mcp",
                "args": ["--state", "${HOME}/state"],
                "env": { "TOKEN_FILE": "${HOME}/token" },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    pub(crate) fn mcp_def_with_permissions() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: None,
                exclude_tools: Vec::new(),
                auto_accept_tools: vec!["read_*".into(), "list_*".into()],
                auto_reject_tools: vec!["delete_*".into(), "mcp__filesystem__write_*".into()],
            },
            source: "<test>".into(),
        }
    }

    pub(crate) fn remote_mcp_def_with_headers() -> MCPDefinition {
        MCPDefinition {
            name: "github".into(),
            raw: json!({
                "url": "https://example.test/mcp",
                "headers": {
                    "Authorization": "Bearer ${NVIM_GITHUB}",
                    "X-MCP-Insiders": "true",
                    "x-api-key": "${env:EXA_API_KEY}",
                },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    pub(crate) fn mcp_def_with_visibility() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: Some(vec!["read_file".into(), "list_*".into()]),
                exclude_tools: vec!["delete_file".into()],
                auto_accept_tools: vec!["read_file".into()],
                auto_reject_tools: vec!["write_*".into()],
            },
            source: "<test>".into(),
        }
    }

    pub(crate) fn mcp_def_with_visibility_conflicts() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: Some(vec!["read_file".into()]),
                exclude_tools: vec!["delete_*".into()],
                auto_accept_tools: vec!["read_file".into(), "write_file".into(), "delete_file".into()],
                auto_reject_tools: vec!["read_file".into()],
            },
            source: "<test>".into(),
        }
    }

    pub(crate) fn sse_mcp_def() -> MCPDefinition {
        MCPDefinition {
            name: "events".into(),
            raw: json!({ "url": "https://example.test/sse", "type": "sse" }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    pub(crate) fn transportless_mcp_def() -> MCPDefinition {
        MCPDefinition {
            name: "broken".into(),
            raw: json!({ "note": "no command or url" }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    pub(crate) fn mcp_def_with_charclass_glob() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({ "command": "npx", "args": ["srv"] }),
            hyprpilot: HyprpilotExtension {
                include_tools: None,
                exclude_tools: Vec::new(),
                auto_accept_tools: vec!["read_[abc]".into()],
                auto_reject_tools: Vec::new(),
            },
            source: "<test>".into(),
        }
    }

    pub(crate) fn assert_json_key_before(body: &str, before: &str, after: &str) {
        let before_pos = body.find(&format!("\"{before}\"")).expect("before key exists");
        let after_pos = body.find(&format!("\"{after}\"")).expect("after key exists");
        assert!(before_pos < after_pos, "expected `{before}` before `{after}` in {body}");
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::config::AgentProvider;

    #[test]
    fn redaction_keeps_flags_and_elides_secret_value_payloads() {
        let secret = "You are a secret system prompt with a bearer token abc123";
        let mcp_json = r#"{"mcpServers":{"x":{"command":"y"}}}"#;
        let argv = vec![
            "--model".to_string(),
            "claude-opus".to_string(),
            "--append-system-prompt".to_string(),
            secret.to_string(),
            "--mcp-config".to_string(),
            mcp_json.to_string(),
            "-c".to_string(),
            "instructions=\"secret-instructions\"".to_string(),
        ];

        // info view: flag names only — no value ever appears.
        let flags = arg_flag_names(&argv);
        assert_eq!(flags, vec!["--model", "--append-system-prompt", "--mcp-config", "-c"]);
        let flags_joined = flags.join(" ");
        assert!(!flags_joined.contains("claude-opus"));
        assert!(!flags_joined.contains("secret"));
        assert!(!flags_joined.contains("abc123"));

        // debug view: flags survive, every value payload is elided to
        // a size (never the raw bytes).
        let redacted = redacted_argv(&argv);
        let redacted_joined = redacted.join(" ");
        assert!(redacted_joined.contains("--append-system-prompt"));
        assert!(redacted_joined.contains("--mcp-config"));
        assert!(!redacted_joined.contains("secret"));
        assert!(!redacted_joined.contains("abc123"));
        assert!(!redacted_joined.contains("claude-opus"));
        assert!(!redacted_joined.contains("instructions="));
        // json payloads annotate their kind; sizes are rendered.
        assert!(redacted.iter().any(|arg| arg.ends_with("json>")), "{redacted:?}");
        assert!(redacted
            .iter()
            .any(|arg| arg == &format!("<{}>", bytesize::ByteSize(secret.len() as u64))));
    }

    /// `${VAR}` expansion runs AFTER the profile-level `env`
    /// overlay, not before. `resolved.agent.env` here stands in for
    /// the already-overlaid map `profile::from_profile_explicit`
    /// produces (pinned separately by
    /// `flat_env_overlays_onto_agent_env_at_resolve` in `profile.rs`)
    /// — this test pins the SECOND half of that pipeline: an
    /// override-authored env value participates in `expand_value`
    /// exactly like an agent-authored one.
    #[test]
    fn agent_env_expansion_runs_after_override_overlay() {
        let _guard = env_test_guard();
        std::env::set_var("HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR", "expanded-value");
        let mut resolved = resolved(AgentProvider::ClaudeCode);

        resolved.agent.env.insert(
            "OVERRIDE_OVERLAID".into(),
            "${HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR}".into(),
        );

        let command = build_command(&resolved, None, &[], vec![], None, None).unwrap();

        assert_eq!(
            command.env.get("OVERRIDE_OVERLAID").map(String::as_str),
            Some("expanded-value")
        );
        std::env::remove_var("HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR");
    }

    #[test]
    fn ensure_inline_size_rejects_oversized_payload() {
        let big = "x".repeat(INLINE_CONFIG_LIMIT + 1);
        let err = ensure_inline_size("test payload", &big).expect_err("oversized payload must error");

        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn ensure_inline_size_accepts_within_limit() {
        assert!(ensure_inline_size("test payload", "small").is_ok());
        assert!(ensure_inline_size("test payload", &"x".repeat(INLINE_CONFIG_LIMIT)).is_ok());
    }
}
