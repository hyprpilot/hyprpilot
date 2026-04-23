//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use tokio::process::Command;

use crate::config::AgentConfig;

use super::{AcpAgent, SystemPromptInjection};

pub struct AcpAgentCodex;

impl AcpAgent for AcpAgentCodex {
    fn command(&self) -> &'static str {
        "bunx"
    }

    fn args(&self) -> &'static [&'static str] {
        &["--bun", "@zed-industries/codex-acp"]
    }

    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program = entry.command.clone().unwrap_or_else(|| self.command().to_string());

        let args = if entry.args.is_empty() {
            self.args().iter().map(|s| (*s).to_string()).collect::<Vec<_>>()
        } else {
            entry.args.clone()
        };

        let mut final_args = args;
        // User args win — only append --model when --model not already present.
        if let Some(model) = &entry.model {
            if !entry.args.iter().any(|a| a == "--model") {
                final_args.push("--model".into());
                final_args.push(model.clone());
            }
        }

        let mut cmd = Command::new(&program);
        cmd.args(&final_args);
        cmd.envs(entry.env.iter());
        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    /// codex-acp only exposes `-c key=value` config overrides (no
    /// `--system-prompt` flag). The codex config accepts a TOML
    /// `instructions = "..."` key; we pass it through `-c` with a
    /// TOML-quoted value. Assumption: codex config key is
    /// `instructions`; document in PR body for fact-check.
    fn inject_system_prompt(&self, cmd: &mut Command, prompt: &str) -> SystemPromptInjection {
        cmd.arg("-c");
        cmd.arg(format!("instructions={}", toml_string(prompt)));
        SystemPromptInjection::Handled
    }
}

/// Serialize a prompt as a TOML basic string literal (single-line,
/// double-quoted, `\n` / `\"` escapes). `toml::Value::String` would
/// emit a multi-line literal (`"""..."""`) when the content contains
/// newlines, which can't survive shell-quoting for a `-c` CLI flag;
/// JSON strings are a subset of TOML basic strings (no `\/` emitted
/// by default), so `serde_json` gives us spec-delegated escaping
/// without hand-rolling.
fn toml_string(s: &str) -> String {
    serde_json::to_string(s).expect("str always serializes")
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
