//! Tool-name attribution shared by the permission controller and
//! anywhere else that needs to attribute a tool call to its MCP server.
//!
//! The convention `mcp__<server>__<leaf>` is the shared shape across
//! claude-code-acp / codex-acp / opencode-acp — every ACP vendor
//! namespaces MCP tools the same way. Vendor-native tools (Bash, Read,
//! …) carry no `mcp__` prefix and skip the lookup entirely.

use serde_json::Value;

/// Parse `mcp__<server>__<leaf>` → `(<server>, <leaf>)`. Returns `None`
/// for vendor-native tool names that don't carry the MCP prefix. The
/// leaf is what per-server auto-accept / auto-reject globs match
/// against — captains write `read_*` / `delete_*` inside the server
/// block; repeating the `mcp__<server>__` prefix would be redundant.
#[must_use]
pub fn parse_mcp_tool_name(tool: &str) -> Option<(&str, &str)> {
    let after_prefix = tool.strip_prefix("mcp__")?;
    let (server, leaf) = after_prefix.split_once("__")?;
    if server.is_empty() || leaf.is_empty() {
        return None;
    }
    Some((server, leaf))
}

/// Attribute a permission request to an MCP server/tool. Most ACP
/// adapters put the canonical `mcp__<server>__<leaf>` identity in the
/// tool name. Codex's MCP approval elicitation is the odd case: the
/// title is generic (`Approve MCP tool call`) and the server/tool live
/// inside rawInput's approval request metadata.
#[must_use]
pub fn parse_mcp_permission<'a>(tool: &'a str, raw_input: Option<&'a Value>) -> Option<(String, String)> {
    parse_mcp_tool_name(tool)
        .map(|(server, leaf)| (server.to_string(), leaf.to_string()))
        .or_else(|| parse_codex_mcp_approval(raw_input?))
}

fn parse_codex_mcp_approval(raw: &Value) -> Option<(String, String)> {
    let payload = codex_approval_payload(raw)?;
    let message = payload.get("message").and_then(Value::as_str)?;
    let server = message.strip_prefix("Allow the ")?.split_once(" MCP server")?.0.trim();
    let tool = message.split_once("run tool \"")?.1.split_once('"')?.0.trim();

    if server.is_empty() || tool.is_empty() {
        return None;
    }

    Some((server.to_string(), tool.to_string()))
}

fn codex_approval_payload(raw: &Value) -> Option<&Value> {
    if is_codex_mcp_approval(raw) {
        return Some(raw);
    }
    raw.get("request").filter(|request| is_codex_mcp_approval(request))
}

fn is_codex_mcp_approval(value: &Value) -> bool {
    value
        .get("_meta")
        .or_else(|| value.get("meta"))
        .and_then(|meta| meta.get("codex_approval_kind"))
        .and_then(Value::as_str)
        == Some("mcp_tool_call")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_prefix_and_returns_leaf() {
        assert_eq!(
            parse_mcp_tool_name("mcp__github__create_issue"),
            Some(("github", "create_issue"))
        );
        assert_eq!(parse_mcp_tool_name("mcp__a__b"), Some(("a", "b")));
    }

    #[test]
    fn rejects_non_mcp_or_empty_components() {
        assert_eq!(parse_mcp_tool_name("Bash"), None);
        assert_eq!(parse_mcp_tool_name("Read"), None);
        assert_eq!(parse_mcp_tool_name("mcp__"), None);
        assert_eq!(parse_mcp_tool_name("mcp____leaf"), None);
        assert_eq!(parse_mcp_tool_name("mcp__server__"), None);
    }

    #[test]
    fn parses_codex_mcp_approval_metadata() {
        let raw = serde_json::json!({
            "request": {
                "_meta": { "codex_approval_kind": "mcp_tool_call" },
                "message": "Allow the hyprpilot MCP server to run tool \"read_skill\"?"
            }
        });

        assert_eq!(
            parse_mcp_permission("Approve MCP tool call", Some(&raw)),
            Some(("hyprpilot".to_string(), "read_skill".to_string()))
        );
    }
}
