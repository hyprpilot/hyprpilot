//! Agent subprocess spawner.
//!
//! Single entry point (`spawn_agent`) that resolves the vendor adapter
//! from `AgentConfig.provider`, builds the `tokio::process::Command`
//! via the existing `AcpAgent::spawn` + `AcpAgent::inject_system_prompt`,
//! and returns the child + the stdio pair the ACP connection takes
//! over.

use anyhow::{bail, Context, Result};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tracing::{error, info};

use super::agents::{match_provider_agent, SystemPromptInjection};
use crate::config::AgentConfig;

pub struct ChildStdio {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

/// Output of a successful spawn: the child + its stdio + the optional
/// text the runtime should prepend to the first `session/prompt` (for
/// vendors without a launch-time system-prompt hook). `stderr` is
/// captured (not inherited) so vendor-SDK cleanup noise stays out of
/// the parent terminal — the runtime drains it into tracing.
pub struct SpawnedAgent {
    pub child: Child,
    pub stdio: ChildStdio,
    pub stderr: ChildStderr,
    pub first_message_prefix: Option<String>,
}

/// Launch the configured agent. `system_prompt`, when set, is routed
/// through the vendor's `inject_system_prompt` hook — which either
/// mutates `cmd` pre-spawn (CLI flag, `-c` override, env var) or
/// returns a `FirstMessage(...)` the runtime prepends onto the first
/// `session/prompt`. Vendors without any hook silently drop it.
pub fn spawn_agent(cfg: &AgentConfig, system_prompt: Option<&str>) -> Result<SpawnedAgent> {
    info!(
        agent = %cfg.id,
        provider = ?cfg.provider,
        cwd = ?cfg.cwd,
        command = ?cfg.command,
        has_system_prompt = system_prompt.is_some(),
        "acp::spawn: launching agent subprocess"
    );

    let agent = match_provider_agent(cfg.provider);
    let mut cmd = agent.spawn(cfg);
    // Centralize stderr capture here rather than duplicating it across
    // every vendor agent. Vendor SDKs (notably claude-agent-sdk under
    // claude-code-acp) print noisy cleanup stack traces to stderr on
    // shutdown; piping keeps that out of the parent terminal and lets
    // the runtime drain it into tracing.
    cmd.stderr(std::process::Stdio::piped());
    let first_message_prefix = match system_prompt {
        Some(prompt) => match agent.inject_system_prompt(&mut cmd, prompt) {
            SystemPromptInjection::Handled => None,
            SystemPromptInjection::FirstMessage(text) => Some(text),
        },
        None => None,
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            error!(agent = %cfg.id, provider = ?cfg.provider, %err, "acp::spawn: failed to spawn agent");
            return Err(err)
                .with_context(|| format!("failed to spawn agent '{}' (provider {:?})", cfg.id, cfg.provider));
        }
    };

    let pid = child.id();

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdin not captured — check Stdio::piped()", cfg.id),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdout not captured — check Stdio::piped()", cfg.id),
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => bail!("agent '{}' stderr not captured — check Stdio::piped()", cfg.id),
    };

    info!(
        agent = %cfg.id,
        pid = ?pid,
        first_message_injection = first_message_prefix.is_some(),
        "acp::spawn: agent subprocess spawned"
    );

    Ok(SpawnedAgent {
        child,
        stdio: ChildStdio { stdin, stdout },
        stderr,
        first_message_prefix,
    })
}
