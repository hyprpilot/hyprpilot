//! opencode ACP adapter.
//!
//! Launches via `opencode acp` — a native binary, no `bunx` wrapper.
//! The live `render_update` + `tool_name_for_permission` bodies land
//! with the session follow-up; today only the launch command ships.

use super::AcpAgent;

pub struct AcpAgentOpenCode;

impl AcpAgent for AcpAgentOpenCode {
    fn command(&self) -> &'static str {
        "opencode"
    }

    fn args(&self) -> &'static [&'static str] {
        &["acp"]
    }
}
