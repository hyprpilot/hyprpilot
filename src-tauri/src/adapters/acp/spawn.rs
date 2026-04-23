//! Agent subprocess spawner.
//!
//! Single entry point (`spawn_agent`) that resolves the vendor adapter
//! from `AgentConfig.provider`, builds the `tokio::process::Command`
//! via the existing `AcpAgent::spawn` + `AcpAgent::inject_system_prompt`,
//! and returns the child + the stdio pair the ACP connection takes
//! over.

use anyhow::{bail, Context, Result};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::agents::{match_provider_agent, SystemPromptInjection};
use crate::config::AgentConfig;

pub struct ChildStdio {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

/// Output of a successful spawn: the child + its stdio + the optional
/// text the runtime should prepend to the first `session/prompt` (for
/// vendors without a launch-time system-prompt hook).
pub struct SpawnedAgent {
    pub child: Child,
    pub stdio: ChildStdio,
    pub first_message_prefix: Option<String>,
}

/// Launch the configured agent. `system_prompt`, when set, is routed
/// through the vendor's `inject_system_prompt` hook — which either
/// mutates `cmd` pre-spawn (CLI flag, `-c` override, env var) or
/// returns a `FirstMessage(...)` the runtime prepends onto the first
/// `session/prompt`. Vendors without any hook silently drop it.
pub fn spawn_agent(cfg: &AgentConfig, system_prompt: Option<&str>) -> Result<SpawnedAgent> {
    let agent = match_provider_agent(cfg.provider);
    let mut cmd = agent.spawn(cfg);
    let first_message_prefix = match system_prompt {
        Some(prompt) => match agent.inject_system_prompt(&mut cmd, prompt) {
            SystemPromptInjection::Handled => None,
            SystemPromptInjection::FirstMessage(text) => Some(text),
        },
        None => None,
    };
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn agent '{}' (provider {:?})", cfg.id, cfg.provider))?;

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdin not captured — check Stdio::piped()", cfg.id),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdout not captured — check Stdio::piped()", cfg.id),
    };

    Ok(SpawnedAgent {
        child,
        stdio: ChildStdio { stdin, stdout },
        first_message_prefix,
    })
}
