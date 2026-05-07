//! Claude Code ACP adapter.
//!
//! Launches via `bunx --bun @agentclientprotocol/claude-agent-acp`. Model
//! selection rides on the `ANTHROPIC_MODEL` env var; the system prompt
//! goes through `FirstMessage` because the shim doesn't expose a
//! launch-time hook.

pub mod formatters;

use std::path::Path;

use serde_json::{json, Map, Value};
use tokio::process::Command;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};

pub struct AcpAgentClaudeCode;

impl AcpAgent for AcpAgentClaudeCode {
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::Env("ANTHROPIC_MODEL")
    }

    /// `@agentclientprotocol/claude-agent-acp` never reads `process.argv`;
    /// CLI flags like `--append-system-prompt` are silently dropped.
    /// The shim's only system-prompt hook is `_meta.systemPrompt` on
    /// the `session/new` request, which `agent-client-protocol` 0.11
    /// doesn't expose as a typed field. Prepending to the first
    /// `session/prompt` is the transport-agnostic path that reaches
    /// the model today.
    fn inject_system_prompt(&self, _cmd: &mut Command, prompt: &str) -> SystemPromptInjection {
        SystemPromptInjection::FirstMessage(prompt.to_string())
    }

    /// Hand the SDK a local plugin pointing at the per-instance
    /// symlink farm. `claude-agent-acp@0.30.0` extracts
    /// `_meta.claudeCode.options` and spreads it into the SDK's
    /// `Options`; `plugins: [{ type: 'local', path }]` triggers the
    /// SDK to load skills (and agents / commands / hooks) from
    /// `<path>/`. Combined with the daemon's per-profile filter,
    /// this is the only path skills take to the spawned agent today.
    fn skills_meta(&self, plugin_dir: &Path) -> Option<Map<String, Value>> {
        let mut meta = Map::new();
        meta.insert(
            "claudeCode".into(),
            json!({
                "options": {
                    "plugins": [
                        { "type": "local", "path": plugin_dir.display().to_string() }
                    ]
                }
            }),
        );
        Some(meta)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentClaudeCode;
    use crate::adapters::acp::agents::AcpAgent;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "claude-code".into(),
            provider: AgentProvider::AcpClaudeCode,
            model: model.map(|s| s.to_string()),
            command: "bunx".into(),
            args: vec!["--bun".into(), "@agentclientprotocol/claude-agent-acp".into()],
            cwd: None,
            env: BTreeMap::new(),
            thinking_budget_tokens: None,
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
    fn skills_meta_returns_local_plugin_pointing_at_dir() {
        let dir = std::path::Path::new("/tmp/hyprpilot-skills-test/instances/abc/plugin");
        let meta = AcpAgentClaudeCode.skills_meta(dir).expect("claude-code returns Some");
        let plugin = meta
            .get("claudeCode")
            .and_then(|v| v.get("options"))
            .and_then(|v| v.get("plugins"))
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .expect("nested shape matches the SDK's expected layout");
        assert_eq!(plugin.get("type").and_then(|v| v.as_str()), Some("local"));
        assert_eq!(
            plugin.get("path").and_then(|v| v.as_str()),
            Some(dir.display().to_string()).as_deref()
        );
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
            crate::adapters::acp::agents::SystemPromptInjection::FirstMessage(s) => assert_eq!(s, "be terse"),
            other => panic!("expected FirstMessage, got {other:?}"),
        }
    }
}
