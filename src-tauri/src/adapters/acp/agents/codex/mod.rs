//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. Model selection
//! rides on a `-c model="..."` TOML override; effort rides on
//! `-c model_reasoning_effort="..."`; the system prompt rides on a
//! `-c instructions="..."` TOML override.

pub(crate) mod approval;
pub mod formatters;

use tokio::process::Command;

use agent_client_protocol::schema::ToolCallUpdate;

use super::{AcpAgent, ConfigOptionRoute, ModelInjection, SystemPromptInjection};
use crate::adapters::{SessionConfigOptionCategory, SessionConfigOptionValue};
use crate::tools::ToolKind;

pub struct AcpAgentCodex;

impl AcpAgent for AcpAgentCodex {
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::Config("model")
    }

    fn effort_injection(&self) -> ModelInjection {
        ModelInjection::Config("model_reasoning_effort")
    }

    fn display_config_option_id(&self, id: &str) -> String {
        match id {
            "reasoning_effort" | "model_reasoning_effort" => "effort".to_string(),
            _ => id.to_string(),
        }
    }

    fn wire_config_option_id(&self, id: &str) -> String {
        match id {
            "effort" => "reasoning_effort".to_string(),
            _ => id.to_string(),
        }
    }

    fn config_option_route(&self, id: &str, value: &str, current_model: Option<&str>) -> ConfigOptionRoute {
        if id == "effort" {
            let model = current_model.unwrap_or("gpt-5.5");
            let base_model = model.split_once('/').map_or(model, |(base, _)| base);

            return ConfigOptionRoute::SetModel {
                model_id: format!("{base_model}/{value}"),
            };
        }

        ConfigOptionRoute::SetConfigOption {
            config_id: self.wire_config_option_id(id),
        }
    }

    fn augment_config_options(
        &self,
        categories: &mut Vec<SessionConfigOptionCategory>,
        configured_effort: Option<&str>,
    ) {
        add_missing_effort_option(categories, configured_effort);
    }

    fn permission_mcp_tool(&self, update: &ToolCallUpdate) -> Option<ToolKind> {
        approval::parse_mcp(update.fields.raw_input.as_ref(), &[])
            .map(|approval| approval.mcp_tool())
            .or_else(|| update.fields.title.as_deref().and_then(mcp_from_strict_dot_title))
    }

    fn mcp_tool(&self, title: &str, raw_input: Option<&serde_json::Value>) -> Option<ToolKind> {
        mcp_from_raw_input(raw_input).or_else(|| mcp_from_tool_title(title))
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

fn mcp_from_raw_input(raw_input: Option<&serde_json::Value>) -> Option<ToolKind> {
    let raw = raw_input?;
    let server = raw
        .get("server")
        .or_else(|| raw.get("server_name"))
        .and_then(|v| v.as_str())?;
    let tool = raw
        .get("tool")
        .or_else(|| raw.get("tool_name"))
        .and_then(|v| v.as_str())?;

    mcp_from_parts(server, tool)
}

fn add_missing_effort_option(categories: &mut Vec<SessionConfigOptionCategory>, configured_effort: Option<&str>) {
    if categories.iter().any(|category| category.id == "effort") {
        return;
    }

    categories.push(SessionConfigOptionCategory {
        id: "effort".into(),
        name: "Effort".into(),
        description: Some("Choose how much reasoning effort Codex should use".into()),
        current_value: configured_effort.map(str::to_string).or_else(|| Some("medium".into())),
        options: ["minimal", "low", "medium", "high"]
            .into_iter()
            .map(|value| SessionConfigOptionValue {
                value: value.into(),
                name: value.into(),
                description: None,
            })
            .collect(),
    });
}

fn mcp_from_tool_title(title: &str) -> Option<ToolKind> {
    title
        .strip_prefix("Tool: ")
        .and_then(|body| body.split_once('/'))
        .and_then(|(server, leaf)| mcp_from_parts(server, leaf))
}

fn mcp_from_strict_dot_title(title: &str) -> Option<ToolKind> {
    let title = title.split_once(" (").map_or(title, |(head, _)| head).trim();
    let (server, leaf) = title.split_once('.')?;

    if leaf.contains('.') {
        return None;
    }

    mcp_from_parts(server, leaf)
}

fn mcp_from_parts(server: &str, tool: &str) -> Option<ToolKind> {
    fn is_ident(value: &str) -> bool {
        !value.is_empty()
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    }

    let server = server.trim();
    let tool = tool.trim();

    if !is_ident(server) || !is_ident(tool) {
        return None;
    }

    Some(ToolKind::Mcp {
        server: server.to_string(),
        tool: tool.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agent_client_protocol::schema::{ToolCallId, ToolCallUpdate, ToolCallUpdateFields};
    use serde_json::json;

    use crate::config::{AgentConfig, AgentProvider};

    use super::AcpAgentCodex;
    use crate::adapters::acp::agents::AcpAgent;
    use crate::adapters::{SessionConfigOptionCategory, SessionConfigOptionValue};
    use crate::tools::ToolKind;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "codex".into(),
            provider: AgentProvider::AcpCodex,
            model: model.map(|s| s.to_string()),
            effort: None,
            command: "bunx".into(),
            args: vec!["--bun".into(), "@zed-industries/codex-acp".into()],
            spawn: None,
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
    fn effort_appends_reasoning_effort_config_override() {
        let mut entry = entry_with_model(None);
        entry.effort = Some("high".into());
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();

        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == r#"model_reasoning_effort="high""#),
            "expected -c model_reasoning_effort=\"high\" in {args:?}"
        );
    }

    #[test]
    fn user_effort_config_override_wins_over_config() {
        let mut entry = entry_with_model(None);
        entry.effort = Some("high".into());
        entry.args = vec![
            "--bun".into(),
            "@zed-industries/codex-acp".into(),
            "-c".into(),
            "model_reasoning_effort=\"medium\"".into(),
        ];
        let cmd = AcpAgentCodex.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();
        let effort_positions: Vec<_> = args
            .windows(2)
            .filter(|w| w[0] == "-c" && w[1].starts_with("model_reasoning_effort="))
            .collect();

        assert_eq!(effort_positions.len(), 1);
        assert_eq!(effort_positions[0][1], "model_reasoning_effort=\"medium\"");
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
        assert_eq!(
            AcpAgentCodex.display_config_option_id("model_reasoning_effort"),
            "effort"
        );
        assert_eq!(AcpAgentCodex.wire_config_option_id("effort"), "reasoning_effort");
        assert_eq!(AcpAgentCodex.display_config_option_id("model"), "model");
        assert_eq!(AcpAgentCodex.wire_config_option_id("model"), "model");
    }

    #[test]
    fn effort_config_option_routes_through_model_change() {
        let route = AcpAgentCodex.config_option_route("effort", "xhigh", Some("gpt-5.5/medium"));

        assert!(matches!(
            route,
            crate::adapters::acp::agents::ConfigOptionRoute::SetModel { ref model_id }
                if model_id == "gpt-5.5/xhigh"
        ));
    }

    #[test]
    fn effort_config_option_is_added_when_codex_omits_it() {
        let mut categories = vec![SessionConfigOptionCategory {
            id: "model".into(),
            name: "Model".into(),
            description: None,
            current_value: Some("gpt-5.5".into()),
            options: Vec::new(),
        }];

        AcpAgentCodex.augment_config_options(&mut categories, Some("high"));

        let effort = categories
            .iter()
            .find(|category| category.id == "effort")
            .expect("effort category added");
        assert_eq!(effort.current_value.as_deref(), Some("high"));
        assert_eq!(
            effort.options,
            vec![
                SessionConfigOptionValue {
                    value: "minimal".into(),
                    name: "minimal".into(),
                    description: None,
                },
                SessionConfigOptionValue {
                    value: "low".into(),
                    name: "low".into(),
                    description: None,
                },
                SessionConfigOptionValue {
                    value: "medium".into(),
                    name: "medium".into(),
                    description: None,
                },
                SessionConfigOptionValue {
                    value: "high".into(),
                    name: "high".into(),
                    description: None,
                },
            ]
        );
    }

    #[test]
    fn effort_config_option_does_not_duplicate_codex_native_option() {
        let mut categories = vec![SessionConfigOptionCategory {
            id: "effort".into(),
            name: "Reasoning Effort".into(),
            description: None,
            current_value: Some("low".into()),
            options: Vec::new(),
        }];

        AcpAgentCodex.augment_config_options(&mut categories, Some("high"));

        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].current_value.as_deref(), Some("low"));
    }

    #[test]
    fn mcp_approval_permission_mcp_tool_uses_metadata() {
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
            AcpAgentCodex.permission_mcp_tool(&update),
            Some(ToolKind::Mcp {
                server: "hyprpilot".to_string(),
                tool: "read_skill".to_string()
            })
        );
    }

    #[test]
    fn mcp_approval_permission_mcp_tool_accepts_legacy_meta_shape() {
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
            AcpAgentCodex.permission_mcp_tool(&update),
            Some(ToolKind::Mcp {
                server: "hyprpilot".to_string(),
                tool: "read_skill".to_string()
            })
        );
    }

    #[test]
    fn mcp_approval_permission_mcp_tool_accepts_raw_server_and_tool_fields() {
        let raw = json!({
            "server_name": "memory",
            "tool_name": "read_graph",
            "request": {
                "_meta": { "codex_approval_kind": "mcp_tool_call" },
                "message": "Allow the memory MCP server to run tool \"read_graph\"?"
            }
        });
        let update = ToolCallUpdate::new(
            ToolCallId::new("tc-1"),
            ToolCallUpdateFields::new()
                .title("Approve MCP tool call")
                .raw_input(raw),
        );

        assert_eq!(
            AcpAgentCodex.permission_mcp_tool(&update),
            Some(ToolKind::Mcp {
                server: "memory".to_string(),
                tool: "read_graph".to_string()
            })
        );
    }

    #[test]
    fn mcp_approval_permission_mcp_tool_ignores_display_tool_title() {
        let raw = json!({
            "server_name": "memory",
            "request": {
                "_meta": {
                    "codex_approval_kind": "mcp_tool_call",
                    "tool_title": "Read Graph"
                },
                "message": "Allow the memory MCP server to run tool \"read_graph\"?"
            }
        });
        let update = ToolCallUpdate::new(
            ToolCallId::new("tc-1"),
            ToolCallUpdateFields::new().title("Approve Read Graph").raw_input(raw),
        );

        assert_eq!(
            AcpAgentCodex.permission_mcp_tool(&update),
            Some(ToolKind::Mcp {
                server: "memory".to_string(),
                tool: "read_graph".to_string()
            })
        );
    }

    #[test]
    fn mcp_tool_uses_raw_server_and_tool() {
        let raw = json!({
            "server": "memory",
            "tool": "read_graph",
            "arguments": {}
        });

        assert_eq!(
            AcpAgentCodex.mcp_tool("Read package.json", Some(&raw)),
            Some(ToolKind::Mcp {
                server: "memory".to_string(),
                tool: "read_graph".to_string()
            })
        );
    }

    #[test]
    fn mcp_tool_uses_tool_title_slash_shape() {
        assert_eq!(
            AcpAgentCodex.mcp_tool("Tool: hyprpilot/read_skill", None),
            Some(ToolKind::Mcp {
                server: "hyprpilot".to_string(),
                tool: "read_skill".to_string()
            })
        );
    }

    #[test]
    fn mcp_tool_rejects_non_mcp_display_titles() {
        for title in ["Edit src/main.rs", "Read package.json", "Opening: https://example.com"] {
            assert_eq!(AcpAgentCodex.mcp_tool(title, None), None, "{title}");
        }
    }

    #[test]
    fn permission_mcp_tool_rejects_non_identifier_dot_titles() {
        for title in ["Read package.json", "Opening: https://example.com", "foo.bar.baz"] {
            let update = ToolCallUpdate::new(ToolCallId::new("tc-1"), ToolCallUpdateFields::new().title(title));

            assert_eq!(AcpAgentCodex.permission_mcp_tool(&update), None, "{title}");
        }
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
