//! Tool-name attribution shared by the permission controller and
//! anywhere else that needs to attribute a tool call to its MCP server.
//!
//! The convention `mcp__<server>__<leaf>` is the shared shape across
//! claude-code-acp / codex-acp / opencode-acp — every ACP vendor
//! namespaces MCP tools the same way. Vendor-native tools (Bash, Read,
//! …) carry no `mcp__` prefix and skip the lookup entirely.

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
}
