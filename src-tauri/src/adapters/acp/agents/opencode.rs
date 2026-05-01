//! opencode ACP adapter.
//!
//! Launches via `opencode acp` — a native binary, no `bunx` wrapper.
//! Model selection rides on the `--model` argv flag; the system prompt
//! goes through `FirstMessage` because opencode has no launch-time
//! hook.

use tokio::process::Command;

use crate::adapters::Capabilities;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};

pub struct AcpAgentOpenCode;

impl AcpAgent for AcpAgentOpenCode {
    fn command(&self) -> &'static str {
        "opencode"
    }

    fn args(&self) -> &'static [&'static str] {
        &["acp"]
    }

    fn model_injection(&self) -> ModelInjection {
        ModelInjection::Argv("--model")
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            load_session: true,
            list_sessions: true,
            permissions: true,
            terminals: true,
            mcps_per_instance: true,
            restart_with_cwd: true,
            // K-251 follow-ups — flip true when the override lands.
            session_model_switch: false,
            session_mode_switch: false,
        }
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
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn model_appends_model_flag() {
        let entry = entry_with_model(Some("claude-sonnet-4-5"));
        let cmd = AcpAgentOpenCode.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "claude-sonnet-4-5"),
            "expected --model claude-sonnet-4-5 in {args:?}"
        );
    }

    #[test]
    fn user_model_flag_wins_over_config() {
        let mut entry = entry_with_model(Some("claude-sonnet-4-5"));
        entry.args = vec!["acp".into(), "--model".into(), "claude-opus-4-5".into()];
        let cmd = AcpAgentOpenCode.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        let model_positions: Vec<_> = args.windows(2).filter(|w| w[0] == "--model").collect();
        assert_eq!(model_positions.len(), 1, "expected exactly one --model in {args:?}");
        assert_eq!(model_positions[0][1], "claude-opus-4-5");
    }

    #[test]
    fn no_model_means_no_model_flag() {
        let entry = entry_with_model(None);
        let cmd = AcpAgentOpenCode.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(!args.contains(&"--model"), "unexpected --model in {args:?}");
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
