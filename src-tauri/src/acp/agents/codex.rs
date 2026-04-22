//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use tokio::process::Command;

use crate::config::AgentConfig;

use super::AcpAgent;

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
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentCodex;
    use crate::acp::agents::AcpAgent;

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
}
