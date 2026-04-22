//! Per-vendor ACP adapters.
//!
//! Each concrete struct encodes the quirks of a specific
//! ACP-speaking agent backend: launch command defaults, the
//! `PermissionOption` kinds the agent actually ships, and the
//! tool-content shape its `SessionUpdate`s carry.
//!
//! The `AcpAgent` trait is the protocol-level surface today; future
//! refactors can layer a more general `Agent` trait above it once a
//! non-ACP backend actually ships (HTTP, local echo, …). Keeping the
//! split minimal for now avoids forward-speculating at traits that
//! haven't earned their keep.

mod claude_code;
mod codex;
mod opencode;

use tokio::process::Command;

use crate::config::{AgentConfig, AgentProvider};

pub use self::claude_code::AcpAgentClaudeCode;
pub use self::codex::AcpAgentCodex;
pub use self::opencode::AcpAgentOpenCode;

/// Vendor-adapter trait. Concrete implementors are unit structs that
/// carry no runtime state — everything returned by the trait methods
/// is a function of the `AgentConfig` + static vendor-specific
/// knowledge.
///
/// Methods listed today:
///
/// - `spawn_command` — build the `tokio::process::Command` that
///   launches the vendor's ACP server binary. Inherits args / env /
///   cwd from `AgentConfig`, with the vendor's defaults filling in
///   when fields are omitted.
///
/// Future methods (land with the live-session plumbing):
///
/// - `render_update(&self, SessionUpdate) -> Option<TranscriptEvent>`
///   — normalise per-vendor tool-content quirks into the shared
///   discriminated-union shape the webview consumes.
/// - `tool_name_for_permission(&self, RequestPermissionRequest) ->
///   Option<String>` — recover the logical tool name from the
///   envelope (claude's `_meta.claudeCode.toolName`, codex's
///   `raw_input` shape, opencode's direct tool name).
/// - `client_capabilities(&self) -> ClientCapabilities` — advertise
///   capabilities the vendor requires (terminal, extended permission
///   modes, …).
pub trait AcpAgent: Send + Sync + 'static {
    /// Build the `tokio::process::Command` that launches this
    /// vendor's ACP server. The default implementation feeds the
    /// vendor's `default_command` + `default_args` when the user
    /// config omits them, overlays `env`, and sets
    /// `stdin=piped`/`stdout=piped`/`kill_on_drop`.
    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program = entry.command.clone().unwrap_or_else(|| self.command().to_string());

        let args = if entry.args.is_empty() {
            self.args().iter().map(|s| (*s).to_string()).collect::<Vec<_>>()
        } else {
            entry.args.clone()
        };

        let mut cmd = Command::new(&program);
        cmd.args(&args);
        cmd.envs(entry.env.iter());
        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    /// Vendor's default executable when `AgentConfig::command` is
    /// unset. Typically `bunx` (to run a Node-side ACP shim) or a
    /// native binary name.
    fn command(&self) -> &'static str;

    /// Vendor's default arguments when `AgentConfig::args` is empty.
    fn args(&self) -> &'static [&'static str];
}

/// Resolve the concrete vendor adapter for a given provider enum.
/// One match arm per variant; keeps the mapping centralised so
/// spawn flow + commands surface don't each grow their own
/// translation.
#[must_use]
pub fn match_provider_agent(provider: AgentProvider) -> Box<dyn AcpAgent> {
    match provider {
        AgentProvider::AcpClaudeCode => Box::new(AcpAgentClaudeCode),
        AgentProvider::AcpCodex => Box::new(AcpAgentCodex),
        AgentProvider::AcpOpenCode => Box::new(AcpAgentOpenCode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_entry(id: &str) -> AgentConfig {
        AgentConfig {
            id: id.into(),
            provider: AgentProvider::AcpClaudeCode,
            model: None,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: Default::default(),
        }
    }

    #[test]
    fn match_provider_agent_picks_concrete_adapter_per_provider() {
        // Each variant builds a spawnable command with the vendor's
        // defaults. Asserting on the program path is enough to
        // distinguish the three — the spawn flow wraps it further.
        let entry = stub_entry("anon");

        let claude_cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        assert_eq!(claude_cmd.as_std().get_program(), "bunx");

        let codex_cmd = match_provider_agent(AgentProvider::AcpCodex).spawn(&entry);
        assert_eq!(codex_cmd.as_std().get_program(), "bunx");

        let opencode_cmd = match_provider_agent(AgentProvider::AcpOpenCode).spawn(&entry);
        assert_eq!(opencode_cmd.as_std().get_program(), "opencode");
    }

    #[test]
    fn spawn_command_respects_user_command_override() {
        let mut entry = stub_entry("override-test");
        entry.command = Some("my-agent".into());
        entry.args = vec!["--yolo".into()];

        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        assert_eq!(cmd.as_std().get_program(), "my-agent");
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, vec!["--yolo"]);
    }

    #[test]
    fn spawn_command_uses_default_args_when_user_args_empty() {
        let entry = stub_entry("default-args");
        // claude-code-acp defaults to `--bun @zed-industries/claude-code-acp`.
        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, vec!["--bun", "@zed-industries/claude-code-acp"]);
    }
}
