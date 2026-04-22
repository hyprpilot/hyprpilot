//! Claude Code ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/claude-code-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use tokio::process::Command;

use crate::config::AgentConfig;

use super::AcpAgent;

/// Unit struct — carries no runtime state. Everything returned by its
/// methods is static per-vendor knowledge; `AgentConfig` on the spawn
/// flow provides the rest.
pub struct AcpAgentClaudeCode;

impl AcpAgent for AcpAgentClaudeCode {
    fn command(&self) -> &'static str {
        "bunx"
    }

    fn args(&self) -> &'static [&'static str] {
        &["--bun", "@zed-industries/claude-code-acp"]
    }

    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program = entry.command.clone().unwrap_or_else(|| self.command().to_string());

        let args = if entry.args.is_empty() {
            self.args().iter().map(|s| (*s).to_string()).collect::<Vec<_>>()
        } else {
            entry.args.clone()
        };

        let mut cmd = Command::new(&program);
        cmd.args(&args);
        cmd.envs(entry.env.iter());
        // User env wins — only inject ANTHROPIC_MODEL when not already set.
        if let Some(model) = &entry.model {
            if !entry.env.contains_key("ANTHROPIC_MODEL") {
                cmd.env("ANTHROPIC_MODEL", model);
            }
        }
        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentClaudeCode;
    use crate::acp::agents::AcpAgent;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "claude-code".into(),
            provider: AgentProvider::AcpClaudeCode,
            model: model.map(|s| s.to_string()),
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn model_sets_anthropic_model_env() {
        let entry = entry_with_model(Some("claude-opus-4-5"));
        let cmd = AcpAgentClaudeCode.spawn(&entry);
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        let val = envs
            .iter()
            .find(|(k, _)| *k == "ANTHROPIC_MODEL")
            .and_then(|(_, v)| v.as_ref());
        assert_eq!(val.map(|v| v.to_str().unwrap()), Some("claude-opus-4-5"));
    }

    #[test]
    fn user_anthropic_model_env_wins_over_config() {
        let mut entry = entry_with_model(Some("claude-opus-4-5"));
        entry.env.insert("ANTHROPIC_MODEL".into(), "claude-haiku-3-5".into());
        let cmd = AcpAgentClaudeCode.spawn(&entry);
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        // The env map has ANTHROPIC_MODEL from entry.env; the model field
        // must not override it.  We check the entry.env value survived.
        let val = envs
            .iter()
            .find(|(k, _)| *k == "ANTHROPIC_MODEL")
            .and_then(|(_, v)| v.as_ref());
        assert_eq!(val.map(|v| v.to_str().unwrap()), Some("claude-haiku-3-5"));
    }

    #[test]
    fn no_model_means_no_anthropic_model_env() {
        let entry = entry_with_model(None);
        let cmd = AcpAgentClaudeCode.spawn(&entry);
        let has_key = cmd.as_std().get_envs().any(|(k, _)| k == "ANTHROPIC_MODEL");
        assert!(!has_key);
    }
}
