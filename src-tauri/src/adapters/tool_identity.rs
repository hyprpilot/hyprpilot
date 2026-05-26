use serde::{Deserialize, Serialize};

/// Stable identity for the tool behind a tool call / permission request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolIdentity {
    #[default]
    Native,
    Mcp {
        server: String,
        leaf: String,
    },
}

impl ToolIdentity {
    #[must_use]
    pub fn from_mcp_name(tool: &str) -> Option<Self> {
        crate::mcp::permission_match::parse_mcp_tool_name(tool).map(|(server, leaf)| Self::Mcp {
            server: server.to_string(),
            leaf: leaf.to_string(),
        })
    }

    #[must_use]
    pub fn is_mcp(&self) -> bool {
        matches!(self, Self::Mcp { .. })
    }
}
