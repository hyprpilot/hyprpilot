use serde::{Deserialize, Serialize};

/// Closed tool classification plus MCP attribution when the adapter can
/// prove the call came from an MCP server. Serialized under the existing
/// `toolKind` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Terminal,
    Acp,
    Mcp {
        server: String,
        tool: String,
    },
    #[default]
    Other,
}

impl ToolKind {
    #[must_use]
    pub fn from_wire(kind: Option<&str>, mcp: Option<Self>) -> Self {
        if matches!(mcp, Some(Self::Mcp { .. })) {
            return mcp.unwrap_or_default();
        }

        match kind.unwrap_or("other") {
            "read" => Self::Read,
            "edit" | "write" => Self::Edit,
            "delete" => Self::Delete,
            "move" => Self::Move,
            "search" | "glob" => Self::Search,
            "execute" | "bash" => Self::Execute,
            "think" => Self::Think,
            "fetch" => Self::Fetch,
            "terminal" => Self::Terminal,
            "acp" => Self::Acp,
            "mcp" => Self::Other,
            _ => Self::Other,
        }
    }

    #[must_use]
    pub fn from_mcp_name(tool: &str) -> Option<Self> {
        crate::mcp::permission_match::parse_mcp_tool_name(tool).map(|(server, leaf)| Self::Mcp {
            server: server.to_string(),
            tool: leaf.to_string(),
        })
    }

    #[must_use]
    pub fn is_mcp(&self) -> bool {
        matches!(self, Self::Mcp { .. })
    }

    #[must_use]
    pub fn wire_key(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Search => "search",
            Self::Execute => "execute",
            Self::Think => "think",
            Self::Fetch => "fetch",
            Self::Terminal => "terminal",
            Self::Acp => "acp",
            Self::Mcp { .. } => "mcp",
            Self::Other => "other",
        }
    }
}
