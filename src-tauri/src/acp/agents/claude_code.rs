//! Claude Code ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/claude-code-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use super::AcpAgent;

/// Unit struct — carries no runtime state. Everything returned by its
/// methods is static per-vendor knowledge; `AgentConfig` on the spawn
/// flow provides the rest.
pub struct AcpAgentClaudeCode;

impl AcpAgent for AcpAgentClaudeCode {
    fn default_command(&self) -> &'static str {
        "bunx"
    }

    fn default_args(&self) -> &'static [&'static str] {
        &["--bun", "@zed-industries/claude-code-acp"]
    }
}
