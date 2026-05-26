//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. Model selection
//! rides on a `-c model="..."` TOML override; the system prompt rides
//! on a `-c instructions="..."` TOML override.

pub(crate) mod approval;
pub mod formatters;

use tokio::process::Command;

use agent_client_protocol::schema::ToolCallUpdate;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};
use crate::adapters::permission::ToolIdentity;

pub struct AcpAgentCodex;

impl AcpAgent for AcpAgentCodex {
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::Config("model")
    }

    fn display_config_option_id(&self, id: &str) -> String {
        match id {
            "reasoning_effort" => "effort".to_string(),
            _ => id.to_string(),
        }
    }

    fn wire_config_option_id(&self, id: &str) -> String {
        match id {
            "effort" => "reasoning_effort".to_string(),
            _ => id.to_string(),
        }
    }

    fn permission_tool_identity(&self, update: &ToolCallUpdate) -> Option<ToolIdentity> {
        approval::parse_mcp(update.fields.raw_input.as_ref(), &[]).map(|approval| approval.identity())
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

    use agent_client_protocol::schema::{ToolCallId, ToolCallUpdate, ToolCallUpdateFields};
    use serde_json::json;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentCodex;
    use crate::adapters::acp::agents::AcpAgent;
    use crate::adapters::permission::ToolIdentity;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "codex".into(),
            provider: AgentProvider::AcpCodex,
            model: model.map(|s| s.to_string()),
            command: "bunx".into(),
            args: vec!["--bun".into(), "@zed-industries/codex-acp".into()],
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn model_appends_model_config_override() {
        let entry = entry_with_model(Some("codex-mini-latest"));
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == r#"model="codex-mini-latest""#),
            "expected -c model=\"codex-mini-latest\" in {args:?}"
        );
    }

    #[test]
    fn user_model_config_override_wins_over_config() {
        let mut entry = entry_with_model(Some("codex-mini-latest"));
        entry.args = vec![
            "--bun".into(),
            "@zed-industries/codex-acp".into(),
            "-c".into(),
            "model=\"o4-mini\"".into(),
        ];
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        // -c model=... must appear exactly once and with the user value.
        let model_positions: Vec<_> = args
            .windows(2)
            .filter(|w| w[0] == "-c" && w[1].starts_with("model="))
            .collect();
        assert_eq!(
            model_positions.len(),
            1,
            "expected exactly one -c model=... in {args:?}"
        );
        assert_eq!(model_positions[0][1], "model=\"o4-mini\"");
    }

    #[test]
    fn user_long_model_config_override_wins_over_config() {
        let mut entry = entry_with_model(Some("codex-mini-latest"));
        entry.args = vec![
            "--bun".into(),
            "@zed-industries/codex-acp".into(),
            "--config=model=\"o4-mini\"".into(),
        ];
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();

        assert_eq!(
            args.iter().filter(|arg| arg.starts_with("--config=model=")).count(),
            1,
            "expected user --config=model=... to be preserved in {args:?}"
        );
        assert!(
            !args.windows(2).any(|w| w[0] == "-c" && w[1].starts_with("model=")),
            "unexpected injected -c model=... in {args:?}"
        );
    }

    #[test]
    fn no_model_means_no_model_config_override() {
        let entry = entry_with_model(None);
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(
            !args.windows(2).any(|w| w[0] == "-c" && w[1].starts_with("model=")),
            "unexpected -c model=... in {args:?}"
        );
    }

    #[test]
    fn effort_config_option_uses_common_display_id() {
        assert_eq!(AcpAgentCodex.display_config_option_id("reasoning_effort"), "effort");
        assert_eq!(AcpAgentCodex.wire_config_option_id("effort"), "reasoning_effort");
        assert_eq!(AcpAgentCodex.display_config_option_id("model"), "model");
        assert_eq!(AcpAgentCodex.wire_config_option_id("model"), "model");
    }

    #[test]
    fn mcp_approval_permission_tool_identity_uses_metadata() {
        let raw = json!({
            "server_name": "hyprpilot",
            "request": {
                "meta": {
                    "codex_approval_kind": "mcp_tool_call",
                    "tool_title": "hyprpilot/read_skill"
                },
                "message": "Allow the hyprpilot MCP server to run tool \"read_skill\"?"
            }
        });
        let update = ToolCallUpdate::new(
            ToolCallId::new("tc-1"),
            ToolCallUpdateFields::new()
                .title("Approve hyprpilot/read_skill")
                .raw_input(raw),
        );

        assert_eq!(
            AcpAgentCodex.permission_tool_identity(&update),
            Some(ToolIdentity::Mcp {
                server: "hyprpilot".to_string(),
                leaf: "read_skill".to_string()
            })
        );
    }

    #[test]
    fn mcp_approval_permission_tool_identity_accepts_legacy_meta_shape() {
        let raw = json!({
            "request": {
                "_meta": { "codex_approval_kind": "mcp_tool_call" },
                "message": "Allow the hyprpilot MCP server to run tool \"read_skill\"?"
            }
        });
        let update = ToolCallUpdate::new(
            ToolCallId::new("tc-1"),
            ToolCallUpdateFields::new()
                .title("Approve MCP tool call")
                .raw_input(raw),
        );

        assert_eq!(
            AcpAgentCodex.permission_tool_identity(&update),
            Some(ToolIdentity::Mcp {
                server: "hyprpilot".to_string(),
                leaf: "read_skill".to_string()
            })
        );
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
