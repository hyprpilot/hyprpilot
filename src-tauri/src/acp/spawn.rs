//! Agent subprocess spawner.
//!
//! Single entry point (`spawn_agent`) that resolves the vendor adapter
//! from `AgentConfig.provider`, builds the `tokio::process::Command`
//! via the existing `AcpAgent::spawn`, and returns the child + the
//! stdio pair the ACP connection takes over.

use anyhow::{bail, Context, Result};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::agents::match_provider_agent;
use crate::config::AgentConfig;

pub struct ChildStdio {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

/// Launch the configured agent, return the process + its stdio pipes.
pub fn spawn_agent(cfg: &AgentConfig) -> Result<(Child, ChildStdio)> {
    let agent = match_provider_agent(cfg.provider);
    let mut cmd = agent.spawn(cfg);
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

    Ok((child, ChildStdio { stdin, stdout }))
}
