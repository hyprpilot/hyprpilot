//! Per-vendor ACP adapters.

pub mod claude_code;
pub mod codex;
pub mod opencode;

use tokio::process::Command;

use agent_client_protocol::schema::ToolCallUpdate;

use crate::config::{AgentConfig, AgentProvider};
use crate::tools::formatter::registry::FormatterRegistry;
use crate::tools::ToolKind;

pub use self::claude_code::AcpAgentClaudeCode;
pub use self::codex::AcpAgentCodex;
pub use self::opencode::AcpAgentOpenCode;

/// Walk every vendor module and let it land its per-tool formatter
/// overrides on the supplied registry. Called once at registry
/// construction; idempotent (each vendor's `register_all` is keyed,
/// last write wins).
pub fn register_all_formatters(reg: &mut FormatterRegistry) {
    claude_code::formatters::register_all(reg);
    codex::formatters::register_all(reg);
    opencode::formatters::register_all(reg);
}

/// Per-vendor pre-spawn model injection knob. Three flavors:
/// - `None` — vendor doesn't accept a model override.
/// - `Env(name)` — set `name=<model>` env when `entry.env` doesn't
///   already define it.
/// - `Argv(flag)` — append `flag <model>` to argv when `entry.args`
///   doesn't already include `flag`.
/// - `Config(key)` — append `-c key="<model>"` when `entry.args`
///   doesn't already carry a Codex-style `-c` / `--config` override
///   for `key`.
///
/// "User value wins" enforcement lives in the trait-default `spawn()`
/// — vendors only declare *where* the injection lands.
#[derive(Debug, Clone, Copy)]
pub enum ModelInjection {
    None,
    Env(&'static str),
    #[allow(dead_code)]
    Argv(&'static str),
    Config(&'static str),
}

#[derive(Debug, Clone)]
pub enum ConfigOptionRoute {
    SetConfigOption { config_id: String },
    SetModel { model_id: String },
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
fn expand_value_with<F>(raw: &str, ctx: &str, lookup: &mut F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(lookup(name))
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "agent spawn: env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn expand_value(raw: &str, ctx: &str) -> String {
    expand_value_with(raw, ctx, &mut |name| std::env::var(name).ok())
}

fn has_config_override(args: &[String], key: &str) -> bool {
    args.iter()
        .filter_map(|arg| arg.strip_prefix("--config="))
        .chain(
            args.windows(2)
                .filter_map(|w| matches!(w[0].as_str(), "-c" | "--config").then_some(w[1].as_str())),
        )
        .any(|raw| {
            raw.split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
        })
}

fn config_override_arg(key: &str, value: &str) -> String {
    format!("{key}={}", serde_json::to_string(value).expect("str always serializes"))
}

fn inject_value(args: &mut Vec<String>, user_args: &[String], value: Option<&str>, injection: ModelInjection) {
    match (value, injection) {
        (Some(value), ModelInjection::Config(key)) if !has_config_override(user_args, key) => {
            args.push("-c".to_string());
            args.push(config_override_arg(key, value));
        }
        (Some(value), ModelInjection::Argv(flag)) if !user_args.iter().any(|a| a == flag) => {
            args.push(flag.to_string());
            args.push(value.to_string());
        }
        _ => {}
    }
}

fn inject_env_value(
    cmd: &mut Command,
    user_env: &std::collections::BTreeMap<String, String>,
    value: Option<&str>,
    injection: ModelInjection,
) {
    if let (Some(value), ModelInjection::Env(name)) = (value, injection) {
        if !user_env.contains_key(name) {
            cmd.env(name, value);
        }
    }
}

/// Vendor-adapter trait. Implementors are unit structs — state lives
/// on `AgentConfig`. `command` + `args` come from config (mandatory at
/// validate time); the trait carries only the per-vendor injection
/// knobs (`model_injection` for spawn-time model dispatch,
/// `inject_system_prompt` for spawn-time prompt placement).
pub trait AcpAgent: Send + Sync + 'static {
    fn spawn(&self, entry: &AgentConfig) -> Command {
        use std::process::Stdio;

        let program = expand_value(&entry.command, "agent.command");
        let mut args: Vec<String> = entry.args.clone();

        inject_value(&mut args, &entry.args, entry.model.as_deref(), self.model_injection());
        inject_value(&mut args, &entry.args, entry.effort.as_deref(), self.effort_injection());

        let mut cmd = Command::new(&program);
        cmd.args(&args);
        for (k, v) in entry.env.iter() {
            cmd.env(k, expand_value(v, "agent.env"));
        }

        inject_env_value(&mut cmd, &entry.env, entry.model.as_deref(), self.model_injection());
        inject_env_value(&mut cmd, &entry.env, entry.effort.as_deref(), self.effort_injection());

        if let Some(cwd) = entry.cwd.as_ref() {
            cmd.current_dir(expand_value(&cwd.to_string_lossy(), "agent.cwd"));
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    /// Where to splice `entry.model` into the spawn command. Default
    /// `None` means the vendor doesn't accept a model override.
    fn model_injection(&self) -> ModelInjection {
        ModelInjection::None
    }

    fn effort_injection(&self) -> ModelInjection {
        ModelInjection::None
    }

    fn display_config_option_id(&self, id: &str) -> String {
        id.to_string()
    }

    fn wire_config_option_id(&self, id: &str) -> String {
        id.to_string()
    }

    fn config_option_route(&self, id: &str, _value: &str, _current_model: Option<&str>) -> ConfigOptionRoute {
        ConfigOptionRoute::SetConfigOption {
            config_id: self.wire_config_option_id(id),
        }
    }

    fn augment_config_options(
        &self,
        _categories: &mut Vec<crate::adapters::SessionConfigOptionCategory>,
        _configured_effort: Option<&str>,
    ) {
    }

    fn permission_mcp_tool(&self, _update: &ToolCallUpdate) -> Option<ToolKind> {
        None
    }

    fn permission_mcp_tool_with_servers(
        &self,
        update: &ToolCallUpdate,
        _mcp_server_names: &[String],
    ) -> Option<ToolKind> {
        self.permission_mcp_tool(update)
    }

    fn mcp_tool(&self, _title: &str, _raw_input: Option<&serde_json::Value>) -> Option<ToolKind> {
        None
    }

    fn mcp_tool_with_servers(
        &self,
        title: &str,
        raw_input: Option<&serde_json::Value>,
        _mcp_server_names: &[String],
    ) -> Option<ToolKind> {
        self.mcp_tool(title, raw_input)
    }

    fn suppress_initial_tool_call(
        &self,
        _title: &str,
        _raw_input: Option<&serde_json::Value>,
        _state: crate::adapters::ToolCallState,
    ) -> bool {
        false
    }

    /// Default drops the prompt — vendors without a hook degrade silently
    /// rather than failing spawn.
    fn inject_system_prompt(&self, _cmd: &mut Command, _prompt: &str) -> SystemPromptInjection {
        SystemPromptInjection::Handled
    }
}

/// `acp` provider — no-op vendor. User-supplied ACP binaries
/// that don't need spawn-time model env / system-prompt injection
/// land here. For vendors that DO need injection, copy one of the
/// three named providers; future TOML overrides on the named
/// providers' injection knobs are an additive follow-up.
pub struct AcpAgentCustom;

impl AcpAgent for AcpAgentCustom {}

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
        AgentProvider::Acp => Box::new(AcpAgentCustom),
    }
}

#[must_use]
pub fn display_config_option_id(adapter_id: &str, id: &str) -> String {
    match adapter_id {
        "acp-claude-code" => AcpAgentClaudeCode.display_config_option_id(id),
        "acp-codex" => AcpAgentCodex.display_config_option_id(id),
        "acp-opencode" => AcpAgentOpenCode.display_config_option_id(id),
        _ => AcpAgentCustom.display_config_option_id(id),
    }
}

pub fn augment_config_options(
    adapter_id: &str,
    categories: &mut Vec<crate::adapters::SessionConfigOptionCategory>,
    configured_effort: Option<&str>,
) {
    match adapter_id {
        "acp-claude-code" => AcpAgentClaudeCode.augment_config_options(categories, configured_effort),
        "acp-codex" => AcpAgentCodex.augment_config_options(categories, configured_effort),
        "acp-opencode" => AcpAgentOpenCode.augment_config_options(categories, configured_effort),
        _ => AcpAgentCustom.augment_config_options(categories, configured_effort),
    }
}

#[must_use]
pub fn mcp_tool_with_servers(
    adapter_id: &str,
    title: &str,
    raw_input: Option<&serde_json::Value>,
    mcp_server_names: &[String],
) -> Option<ToolKind> {
    match adapter_id {
        "acp-claude-code" => AcpAgentClaudeCode.mcp_tool_with_servers(title, raw_input, mcp_server_names),
        "acp-codex" => AcpAgentCodex.mcp_tool_with_servers(title, raw_input, mcp_server_names),
        "acp-opencode" => AcpAgentOpenCode.mcp_tool_with_servers(title, raw_input, mcp_server_names),
        _ => AcpAgentCustom.mcp_tool_with_servers(title, raw_input, mcp_server_names),
    }
}

#[must_use]
pub fn suppress_initial_tool_call(
    adapter_id: &str,
    title: &str,
    raw_input: Option<&serde_json::Value>,
    state: crate::adapters::ToolCallState,
) -> bool {
    match adapter_id {
        "acp-claude-code" => AcpAgentClaudeCode.suppress_initial_tool_call(title, raw_input, state),
        "acp-codex" => AcpAgentCodex.suppress_initial_tool_call(title, raw_input, state),
        "acp-opencode" => AcpAgentOpenCode.suppress_initial_tool_call(title, raw_input, state),
        _ => AcpAgentCustom.suppress_initial_tool_call(title, raw_input, state),
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
            effort: None,
            command: "bunx".into(),
            args: vec!["--bun".into(), "@agentclientprotocol/claude-agent-acp".into()],
            spawn: None,
            cwd: None,
            env: Default::default(),
        }
    }

    #[test]
    fn spawn_command_respects_user_command() {
        let mut entry = stub_entry("override-test");
        entry.command = "my-agent".into();
        entry.args = vec!["--yolo".into()];

        let cmd = match_provider_agent(AgentProvider::AcpClaudeCode).spawn(&entry);
        assert_eq!(cmd.as_std().get_program(), "my-agent");
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, vec!["--yolo"]);
    }

    #[test]
    fn custom_provider_resolves_to_no_op_agent() {
        let mut entry = stub_entry("custom");
        entry.provider = AgentProvider::Acp;
        entry.command = "my-acp-binary".into();
        entry.args = vec!["--serve".into()];

        let cmd = match_provider_agent(AgentProvider::Acp).spawn(&entry);
        assert_eq!(cmd.as_std().get_program(), "my-acp-binary");
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, vec!["--serve"]);
    }

    #[test]
    fn argv_model_injection_appends_missing_flag() {
        struct ArgvAgent;

        impl AcpAgent for ArgvAgent {
            fn model_injection(&self) -> ModelInjection {
                ModelInjection::Argv("--model")
            }
        }

        let mut entry = stub_entry("argv");
        entry.model = Some("test-model".into());
        let cmd = ArgvAgent.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();

        assert!(args.windows(2).any(|w| w == ["--model", "test-model"]));
    }

    #[test]
    fn argv_model_injection_preserves_user_flag() {
        struct ArgvAgent;

        impl AcpAgent for ArgvAgent {
            fn model_injection(&self) -> ModelInjection {
                ModelInjection::Argv("--model")
            }
        }

        let mut entry = stub_entry("argv");
        entry.model = Some("config-model".into());
        entry.args.push("--model".into());
        entry.args.push("user-model".into());
        let cmd = ArgvAgent.spawn(&entry);
        let args: Vec<_> = cmd.as_std().get_args().map(|a| a.to_str().unwrap()).collect();

        assert_eq!(args.iter().filter(|arg| **arg == "--model").count(), 1);
        assert!(args.windows(2).any(|w| w == ["--model", "user-model"]));
    }

    #[test]
    fn expand_value_uses_supplied_environment_lookup() {
        let expanded =
            expand_value_with(
                "prefix-${HYPRPILOT_TEST_ENV_EXPAND}-suffix",
                "agent.env",
                &mut |name| match name {
                    "HYPRPILOT_TEST_ENV_EXPAND" => Some("expanded-value".into()),
                    _ => None,
                },
            );

        assert_eq!(expanded, "prefix-expanded-value-suffix");
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
