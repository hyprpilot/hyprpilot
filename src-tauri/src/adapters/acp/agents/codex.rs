//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. Model selection
//! rides on the `--model` argv flag; the system prompt rides on a
//! `-c instructions="..."` TOML override.

use tokio::process::Command;

use crate::adapters::Capabilities;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};

pub struct AcpAgentCodex;

impl AcpAgent for AcpAgentCodex {
    fn command(&self) -> &'static str {
        "bunx"
    }

    fn args(&self) -> &'static [&'static str] {
        &["--bun", "@zed-industries/codex-acp"]
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

    /// codex-acp only exposes `-c key=value` overrides; the TOML
    /// `instructions` key is the system-prompt slot.
    fn inject_system_prompt(&self, cmd: &mut Command, prompt: &str) -> SystemPromptInjection {
        cmd.arg("-c");
        // JSON strings are a subset of TOML basic strings; `toml::Value::String`
        // emits multi-line `"""..."""` on newlines which breaks `-c` shell-quoting.
        cmd.arg(format!(
            "instructions={}",
            serde_json::to_string(prompt).expect("str always serializes")
        ));
        SystemPromptInjection::Handled
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentCodex;
    use crate::adapters::acp::agents::AcpAgent;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "codex".into(),
            provider: AgentProvider::AcpCodex,
            model: model.map(|s| s.to_string()),
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn model_appends_model_flag() {
        let entry = entry_with_model(Some("codex-mini-latest"));
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "codex-mini-latest"),
            "expected --model codex-mini-latest in {args:?}"
        );
    }

    #[test]
    fn user_model_flag_wins_over_config() {
        let mut entry = entry_with_model(Some("codex-mini-latest"));
        entry.args = vec![
            "--bun".into(),
            "@zed-industries/codex-acp".into(),
            "--model".into(),
            "o4-mini".into(),
        ];
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        // --model must appear exactly once and with the user value.
        let model_positions: Vec<_> = args.windows(2).filter(|w| w[0] == "--model").collect();
        assert_eq!(model_positions.len(), 1, "expected exactly one --model in {args:?}");
        assert_eq!(model_positions[0][1], "o4-mini");
    }

    #[test]
    fn no_model_means_no_model_flag() {
        let entry = entry_with_model(None);
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(!args.contains(&"--model"), "unexpected --model in {args:?}");
    }

    #[test]
    fn inject_system_prompt_appends_c_instructions_override() {
        let entry = entry_with_model(None);
        let mut cmd = AcpAgentCodex.spawn(&entry);
        let out = AcpAgentCodex.inject_system_prompt(&mut cmd, "be terse");
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == r#"instructions="be terse""#),
            "expected -c instructions=\"be terse\" in {args:?}"
        );
        assert!(matches!(
            out,
            crate::adapters::acp::agents::SystemPromptInjection::Handled
        ));
    }

    #[test]
    fn inject_system_prompt_escapes_quotes_and_newlines() {
        let entry = entry_with_model(None);
        let mut cmd = AcpAgentCodex.spawn(&entry);
        AcpAgentCodex.inject_system_prompt(&mut cmd, "say \"hi\"\nline2");
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        let want = r#"instructions="say \"hi\"\nline2""#;
        assert!(args.contains(&want), "expected {want:?} among args; got {args:?}");
    }
}
