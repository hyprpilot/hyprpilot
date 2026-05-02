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

/// Expand `~` and `$VAR` / `${VAR}` references against the daemon's
/// own environment. Used for every captain-supplied path or value
/// that reaches the spawn surface (binary path, cwd, env values) so
/// a config like `env.PATH = "$HOME/bin:$PATH"` or `cwd = "~/projects/foo"`
/// works the way a shell would resolve it. Failure (undefined variable,
/// broken `~`) returns the input unchanged and logs a warn — a hard
/// error here would refuse to spawn the agent over a typo, worse
/// than letting the agent inherit a literal `$FOO` and fail visibly
/// downstream.
fn expand_value(raw: &str, ctx: &str) -> String {
    match shellexpand::full(raw) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "agent spawn: env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

/// Vendor-adapter trait. Implementors are unit structs — state lives
/// on `AgentConfig`.
pub trait AcpAgent: Send + Sync + 'static {
    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program_raw = entry.command.clone().unwrap_or_else(|| self.command().to_string());
        let program = expand_value(&program_raw, "agent.command");

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
        for (k, v) in entry.env.iter() {
            cmd.env(k, expand_value(v, "agent.env"));
        }

        // Env-style model injection — set the env var when user didn't
        // already define it. Runs after envs(entry.env) so the user's
        // entry overrides the vendor default.
        if let (Some(model), ModelInjection::Env(name)) = (entry.model.as_deref(), self.model_injection()) {
            if !entry.env.contains_key(name) {
                cmd.env(name, model);
            }
        }

        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(expand_value(&cwd.to_string_lossy(), "agent.cwd"));
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

    #[test]
    fn spawn_expands_env_values_against_process_env() {
        // SAFETY: tests in this module run in the same process; no
        // other test reads HYPRPILOT_TEST_ENV_EXPAND so this is safe.
        unsafe {
            std::env::set_var("HYPRPILOT_TEST_ENV_EXPAND", "expanded-value");
        }
        let mut entry = stub_entry("env-expand");

        entry
            .env
            .insert("FOO".into(), "prefix-${HYPRPILOT_TEST_ENV_EXPAND}-suffix".into());
        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        let envs: Vec<_> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| v.map(|vv| (k.to_owned(), vv.to_owned())))
            .collect();
        let foo = envs.iter().find(|(k, _)| k == "FOO").expect("FOO is set");
        assert_eq!(foo.1.to_str().unwrap(), "prefix-expanded-value-suffix");

        unsafe {
            std::env::remove_var("HYPRPILOT_TEST_ENV_EXPAND");
        }
    }

    #[test]
    fn spawn_expands_tilde_in_cwd() {
        let home = std::env::var_os("HOME").expect("HOME is always set in CI/dev");
        let mut entry = stub_entry("cwd-expand");

        entry.cwd = Some(std::path::PathBuf::from("~"));
        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        // tokio::process::Command exposes get_current_dir via the
        // wrapped std::process::Command.
        let cwd = cmd.as_std().get_current_dir().expect("cwd set");
        assert_eq!(cwd.as_os_str(), home.as_os_str());
    }

    #[test]
    fn spawn_leaves_undefined_var_literal_when_expansion_fails() {
        let mut entry = stub_entry("env-undef");

        entry
            .env
            .insert("FOO".into(), "${HYPRPILOT_NEVER_DEFINED_X1Y2Z3}".into());
        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        let envs: Vec<_> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| v.map(|vv| (k.to_owned(), vv.to_owned())))
            .collect();
        let foo = envs.iter().find(|(k, _)| k == "FOO").expect("FOO is set");
        // Undefined var → keep the literal so the agent inherits an
        // observable failure rather than silently dropping the value.
        assert_eq!(foo.1.to_str().unwrap(), "${HYPRPILOT_NEVER_DEFINED_X1Y2Z3}");
    }
}
