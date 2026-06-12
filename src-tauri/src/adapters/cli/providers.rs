use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};

use agent_client_protocol::schema::McpServer;
use anyhow::{bail, Context, Result};

use crate::adapters::profile::ResolvedInstance;
use crate::adapters::Bootstrap;
use crate::config::{AgentProvider, AgentSpawnConfig};
use crate::mcp::{project_to_acp, MCPDefinition};

const INLINE_CONFIG_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub(super) struct DirectCommand {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
}

pub(super) fn build_command(
    resolved: &ResolvedInstance,
    bootstrap: &Bootstrap,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    match resolved.agent.provider {
        AgentProvider::AcpClaudeCode => build_claude(resolved, bootstrap, system_prompt, mcp_defs, provider_args),
        AgentProvider::AcpCodex => build_codex(resolved, bootstrap, system_prompt, mcp_defs, provider_args),
        AgentProvider::AcpOpenCode => build_opencode(resolved, bootstrap, system_prompt, mcp_defs, provider_args),
        AgentProvider::Acp => build_generic(resolved, provider_args),
    }
}

pub(super) fn exec(command: DirectCommand) -> Result<ExitCode> {
    let mut cmd = std::process::Command::new(&command.program);
    cmd.args(&command.args)
        .envs(&command.env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(cwd) = command.cwd.as_ref() {
        cmd.current_dir(cwd);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = cmd.exec();
        Err(err).with_context(|| format!("exec {}", command.program))
    }

    #[cfg(not(unix))]
    {
        let status = cmd.status().with_context(|| format!("spawning {}", command.program))?;

        Ok(status
            .code()
            .map_or_else(|| ExitCode::from(1), |code| ExitCode::from(code as u8)))
    }
}

fn build_generic(resolved: &ResolvedInstance, provider_args: Vec<String>) -> Result<DirectCommand> {
    let mut command = base_command(resolved)?;
    command.args.extend(provider_args);

    Ok(command)
}

fn build_claude(
    resolved: &ResolvedInstance,
    bootstrap: &Bootstrap,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    let mut command = base_command(resolved)?;
    let detect_args = combined_args(&command.args, &provider_args);

    if let Some(model) = resolved.model.as_deref() {
        if !has_flag(&detect_args, "--model", None) {
            command.args.push("--model".into());
            command.args.push(model.into());
        }
    }
    if let Some(effort) = resolved.effort.as_deref() {
        if !has_flag(&detect_args, "--effort", None) {
            command.args.push("--effort".into());
            command.args.push(effort.into());
        }
    }
    if let Some(mode) = resolved.mode.as_deref() {
        if !has_flag(&detect_args, "--permission-mode", None) {
            command.args.push("--permission-mode".into());
            command.args.push(mode.into());
        }
    }
    if let Some(prompt) = system_prompt {
        if !prompt.is_empty() && !has_flag(&detect_args, "--append-system-prompt", None) {
            ensure_inline_size("claude --append-system-prompt", prompt)?;
            command.args.push("--append-system-prompt".into());
            command.args.push(prompt.into());
        }
    }
    if !mcp_defs.is_empty() && !has_flag(&detect_args, "--mcp-config", None) {
        let config = claude_mcp_config(mcp_defs)?;
        ensure_inline_size("claude --mcp-config", &config)?;
        command.args.push("--mcp-config".into());
        command.args.push(config);
    }
    if let Bootstrap::Resume(session_id) = bootstrap {
        if !has_flag(&detect_args, "--resume", None) {
            command.args.push("--resume".into());
            command.args.push(session_id.clone());
        }
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn build_codex(
    resolved: &ResolvedInstance,
    bootstrap: &Bootstrap,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    let mut command = base_command(resolved)?;
    let detect_args = combined_args(&command.args, &provider_args);
    if matches!(bootstrap, Bootstrap::Resume(_)) && !detect_args.iter().any(|arg| arg == "resume") {
        command.args.push("resume".into());
    }

    if let Some(model) = resolved.model.as_deref() {
        if !has_flag(&detect_args, "--model", Some("-m")) {
            command.args.push("--model".into());
            command.args.push(model.into());
        }
    }
    if let Some(effort) = resolved.effort.as_deref() {
        push_codex_config_if_absent(
            &mut command.args,
            &detect_args,
            "model_reasoning_effort",
            toml_string(effort),
        );
    }
    if let Some(mode) = resolved.mode.as_deref() {
        if !has_flag(&detect_args, "--ask-for-approval", Some("-a")) {
            command.args.push("--ask-for-approval".into());
            command.args.push(mode.into());
        }
    }
    if let Some(prompt) = system_prompt {
        if !prompt.is_empty() {
            ensure_inline_size("codex -c instructions", prompt)?;
            push_codex_config_if_absent(&mut command.args, &detect_args, "instructions", toml_string(prompt));
        }
    }
    for (key, value) in codex_mcp_config_entries(mcp_defs) {
        push_codex_config_if_absent(&mut command.args, &detect_args, &key, value);
    }
    if let Bootstrap::Resume(session_id) = bootstrap {
        command.args.push(session_id.clone());
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn build_opencode(
    resolved: &ResolvedInstance,
    bootstrap: &Bootstrap,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    let mut command = base_command(resolved)?;
    let detect_args = combined_args(&command.args, &provider_args);
    let agent_name = flag_value(&detect_args, "--agent", None)
        .or_else(|| resolved.mode.clone())
        .unwrap_or_else(|| "hyprpilot".into());

    if let Some(model) = resolved.model.as_deref() {
        if !has_flag(&detect_args, "--model", Some("-m")) {
            command.args.push("--model".into());
            command.args.push(model.into());
        }
    }
    if !has_flag(&detect_args, "--agent", None) {
        command.args.push("--agent".into());
        command.args.push(agent_name.clone());
    }
    if let Bootstrap::Resume(session_id) = bootstrap {
        if !has_flag(&detect_args, "--session", Some("-s")) {
            command.args.push("--session".into());
            command.args.push(session_id.clone());
        }
    }

    if !command.env.contains_key("OPENCODE_CONFIG_CONTENT") {
        if let Some(config) = opencode_config_content(
            &agent_name,
            resolved.model.as_deref(),
            resolved.effort.as_deref(),
            system_prompt,
            mcp_defs,
        )? {
            ensure_inline_size("OPENCODE_CONFIG_CONTENT", &config)?;
            command.env.insert("OPENCODE_CONFIG_CONTENT".into(), config);
        }
    } else if system_prompt.is_some() || resolved.effort.is_some() || !mcp_defs.is_empty() {
        tracing::warn!(
            "cli spawn: OPENCODE_CONFIG_CONTENT already set by agent env; skipping generated opencode prompt/MCP/variant config"
        );
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn base_command(resolved: &ResolvedInstance) -> Result<DirectCommand> {
    let spawn = resolved
        .agent
        .spawn
        .as_ref()
        .with_context(|| format!("agent '{}' does not define [agents.spawn]", resolved.agent.id))?;
    let program = expand_value(&spawn.command, "agents.spawn.command");
    let args = expand_args(spawn);
    let env = resolved
        .agent
        .env
        .iter()
        .map(|(key, value)| (key.clone(), expand_value(value, "agents.env")))
        .collect();
    let cwd = resolved
        .agent
        .cwd
        .as_ref()
        .map(|path| PathBuf::from(expand_value(&path.to_string_lossy(), "agents.cwd")));

    Ok(DirectCommand {
        program,
        args,
        env,
        cwd,
    })
}

fn expand_args(spawn: &AgentSpawnConfig) -> Vec<String> {
    spawn
        .args
        .iter()
        .map(|arg| expand_value(arg, "agents.spawn.args"))
        .collect()
}

fn expand_value(raw: &str, ctx: &str) -> String {
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(std::env::var(name).ok())
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "cli spawn: env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn combined_args(base: &[String], provider: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(base.len() + provider.len());
    out.extend_from_slice(base);
    out.extend_from_slice(provider);
    out
}

fn has_flag(args: &[String], long: &str, short: Option<&str>) -> bool {
    args.iter().any(|arg| {
        arg == long
            || short.is_some_and(|short| arg == short)
            || arg.strip_prefix(long).is_some_and(|rest| rest.starts_with('='))
    })
}

fn flag_value(args: &[String], long: &str, short: Option<&str>) -> Option<String> {
    for (idx, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(long).and_then(|rest| rest.strip_prefix('=')) {
            return Some(value.to_string());
        }
        if arg == long || short.is_some_and(|short| arg == short) {
            return args.get(idx + 1).cloned();
        }
    }

    None
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

fn push_codex_config_if_absent(args: &mut Vec<String>, detect_args: &[String], key: &str, value: String) {
    if has_config_override(detect_args, key) {
        return;
    }

    args.push("-c".into());
    args.push(format!("{key}={value}"));
}

fn claude_mcp_config(defs: &[MCPDefinition]) -> Result<String> {
    let mut servers = serde_json::Map::new();
    for def in defs {
        servers.insert(def.name.clone(), def.raw.clone());
    }
    serde_json::to_string(&serde_json::json!({ "mcpServers": servers })).context("serialize claude MCP config")
}

fn codex_mcp_config_entries(defs: &[MCPDefinition]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for def in defs {
        let Some(server) = project_to_acp(def) else {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for codex");
            continue;
        };
        match server {
            McpServer::Stdio(stdio) => {
                let prefix = toml_key_path(&["mcp_servers", &stdio.name]);
                entries.push((
                    format!("{prefix}.command"),
                    toml_string(&stdio.command.to_string_lossy()),
                ));
                if !stdio.args.is_empty() {
                    entries.push((format!("{prefix}.args"), toml_array(&stdio.args)));
                }
                for env in stdio.env {
                    entries.push((
                        toml_key_path(&["mcp_servers", &stdio.name, "env", &env.name]),
                        toml_string(&env.value),
                    ));
                }
            }
            McpServer::Http(http) => {
                let prefix = toml_key_path(&["mcp_servers", &http.name]);
                entries.push((format!("{prefix}.url"), toml_string(&http.url)));
                for header in http.headers {
                    entries.push((
                        toml_key_path(&["mcp_servers", &http.name, "headers", &header.name]),
                        toml_string(&header.value),
                    ));
                }
            }
            McpServer::Sse(sse) => {
                let prefix = toml_key_path(&["mcp_servers", &sse.name]);
                entries.push((format!("{prefix}.url"), toml_string(&sse.url)));
                for header in sse.headers {
                    entries.push((
                        toml_key_path(&["mcp_servers", &sse.name, "headers", &header.name]),
                        toml_string(&header.value),
                    ));
                }
            }
            _ => {}
        }
    }

    entries
}

fn opencode_config_content(
    agent_name: &str,
    model: Option<&str>,
    effort: Option<&str>,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
) -> Result<Option<String>> {
    let mut root = serde_json::Map::new();
    let mut agent = serde_json::Map::new();
    if let Some(prompt) = system_prompt {
        if !prompt.is_empty() {
            agent.insert("prompt".into(), serde_json::Value::String(prompt.into()));
        }
    }
    if let Some(model) = model {
        agent.insert("model".into(), serde_json::Value::String(model.into()));
    }
    if let Some(effort) = effort {
        agent.insert("variant".into(), serde_json::Value::String(effort.into()));
    }
    if !agent.is_empty() {
        root.insert(
            "agent".into(),
            serde_json::json!({
                agent_name: agent,
            }),
        );
        root.insert("default_agent".into(), serde_json::Value::String(agent_name.into()));
    }

    let mcp = opencode_mcp_config(mcp_defs);
    if !mcp.is_empty() {
        root.insert("mcp".into(), serde_json::Value::Object(mcp));
    }
    if root.is_empty() {
        return Ok(None);
    }

    serde_json::to_string(&serde_json::Value::Object(root))
        .map(Some)
        .context("serialize opencode config")
}

fn opencode_mcp_config(defs: &[MCPDefinition]) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for def in defs {
        let Some(server) = project_to_acp(def) else {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for opencode");
            continue;
        };
        match server {
            McpServer::Stdio(stdio) => {
                let mut command = Vec::with_capacity(1 + stdio.args.len());
                command.push(stdio.command.to_string_lossy().to_string());
                command.extend(stdio.args);
                let environment: serde_json::Map<String, serde_json::Value> = stdio
                    .env
                    .into_iter()
                    .map(|env| (env.name, serde_json::Value::String(env.value)))
                    .collect();
                let mut entry = serde_json::json!({
                    "type": "local",
                    "command": command,
                    "enabled": true,
                });
                if !environment.is_empty() {
                    entry["environment"] = serde_json::Value::Object(environment);
                }
                out.insert(stdio.name, entry);
            }
            McpServer::Http(http) => {
                out.insert(http.name.clone(), opencode_remote_mcp(http.url, http.headers));
            }
            McpServer::Sse(sse) => {
                out.insert(sse.name.clone(), opencode_remote_mcp(sse.url, sse.headers));
            }
            _ => {}
        }
    }

    out
}

fn opencode_remote_mcp(url: String, headers: Vec<agent_client_protocol::schema::HttpHeader>) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "type": "remote",
        "url": url,
        "enabled": true,
    });
    if !headers.is_empty() {
        let headers: serde_json::Map<String, serde_json::Value> = headers
            .into_iter()
            .map(|header| (header.name, serde_json::Value::String(header.value)))
            .collect();
        entry["headers"] = serde_json::Value::Object(headers);
    }

    entry
}

fn toml_key_path(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| {
            if is_bare_toml_key(part) {
                (*part).to_string()
            } else {
                serde_json::to_string(part).expect("str always serializes")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn is_bare_toml_key(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("str always serializes")
}

fn toml_array(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| toml_string(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

fn ensure_inline_size(label: &str, value: &str) -> Result<()> {
    if value.len() > INLINE_CONFIG_LIMIT {
        bail!(
            "{label} is too large for inline direct CLI injection ({} bytes > {} bytes); refusing to write unmanaged temp config",
            value.len(),
            INLINE_CONFIG_LIMIT
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::config::{AgentConfig, AgentSpawnConfig};
    use crate::mcp::{HyprpilotExtension, MCPDefinition};

    fn resolved(provider: AgentProvider) -> ResolvedInstance {
        ResolvedInstance {
            agent: AgentConfig {
                id: "agent".into(),
                provider,
                model: None,
                effort: None,
                command: "/bin/false".into(),
                args: Vec::new(),
                spawn: Some(AgentSpawnConfig {
                    command: "provider".into(),
                    args: Vec::new(),
                }),
                cwd: None,
                env: BTreeMap::new(),
            },
            profile_id: Some("profile".into()),
            model: Some("model-a".into()),
            effort: Some("high".into()),
            system_prompt: Vec::new(),
            mode: Some("plan".into()),
        }
    }

    fn mcp_def() -> MCPDefinition {
        MCPDefinition {
            name: "hyprpilot-nvim".into(),
            raw: json!({
                "command": "uvx",
                "args": ["hyprpilot-nvim-mcp"],
                "env": { "NVIM": "/tmp/nvim.sock" },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    #[test]
    fn claude_provider_args_suppress_generated_model() {
        let command = build_command(
            &resolved(AgentProvider::AcpClaudeCode),
            &Bootstrap::Fresh,
            None,
            &[],
            vec!["--model".into(), "user-model".into()],
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--model").count(), 1);
    }

    #[test]
    fn codex_projects_mcp_into_config_overrides() {
        let command = build_command(
            &resolved(AgentProvider::AcpCodex),
            &Bootstrap::Fresh,
            Some("be terse"),
            &[mcp_def()],
            Vec::new(),
        )
        .unwrap();
        let joined = command.args.join("\n");

        assert!(joined.contains("instructions=\"be terse\""));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.command=\"uvx\""));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.args=[\"hyprpilot-nvim-mcp\"]"));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.env.NVIM=\"/tmp/nvim.sock\""));
    }

    #[test]
    fn opencode_puts_prompt_mcp_and_variant_in_inline_config() {
        let command = build_command(
            &resolved(AgentProvider::AcpOpenCode),
            &Bootstrap::Resume("ses_123".into()),
            Some("be terse"),
            &[mcp_def()],
            Vec::new(),
        )
        .unwrap();
        let config: serde_json::Value =
            serde_json::from_str(command.env.get("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();

        assert_eq!(config["agent"]["plan"]["prompt"], "be terse");
        assert_eq!(config["agent"]["plan"]["variant"], "high");
        assert_eq!(config["mcp"]["hyprpilot-nvim"]["command"][0], "uvx");
        assert!(command.args.windows(2).any(|w| w == ["--session", "ses_123"]));
    }
}
