//! Claude Code ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/claude-code-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use tokio::process::Command;

use crate::config::AgentConfig;

use super::{AcpAgent, SystemPromptInjection};

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

    /// `@zed-industries/claude-code-acp` never reads `process.argv`;
    /// CLI flags like `--append-system-prompt` are silently dropped.
    /// The shim's only system-prompt hook is `_meta.systemPrompt` on
    /// the `session/new` request, which `agent-client-protocol` 0.11
    /// doesn't expose as a typed field. Prepending to the first
    /// `session/prompt` is the transport-agnostic path that reaches
    /// the model today.
    fn inject_system_prompt(&self, _cmd: &mut Command, prompt: &str) -> SystemPromptInjection {
        SystemPromptInjection::FirstMessage(prompt.to_string())
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

    #[test]
    fn inject_returns_first_message_and_leaves_cmd_untouched() {
        let entry = entry_with_model(None);
        let mut cmd = AcpAgentClaudeCode.spawn(&entry);
        let before: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        let out = AcpAgentClaudeCode.inject_system_prompt(&mut cmd, "be terse");
        let after: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        assert_eq!(before, after, "claude-code must not mutate args from inject");
        match out {
            crate::acp::agents::SystemPromptInjection::FirstMessage(s) => assert_eq!(s, "be terse"),
            other => panic!("expected FirstMessage, got {other:?}"),
        }
    }
}
