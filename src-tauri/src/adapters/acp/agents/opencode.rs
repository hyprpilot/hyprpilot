//! opencode ACP adapter.
//!
//! Launches via `opencode acp` — a native binary, no `bunx` wrapper.
//! Model selection rides on the `--model` argv flag; the system prompt
//! goes through `FirstMessage` because opencode has no launch-time
//! hook.

use tokio::process::Command;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};

pub struct AcpAgentOpenCode;

impl AcpAgent for AcpAgentOpenCode {
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::Env("OPENCODE_MODEL")
    }

    /// opencode has no launch-time hook; the runtime prepends the
    /// returned string to the first `session/prompt` text block.
    fn inject_system_prompt(&self, _cmd: &mut Command, prompt: &str) -> SystemPromptInjection {
        SystemPromptInjection::FirstMessage(prompt.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentOpenCode;
    use crate::adapters::acp::agents::AcpAgent;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "opencode".into(),
            provider: AgentProvider::AcpOpenCode,
            model: model.map(|s| s.to_string()),
            command: "opencode".into(),
            args: vec!["acp".into()],
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    fn env_value(cmd: &tokio::process::Command, key: &str) -> Option<String> {
        cmd.as_std()
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(key))
            .and_then(|(_, v)| v.map(|vv| vv.to_string_lossy().into_owned()))
    }

    #[test]
    fn model_sets_opencode_model_env() {
        let entry = entry_with_model(Some("claude-sonnet-4-5"));
        let cmd = AcpAgentOpenCode.spawn(&entry);

        assert_eq!(env_value(&cmd, "OPENCODE_MODEL").as_deref(), Some("claude-sonnet-4-5"));
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();

        assert!(!args.contains(&"--model"), "model goes through env, not argv: {args:?}");
    }

    #[test]
    fn user_env_wins_over_config_model() {
        let mut entry = entry_with_model(Some("claude-sonnet-4-5"));

        entry.env.insert("OPENCODE_MODEL".into(), "claude-opus-4-5".into());
        let cmd = AcpAgentOpenCode.spawn(&entry);
        // User's explicit env entry beats the config-driven model
        // injection (the trait default's "user value wins" rule).
        assert_eq!(env_value(&cmd, "OPENCODE_MODEL").as_deref(), Some("claude-opus-4-5"));
    }

    #[test]
    fn no_model_means_no_opencode_model_env() {
        let entry = entry_with_model(None);
        let cmd = AcpAgentOpenCode.spawn(&entry);

        assert!(env_value(&cmd, "OPENCODE_MODEL").is_none());
    }

    #[test]
    fn inject_returns_first_message_and_leaves_cmd_untouched() {
        let entry = entry_with_model(None);
        let mut cmd = AcpAgentOpenCode.spawn(&entry);
        let before: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        let out = AcpAgentOpenCode.inject_system_prompt(&mut cmd, "be terse");
        let after: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        assert_eq!(before, after, "opencode must not mutate args from inject");
        match out {
            crate::adapters::acp::agents::SystemPromptInjection::FirstMessage(s) => assert_eq!(s, "be terse"),
            other => panic!("expected FirstMessage, got {other:?}"),
        }
    }
}
