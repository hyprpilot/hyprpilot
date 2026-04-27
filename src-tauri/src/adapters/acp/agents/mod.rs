//! Per-vendor ACP adapters.

mod claude_code;
mod codex;
mod opencode;

use tokio::process::Command;

use crate::adapters::Capabilities;
use crate::config::{AgentConfig, AgentProvider};

pub use self::claude_code::AcpAgentClaudeCode;
pub use self::codex::AcpAgentCodex;
pub use self::opencode::AcpAgentOpenCode;

/// Per-vendor pre-spawn model injection knob. Three flavors:
/// - `None` — vendor doesn't accept a model override.
/// - `Env(name)` — set `name=<model>` env when `entry.env` doesn't
///   already define it.
/// - `Argv(flag)` — append `flag <model>` to argv when `entry.args`
///   doesn't already include `flag`.
///
/// "User value wins" enforcement lives in the trait-default `spawn()`
/// — vendors only declare *where* the injection lands.
#[derive(Debug, Clone, Copy)]
pub enum ModelInjection {
    None,
    Env(&'static str),
    Argv(&'static str),
}

/// Vendor-adapter trait. Implementors are unit structs — state lives
/// on `AgentConfig`.
pub trait AcpAgent: Send + Sync + 'static {
    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program = entry.command.clone().unwrap_or_else(|| self.command().to_string());

        let mut args: Vec<String> = if entry.args.is_empty() {
            self.args().iter().map(|s| (*s).to_string()).collect()
        } else {
            entry.args.clone()
        };

        // Argv-style model injection — append flag + value when user
        // didn't already pass the flag explicitly. Done before
        // Command::new so the arg ordering reflects user intent.
        if let (Some(model), ModelInjection::Argv(flag)) = (entry.model.as_deref(), self.model_injection()) {
            if !entry.args.iter().any(|a| a == flag) {
                args.push(flag.to_string());
                args.push(model.to_string());
            }
        }

        let mut cmd = Command::new(&program);
        cmd.args(&args);
        cmd.envs(entry.env.iter());

        // Env-style model injection — set the env var when user didn't
        // already define it. Runs after envs(entry.env) so the user's
        // entry overrides the vendor default.
        if let (Some(model), ModelInjection::Env(name)) = (entry.model.as_deref(), self.model_injection()) {
            if !entry.env.contains_key(name) {
                cmd.env(name, model);
            }
        }

        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    fn command(&self) -> &'static str;

    fn args(&self) -> &'static [&'static str];

    /// Per-vendor static capability set. Drives UI gating + the
    /// `Adapter::capabilities_for_agent` lookup. Defaults to no caps —
    /// vendors override to declare their truthful surface.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Where to splice `entry.model` into the spawn command. Default
    /// `None` means the vendor doesn't accept a model override.
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::None
    }

    /// Default drops the prompt — vendors without a hook degrade silently
    /// rather than failing spawn.
    fn inject_system_prompt(&self, _cmd: &mut Command, _prompt: &str) -> SystemPromptInjection {
        SystemPromptInjection::Handled
    }
}

/// Outcome of pre-spawn system-prompt injection.
#[derive(Debug, Clone, Default)]
pub enum SystemPromptInjection {
    /// Vendor consumed the prompt pre-spawn, or has no hook.
    #[default]
    Handled,
    /// Runtime prepends this text onto the first `session/prompt`.
    FirstMessage(String),
}

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
        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, vec!["--bun", "@zed-industries/claude-code-acp"]);
    }
}
