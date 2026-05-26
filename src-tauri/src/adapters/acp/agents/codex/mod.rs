//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. Model selection
//! rides on a `-c model="..."` TOML override; the system prompt rides
//! on a `-c instructions="..."` TOML override.

pub mod formatters;

use tokio::process::Command;

use agent_client_protocol::schema::ToolCallUpdate;
use serde_json::Value;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};

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

    fn permission_tool_name(&self, update: &ToolCallUpdate) -> Option<String> {
        let raw = update.fields.raw_input.as_ref()?;
        let (server, tool) = mcp_approval_identity(raw)?;

        Some(format!("mcp__{server}__{tool}"))
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

fn mcp_approval_identity(raw: &Value) -> Option<(String, String)> {
    let request = raw.get("request").unwrap_or(raw);
    let meta = approval_meta(request).or_else(|| approval_meta(raw))?;
    if meta.get("codex_approval_kind").and_then(Value::as_str) != Some("mcp_tool_call") {
        return None;
    }

    if let Some(tool_title) = meta
        .get("tool_title")
        .or_else(|| request.get("tool_title"))
        .and_then(Value::as_str)
    {
        if let Some((server, tool)) = tool_title.split_once('/') {
            if !server.trim().is_empty() && !tool.trim().is_empty() {
                return Some((server.to_string(), tool.to_string()));
            }
        }
    }

    let message = request
        .get("message")
        .or_else(|| raw.get("message"))
        .and_then(Value::as_str)?;
    let server = message.strip_prefix("Allow the ")?.split_once(" MCP server")?.0.trim();
    let tool = message.split_once("run tool \"")?.1.split_once('"')?.0.trim();

    if server.is_empty() || tool.is_empty() {
        return None;
    }

    Some((server.to_string(), tool.to_string()))
}

fn approval_meta(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value
        .get("_meta")
        .or_else(|| value.get("meta"))
        .and_then(Value::as_object)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agent_client_protocol::schema::{ToolCallId, ToolCallUpdate, ToolCallUpdateFields};
    use serde_json::json;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentCodex;
    use crate::adapters::acp::agents::AcpAgent;

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
    fn mcp_approval_permission_tool_name_uses_canonical_mcp_name() {
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
            AcpAgentCodex.permission_tool_name(&update).as_deref(),
            Some("mcp__hyprpilot__read_skill")
        );
    }

    #[test]
    fn mcp_approval_permission_tool_name_accepts_legacy_meta_shape() {
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
            AcpAgentCodex.permission_tool_name(&update).as_deref(),
            Some("mcp__hyprpilot__read_skill")
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
