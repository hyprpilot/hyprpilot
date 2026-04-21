//! Codex ACP adapter.
//!
//! Launches via `bunx --bun @zed-industries/codex-acp`. The live
//! `render_update` + `tool_name_for_permission` bodies land with the
//! session follow-up; today only the launch command ships.

use super::AcpAgent;

pub struct AcpAgentCodex;

impl AcpAgent for AcpAgentCodex {
    fn command(&self) -> &'static str {
        "bunx"
    }

    fn args(&self) -> &'static [&'static str] {
        &["--bun", "@zed-industries/codex-acp"]
    }
}
