//! opencode ACP adapter.
//!
//! Launches via `opencode acp` — a native binary, no `bunx` wrapper.
//! Model selection rides on the `--model` argv flag; the system prompt
//! goes through `FirstMessage` because opencode has no launch-time
//! hook.

pub mod formatters;

use tokio::process::Command;

use agent_client_protocol::schema::ToolCallUpdate;

use super::{AcpAgent, ModelInjection, SystemPromptInjection};
use crate::adapters::ToolCallState;
use crate::tools::ToolKind;

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

    fn permission_mcp_tool_with_servers(
        &self,
        update: &ToolCallUpdate,
        mcp_server_names: &[String],
    ) -> Option<ToolKind> {
        update
            .fields
            .title
            .as_deref()
            .and_then(|title| mcp_tool_from_single_underscore_name(title, mcp_server_names))
    }

    fn mcp_tool_with_servers(
        &self,
        title: &str,
        raw_input: Option<&serde_json::Value>,
        mcp_server_names: &[String],
    ) -> Option<ToolKind> {
        self.mcp_tool(title, raw_input)
            .or_else(|| mcp_tool_from_single_underscore_name(title, mcp_server_names))
    }

    fn suppress_initial_tool_call(
        &self,
        _title: &str,
        raw_input: Option<&serde_json::Value>,
        state: ToolCallState,
    ) -> bool {
        matches!(state, ToolCallState::Pending)
            && raw_input
                .and_then(|value| value.as_object())
                .map_or(true, serde_json::Map::is_empty)
    }
}

/// opencode flattens MCP tools as
/// `sanitize(server) + "_" + sanitize(tool)` (single underscore).
/// Match against the resolved per-instance server names so servers
/// containing underscores / dashes still attribute correctly.
#[must_use]
pub fn mcp_tool_from_single_underscore_name(title: &str, server_names: &[String]) -> Option<ToolKind> {
    let title = title.trim();
    let mut matches: Vec<_> = server_names
        .iter()
        .map(|name| (name, sanitize_mcp_component(name)))
        .filter(|(_, sanitized)| {
            title
                .strip_prefix(sanitized)
                .is_some_and(|rest| rest.starts_with('_') && rest.len() > 1)
        })
        .collect();
    matches.sort_by_key(|(_, sanitized)| std::cmp::Reverse(sanitized.len()));
    let (server, sanitized_server) = matches.first()?;
    let max_len = sanitized_server.len();
    if matches
        .iter()
        .filter(|(_, sanitized)| sanitized.len() == max_len)
        .count()
        > 1
    {
        return None;
    }

    let leaf = title.strip_prefix(sanitized_server.as_str())?.strip_prefix('_')?;
    if leaf.is_empty() {
        return None;
    }

    Some(ToolKind::Mcp {
        server: server.to_string(),
        tool: leaf.to_string(),
    })
}

fn sanitize_mcp_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{AgentConfig, AgentProvider};

    use super::mcp_tool_from_single_underscore_name;
    use super::AcpAgentOpenCode;
    use crate::adapters::acp::agents::AcpAgent;
    use crate::tools::ToolKind;

    fn entry_with_model(model: Option<&str>) -> AgentConfig {
        AgentConfig {
            id: "opencode".into(),
            provider: AgentProvider::AcpOpenCode,
            model: model.map(|s| s.to_string()),
            effort: None,
            command: "opencode".into(),
            args: vec!["acp".into()],
            spawn: None,
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

    #[test]
    fn mcp_tool_from_single_underscore_name_matches_longest_sanitized_server() {
        let servers = vec!["linear-kilic-dev".to_string(), "linear".to_string()];

        assert_eq!(
            mcp_tool_from_single_underscore_name("linear-kilic-dev_list_issues", &servers),
            Some(ToolKind::Mcp {
                server: "linear-kilic-dev".into(),
                tool: "list_issues".into(),
            })
        );
    }

    #[test]
    fn mcp_tool_from_single_underscore_name_uses_original_server_name() {
        let servers = vec!["my.server".to_string()];

        assert_eq!(
            mcp_tool_from_single_underscore_name("my_server_fetch", &servers),
            Some(ToolKind::Mcp {
                server: "my.server".into(),
                tool: "fetch".into(),
            })
        );
    }

    #[test]
    fn mcp_tool_from_single_underscore_name_rejects_unknown_prefixes() {
        let servers = vec!["memory".to_string()];

        assert_eq!(mcp_tool_from_single_underscore_name("read", &servers), None);
        assert_eq!(mcp_tool_from_single_underscore_name("memory", &servers), None);
    }

    #[test]
    fn mcp_tool_from_single_underscore_name_rejects_sanitized_server_collisions() {
        let servers = vec!["a.b".to_string(), "a_b".to_string()];

        assert_eq!(mcp_tool_from_single_underscore_name("a_b_fetch", &servers), None);
    }
}
