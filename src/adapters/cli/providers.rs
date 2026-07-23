use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};

use anyhow::{bail, Context, Result};

use crate::adapters::profile::ResolvedInstance;
use crate::config::AgentProvider;
use crate::mcp::{expanded_raw, project_transport, MCPDefinition, McpTransport};

const INLINE_CONFIG_LIMIT: usize = 256 * 1024;
const CODEX_APPROVAL_POLICIES: &[&str] = &["untrusted", "on-request", "never", "on-failure"];
const CODEX_SANDBOX_MODES: &[&str] = &["read-only", "workspace-write", "danger-full-access"];

#[derive(Debug)]
pub(super) struct DirectCommand {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
}

pub(super) fn build_command(
    resolved: &ResolvedInstance,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    match resolved.agent.provider {
        AgentProvider::ClaudeCode => build_claude(resolved, system_prompt, mcp_defs, provider_args),
        AgentProvider::Codex => build_codex(resolved, system_prompt, mcp_defs, provider_args),
        AgentProvider::OpenCode => build_opencode(resolved, system_prompt, mcp_defs, provider_args),
        AgentProvider::Custom => build_generic(resolved, provider_args),
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
    let permission_tools = claude_mcp_permission_tools(mcp_defs);
    if !permission_tools.allow.is_empty() && !has_claude_allowed_tools_flag(&detect_args) {
        command.args.push("--allowedTools".into());
        command.args.push(permission_tools.allow.join(","));
    }
    if !permission_tools.deny.is_empty() && !has_claude_disallowed_tools_flag(&detect_args) {
        command.args.push("--disallowedTools".into());
        command.args.push(permission_tools.deny.join(","));
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn build_codex(
    resolved: &ResolvedInstance,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
) -> Result<DirectCommand> {
    let mut command = base_command(resolved)?;
    let detect_args = combined_args(&command.args, &provider_args);

    if let Some(cwd) = command.cwd.as_ref() {
        if !has_flag(&detect_args, "--cd", Some("-C")) {
            command.args.push("--cd".into());
            command.args.push(cwd.display().to_string());
        }
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
        push_codex_mode_if_absent(&mut command.args, &detect_args, mode)?;
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
    for (key, value) in codex_mcp_tool_policy_entries(mcp_defs) {
        push_codex_config_if_absent(&mut command.args, &detect_args, &key, value);
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn build_opencode(
    resolved: &ResolvedInstance,
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
    if !command.env.contains_key("OPENCODE_PERMISSION") {
        if let Some(permissions) = opencode_permission_content(mcp_defs)? {
            ensure_inline_size("OPENCODE_PERMISSION", &permissions)?;
            command.env.insert("OPENCODE_PERMISSION".into(), permissions);
        }
    } else if mcp_defs.iter().any(|def| def.hyprpilot.has_tool_policy()) {
        tracing::warn!(
            "cli spawn: OPENCODE_PERMISSION already set by agent env; skipping generated opencode MCP tool policy config"
        );
    }

    command.args.extend(provider_args);

    Ok(command)
}

fn base_command(resolved: &ResolvedInstance) -> Result<DirectCommand> {
    let program = expand_value(&resolved.agent.command, "agents.command");
    let args = resolved
        .agent
        .args
        .iter()
        .map(|arg| expand_value(arg, "agents.args"))
        .collect();
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

fn expand_value(raw: &str, ctx: &str) -> String {
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(lookup_process_env(name))
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "cli spawn: env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn lookup_process_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| name.strip_prefix("env:").and_then(|name| std::env::var(name).ok()))
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

fn has_claude_allowed_tools_flag(args: &[String]) -> bool {
    has_flag(args, "--allowedTools", None) || has_flag(args, "--allowed-tools", None)
}

fn has_claude_disallowed_tools_flag(args: &[String]) -> bool {
    has_flag(args, "--disallowedTools", None) || has_flag(args, "--disallowed-tools", None)
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

fn push_codex_mode_if_absent(args: &mut Vec<String>, detect_args: &[String], mode: &str) -> Result<()> {
    let mode = mode.trim();
    if CODEX_APPROVAL_POLICIES.contains(&mode) {
        if !has_flag(detect_args, "--ask-for-approval", Some("-a")) {
            args.push("--ask-for-approval".into());
            args.push(mode.into());
        }

        return Ok(());
    }

    if CODEX_SANDBOX_MODES.contains(&mode) {
        if !has_flag(detect_args, "--sandbox", Some("-s")) {
            args.push("--sandbox".into());
            args.push(mode.into());
        }

        return Ok(());
    }

    if has_codex_mode_override(detect_args) {
        tracing::warn!(
            mode,
            "cli spawn: ignoring unsupported codex profile mode because provider args override codex approval/sandbox policy"
        );

        return Ok(());
    }

    bail!(
        "codex direct spawn mode '{mode}' is not supported by Codex CLI; use an approval policy ({}) or sandbox mode ({})",
        CODEX_APPROVAL_POLICIES.join(", "),
        CODEX_SANDBOX_MODES.join(", ")
    );
}

fn has_codex_mode_override(args: &[String]) -> bool {
    has_flag(args, "--ask-for-approval", Some("-a"))
        || has_flag(args, "--sandbox", Some("-s"))
        || has_flag(args, "--dangerously-bypass-approvals-and-sandbox", None)
}

fn claude_mcp_config(defs: &[MCPDefinition]) -> Result<String> {
    let mut servers = serde_json::Map::new();
    for def in defs {
        servers.insert(def.name.clone(), claude_mcp_server_config(def));
    }
    serde_json::to_string(&serde_json::json!({ "mcpServers": servers })).context("serialize claude MCP config")
}

fn claude_mcp_server_config(def: &MCPDefinition) -> serde_json::Value {
    let mut raw = expanded_raw(def);
    let Some(obj) = raw.as_object_mut() else {
        return raw;
    };
    if obj.contains_key("url") && !obj.contains_key("type") {
        let transport = obj
            .get("transport")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("http");
        obj.insert("type".into(), serde_json::Value::String(transport.into()));
    }

    raw
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ClaudePermissionTools {
    allow: Vec<String>,
    deny: Vec<String>,
}

fn claude_mcp_permission_tools(defs: &[MCPDefinition]) -> ClaudePermissionTools {
    let mut tools = ClaudePermissionTools::default();
    for def in defs {
        if let Some(include) = def.hyprpilot.include_tools.as_ref() {
            tools.allow.extend(
                include
                    .iter()
                    .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
            );
        }
        tools.deny.extend(
            def.hyprpilot
                .exclude_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
        tools.allow.extend(
            def.hyprpilot
                .auto_accept_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
        tools.deny.extend(
            def.hyprpilot
                .auto_reject_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
    }

    tools.allow.sort();
    tools.allow.dedup();
    tools.deny.sort();
    tools.deny.dedup();

    tools
}

fn claude_mcp_tool_pattern(server: &str, pattern: &str) -> String {
    if pattern.starts_with("mcp__") {
        pattern.to_string()
    } else {
        format!("mcp__{server}__{pattern}")
    }
}

fn mcp_leaf_pattern<'a>(server: &str, pattern: &'a str) -> Option<&'a str> {
    if let Some(rest) = pattern.strip_prefix("mcp__") {
        let (prefix, leaf) = rest.split_once("__")?;
        return (prefix == server).then_some(leaf);
    }

    Some(pattern)
}

fn is_exact_tool_name(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[')
}

#[derive(Debug, Default)]
struct CodexToolPolicy {
    default_approval: bool,
    approval_tools: BTreeSet<String>,
    enabled_tools: Option<BTreeSet<String>>,
    disabled_tools: BTreeSet<String>,
}

fn codex_mcp_tool_policy_entries(defs: &[MCPDefinition]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for def in defs {
        let policy = codex_mcp_tool_policy(def);
        let prefix = toml_key_path(&["mcp_servers", &def.name]);

        if policy.default_approval {
            entries.push((format!("{prefix}.default_tools_approval_mode"), toml_string("approve")));
        }
        for tool in policy.approval_tools {
            entries.push((
                toml_key_path(&["mcp_servers", &def.name, "tools", &tool, "approval_mode"]),
                toml_string("approve"),
            ));
        }
        if let Some(enabled_tools) = policy.enabled_tools {
            entries.push((
                format!("{prefix}.enabled_tools"),
                toml_array(&enabled_tools.into_iter().collect::<Vec<_>>()),
            ));
        }
        if !policy.disabled_tools.is_empty() {
            entries.push((
                format!("{prefix}.disabled_tools"),
                toml_array(&policy.disabled_tools.into_iter().collect::<Vec<_>>()),
            ));
        }
    }

    entries
}

fn codex_mcp_tool_policy(def: &MCPDefinition) -> CodexToolPolicy {
    let mut policy = CodexToolPolicy::default();

    if let Some(include) = def.hyprpilot.include_tools.as_ref() {
        let mut enabled = BTreeSet::new();
        let mut all_tools = false;
        for pattern in include {
            match mcp_leaf_pattern(&def.name, pattern) {
                Some("*") => {
                    all_tools = true;
                }
                Some(leaf) if is_exact_tool_name(leaf) => {
                    enabled.insert(leaf.to_string());
                }
                Some(leaf) => {
                    tracing::warn!(
                        server = %def.name,
                        pattern = %leaf,
                        "cli spawn: skipping codex includeTools glob; Codex enabled_tools supports exact tool names only"
                    );
                }
                None => {}
            }
        }
        if !all_tools {
            policy.enabled_tools = Some(enabled);
        }
    }

    for pattern in def
        .hyprpilot
        .exclude_tools
        .iter()
        .chain(def.hyprpilot.auto_reject_tools.iter())
    {
        match mcp_leaf_pattern(&def.name, pattern) {
            Some("*") => {
                policy.enabled_tools = Some(BTreeSet::new());
                policy.disabled_tools.clear();
            }
            Some(leaf)
                if is_exact_tool_name(leaf)
                    && policy
                        .enabled_tools
                        .as_ref()
                        .map_or(true, |enabled| !enabled.is_empty()) =>
            {
                policy.disabled_tools.insert(leaf.to_string());
            }
            Some(leaf) => {
                tracing::warn!(
                    server = %def.name,
                    pattern = %leaf,
                    "cli spawn: skipping codex reject/exclude glob; Codex disabled_tools supports exact tool names only"
                );
            }
            None => {}
        }
    }

    if policy.enabled_tools.as_ref().is_some_and(BTreeSet::is_empty) {
        return policy;
    }

    for pattern in &def.hyprpilot.auto_accept_tools {
        match mcp_leaf_pattern(&def.name, pattern) {
            Some("*") => {
                policy.default_approval = true;
            }
            Some(leaf) if is_exact_tool_name(leaf) => {
                policy.approval_tools.insert(leaf.to_string());
            }
            Some(leaf) => {
                tracing::warn!(
                    server = %def.name,
                    pattern = %leaf,
                    "cli spawn: skipping codex autoAcceptTools glob; Codex tool approval overrides support exact tool names only"
                );
            }
            None => {}
        }
    }

    policy
}

fn codex_mcp_config_entries(defs: &[MCPDefinition]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for def in defs {
        let Some(server) = project_transport(def) else {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for codex");
            continue;
        };
        match server {
            McpTransport::Stdio {
                name,
                command,
                args,
                env,
            } => {
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.command"), toml_string(&command.to_string_lossy())));
                if !args.is_empty() {
                    entries.push((format!("{prefix}.args"), toml_array(&args)));
                }
                for (key, value) in env {
                    entries.push((toml_key_path(&["mcp_servers", &name, "env", &key]), toml_string(&value)));
                }
            }
            McpTransport::Http { name, url, headers } => {
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.url"), toml_string(&url)));
                for (key, value) in headers {
                    push_codex_http_header(&mut entries, def, &name, &key, &value);
                }
            }
            McpTransport::Sse { name, url, headers } => {
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.url"), toml_string(&url)));
                for (key, value) in headers {
                    push_codex_http_header(&mut entries, def, &name, &key, &value);
                }
            }
        }
    }

    entries
}

fn push_codex_http_header(
    entries: &mut Vec<(String, String)>,
    def: &MCPDefinition,
    server: &str,
    header: &str,
    expanded_value: &str,
) {
    if let Some(var) = raw_header_value(def, header).and_then(bearer_env_reference) {
        if header.eq_ignore_ascii_case("authorization") {
            entries.push((
                toml_key_path(&["mcp_servers", server, "bearer_token_env_var"]),
                toml_string(&var),
            ));
            return;
        }
    }

    if let Some(var) = raw_header_value(def, header).and_then(env_reference) {
        entries.push((
            toml_key_path(&["mcp_servers", server, "env_http_headers", header]),
            toml_string(&var),
        ));
        return;
    }

    entries.push((
        toml_key_path(&["mcp_servers", server, "http_headers", header]),
        toml_string(expanded_value),
    ));
}

fn raw_header_value<'a>(def: &'a MCPDefinition, header: &str) -> Option<&'a str> {
    let headers = def.raw.get("headers")?.as_object()?;
    headers.get(header).and_then(serde_json::Value::as_str).or_else(|| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(header))
            .and_then(|(_, value)| value.as_str())
    })
}

fn bearer_env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let (scheme, value) = trimmed.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    env_reference(value.trim())
}

fn env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let candidate = if let Some(inner) = trimmed.strip_prefix("${").and_then(|value| value.strip_suffix('}')) {
        inner.strip_prefix("env:").unwrap_or(inner)
    } else {
        trimmed.strip_prefix('$')?
    };

    is_env_name(candidate).then(|| candidate.to_string())
}

fn is_env_name(candidate: &str) -> bool {
    !candidate.is_empty() && candidate.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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
        let Some(server) = project_transport(def) else {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for opencode");
            continue;
        };
        match server {
            McpTransport::Stdio {
                name,
                command,
                args,
                env,
            } => {
                let mut cmd = Vec::with_capacity(1 + args.len());
                cmd.push(command.to_string_lossy().to_string());
                cmd.extend(args);
                let environment: serde_json::Map<String, serde_json::Value> = env
                    .into_iter()
                    .map(|(key, value)| (key, serde_json::Value::String(value)))
                    .collect();
                let mut entry = serde_json::json!({
                    "type": "local",
                    "command": cmd,
                    "enabled": true,
                });
                if !environment.is_empty() {
                    entry["environment"] = serde_json::Value::Object(environment);
                }
                out.insert(name, entry);
            }
            McpTransport::Http { name, url, headers } => {
                out.insert(name, opencode_remote_mcp(url, headers));
            }
            McpTransport::Sse { name, url, headers } => {
                out.insert(name, opencode_remote_mcp(url, headers));
            }
        }
    }

    out
}

fn opencode_permission_content(defs: &[MCPDefinition]) -> Result<Option<String>> {
    let mut permissions: Vec<(String, String)> = Vec::new();
    for def in defs {
        let server = opencode_sanitize_tool_name(&def.name);

        for pattern in &def.hyprpilot.auto_accept_tools {
            if let Some(pattern) = opencode_mcp_tool_pattern(&def.name, pattern) {
                opencode_push_permission(&mut permissions, pattern, "allow");
            }
        }
        if def.hyprpilot.include_tools.is_some() {
            opencode_push_permission(&mut permissions, format!("{server}_*"), "deny");
        }
        if let Some(include) = def.hyprpilot.include_tools.as_ref() {
            for pattern in include {
                if let Some(pattern) = opencode_mcp_tool_pattern(&def.name, pattern) {
                    opencode_push_permission(&mut permissions, pattern, "allow");
                }
            }
        }
        for pattern in &def.hyprpilot.auto_reject_tools {
            if let Some(pattern) = opencode_mcp_tool_pattern(&def.name, pattern) {
                opencode_push_permission(&mut permissions, pattern, "deny");
            }
        }
        for pattern in &def.hyprpilot.exclude_tools {
            if let Some(pattern) = opencode_mcp_tool_pattern(&def.name, pattern) {
                opencode_push_permission(&mut permissions, pattern, "deny");
            }
        }
    }

    if permissions.is_empty() {
        return Ok(None);
    }

    opencode_permission_json(&permissions)
        .map(Some)
        .context("serialize opencode permissions")
}

fn opencode_push_permission(permissions: &mut Vec<(String, String)>, pattern: String, action: &str) {
    if let Some(pos) = permissions.iter().position(|(existing, _)| existing == &pattern) {
        permissions.remove(pos);
    }
    permissions.push((pattern, action.into()));
}

fn opencode_permission_json(permissions: &[(String, String)]) -> serde_json::Result<String> {
    let mut out = String::from("{");
    for (idx, (pattern, action)) in permissions.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&serde_json::to_string(pattern)?);
        out.push(':');
        out.push_str(&serde_json::to_string(action)?);
    }
    out.push('}');

    Ok(out)
}

fn opencode_mcp_tool_pattern(server: &str, pattern: &str) -> Option<String> {
    let leaf = mcp_leaf_pattern(server, pattern)?;
    Some(format!(
        "{}_{}",
        opencode_sanitize_tool_name(server),
        opencode_sanitize_tool_pattern(leaf)
    ))
}

fn opencode_sanitize_tool_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn opencode_sanitize_tool_pattern(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '*' | '?') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn opencode_remote_mcp(url: String, headers: Vec<(String, String)>) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "type": "remote",
        "url": url,
        "enabled": true,
    });
    if !headers.is_empty() {
        let headers: serde_json::Map<String, serde_json::Value> = headers
            .into_iter()
            .map(|(key, value)| (key, serde_json::Value::String(value)))
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
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::config::AgentConfig;
    use crate::mcp::{HyprpilotExtension, MCPDefinition};

    fn resolved(provider: AgentProvider) -> ResolvedInstance {
        ResolvedInstance {
            agent: AgentConfig {
                id: "agent".into(),
                provider,
                model: None,
                effort: None,
                command: "provider".into(),
                args: Vec::new(),
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

    fn resolved_with_mode(provider: AgentProvider, mode: Option<&str>) -> ResolvedInstance {
        let mut resolved = resolved(provider);
        resolved.mode = mode.map(str::to_string);

        resolved
    }

    fn resolved_with_cwd(provider: AgentProvider, cwd: &str) -> ResolvedInstance {
        let mut resolved = resolved(provider);
        resolved.agent.cwd = Some(PathBuf::from(cwd));
        resolved.mode = None;

        resolved
    }

    /// `resolved.agent.args` already carries the resolve-time
    /// profile `args` REPLACE merge by the time `build_command`
    /// runs — that merge happens in
    /// `adapters::profile::ResolvedInstance::from_profile_explicit`,
    /// not here (pinned separately by
    /// `flat_args_replace_base_agent_args_wholesale` in
    /// `adapters/profile.rs`). This helper stands in for that
    /// already-resolved shape, isolating `effort` / `mode` (both
    /// `None`) so the ordering / suppression assertions only have to
    /// reason about `--model`.
    fn resolved_with_agent_args(provider: AgentProvider, args: Vec<String>) -> ResolvedInstance {
        let mut resolved = resolved(provider);
        resolved.agent.args = args;
        resolved.effort = None;
        resolved.mode = None;

        resolved
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

    fn mcp_def_with_home_env() -> MCPDefinition {
        MCPDefinition {
            name: "shell-env".into(),
            raw: json!({
                "command": "${HOME}/bin/mcp",
                "args": ["--state", "${HOME}/state"],
                "env": { "TOKEN_FILE": "${HOME}/token" },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    fn mcp_def_with_permissions() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: None,
                exclude_tools: Vec::new(),
                auto_accept_tools: vec!["read_*".into(), "list_*".into()],
                auto_reject_tools: vec!["delete_*".into(), "mcp__filesystem__write_*".into()],
            },
            source: "<test>".into(),
        }
    }

    fn remote_mcp_def_with_headers() -> MCPDefinition {
        MCPDefinition {
            name: "github".into(),
            raw: json!({
                "url": "https://example.test/mcp",
                "headers": {
                    "Authorization": "Bearer ${NVIM_GITHUB}",
                    "X-MCP-Insiders": "true",
                    "x-api-key": "${env:EXA_API_KEY}",
                },
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: "<test>".into(),
        }
    }

    fn mcp_def_with_visibility() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: Some(vec!["read_file".into(), "list_*".into()]),
                exclude_tools: vec!["delete_file".into()],
                auto_accept_tools: vec!["read_file".into()],
                auto_reject_tools: vec!["write_*".into()],
            },
            source: "<test>".into(),
        }
    }

    fn mcp_def_with_visibility_conflicts() -> MCPDefinition {
        MCPDefinition {
            name: "filesystem".into(),
            raw: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            }),
            hyprpilot: HyprpilotExtension {
                include_tools: Some(vec!["read_file".into()]),
                exclude_tools: vec!["delete_*".into()],
                auto_accept_tools: vec!["read_file".into(), "write_file".into(), "delete_file".into()],
                auto_reject_tools: vec!["read_file".into()],
            },
            source: "<test>".into(),
        }
    }

    fn assert_json_key_before(body: &str, before: &str, after: &str) {
        let before_pos = body.find(&format!("\"{before}\"")).expect("before key exists");
        let after_pos = body.find(&format!("\"{after}\"")).expect("after key exists");
        assert!(before_pos < after_pos, "expected `{before}` before `{after}` in {body}");
    }

    #[test]
    fn claude_provider_args_suppress_generated_model() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[],
            vec!["--model".into(), "user-model".into()],
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--model").count(), 1);
    }

    /// Resolved argv order: profile-replaced `agent.args →
    /// generated…`. `resolved.agent.args` here stands in for the
    /// already-replaced list — a `--fallback-model` override arg must
    /// precede the generated `--model` flag `build_command` appends
    /// from `resolved.model`.
    #[test]
    fn override_agent_args_precede_generated_flags() {
        let resolved = resolved_with_agent_args(
            AgentProvider::ClaudeCode,
            vec!["--fallback-model".into(), "claude-sonnet-4-5".into()],
        );
        let command = build_command(&resolved, None, &[], vec![]).unwrap();

        let fallback_idx = command
            .args
            .iter()
            .position(|a| a == "--fallback-model")
            .expect("--fallback-model present");
        let model_idx = command
            .args
            .iter()
            .position(|a| a == "--model")
            .expect("generated --model present");

        assert!(
            fallback_idx < model_idx,
            "override args must precede generated vendor flags"
        );
        assert_eq!(
            command.args[model_idx + 1],
            "model-a",
            "generated --model flag still carries resolved.model's value"
        );
    }

    /// An override arg spelling a generated flag (`--model`)
    /// suppresses the generated duplicate — the existing `has_flag`
    /// dedup in `build_claude` covers `resolved.agent.args` the same
    /// way it already covers `provider_args` (trailing `-- <args>`).
    #[test]
    fn override_authored_model_flag_suppresses_generated_duplicate() {
        let resolved = resolved_with_agent_args(
            AgentProvider::ClaudeCode,
            vec!["--model".into(), "override-model".into()],
        );
        let command = build_command(&resolved, None, &[], vec![]).unwrap();

        assert_eq!(
            command.args.iter().filter(|arg| arg.as_str() == "--model").count(),
            1,
            "generated --model must not duplicate the override-authored one"
        );
        assert_eq!(
            command.args,
            vec!["--model".to_string(), "override-model".to_string()],
            "override's --model value wins; no generated --model appended"
        );
    }

    /// `${VAR}` expansion runs AFTER the profile-level `env`
    /// overlay, not before. `resolved.agent.env` here stands in for
    /// the already-overlaid map
    /// `adapters::profile::from_profile_explicit` produces (pinned
    /// separately by `flat_env_overlays_onto_agent_env_at_resolve`
    /// in `adapters/profile.rs`) — this test pins the SECOND half of
    /// that pipeline: an override-authored env value participates in
    /// `expand_value` exactly like an agent-authored one.
    #[test]
    fn agent_env_expansion_runs_after_override_overlay() {
        std::env::set_var("HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR", "expanded-value");
        let mut resolved = resolved(AgentProvider::ClaudeCode);

        resolved.agent.env.insert(
            "OVERRIDE_OVERLAID".into(),
            "${HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR}".into(),
        );

        let command = build_command(&resolved, None, &[], vec![]).unwrap();

        assert_eq!(
            command.env.get("OVERRIDE_OVERLAID").map(String::as_str),
            Some("expanded-value")
        );
        std::env::remove_var("HYPRPILOT_TEST_OVERRIDE_OVERLAID_VAR");
    }

    #[test]
    fn claude_mcp_config_uses_expanded_mcp_entries() {
        let home = std::env::var("HOME").expect("HOME should be present in test env");
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_home_env()],
            Vec::new(),
        )
        .unwrap();
        let config = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--mcp-config").then_some(&w[1]))
            .expect("claude mcp config injected");
        let parsed: serde_json::Value = serde_json::from_str(config).unwrap();

        assert_eq!(parsed["mcpServers"]["shell-env"]["command"], format!("{home}/bin/mcp"));
        assert_eq!(parsed["mcpServers"]["shell-env"]["args"][1], format!("{home}/state"));
        assert_eq!(
            parsed["mcpServers"]["shell-env"]["env"]["TOKEN_FILE"],
            format!("{home}/token")
        );
    }

    #[test]
    fn claude_mcp_config_adds_http_type_for_url_servers() {
        let config = claude_mcp_config(&[remote_mcp_def_with_headers()]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        assert_eq!(parsed["mcpServers"]["github"]["type"], "http");
        assert_eq!(parsed["mcpServers"]["github"]["url"], "https://example.test/mcp");
    }

    #[test]
    fn claude_mcp_permission_globs_map_to_allowed_and_disallowed_tools() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_permissions()],
            Vec::new(),
        )
        .unwrap();
        let allowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--allowedTools").then_some(w[1].as_str()))
            .expect("allowed tools injected");
        let disallowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--disallowedTools").then_some(w[1].as_str()))
            .expect("disallowed tools injected");

        assert_eq!(allowed, "mcp__filesystem__list_*,mcp__filesystem__read_*");
        assert_eq!(disallowed, "mcp__filesystem__delete_*,mcp__filesystem__write_*");
    }

    #[test]
    fn claude_mcp_visibility_globs_map_to_allowed_and_disallowed_tools() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
        )
        .unwrap();
        let allowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--allowedTools").then_some(w[1].as_str()))
            .expect("allowed tools injected");
        let disallowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--disallowedTools").then_some(w[1].as_str()))
            .expect("disallowed tools injected");

        assert_eq!(allowed, "mcp__filesystem__list_*,mcp__filesystem__read_file");
        assert_eq!(disallowed, "mcp__filesystem__delete_file,mcp__filesystem__write_*");
    }

    #[test]
    fn claude_provider_args_suppress_generated_mcp_permissions() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_permissions()],
            vec![
                "--allowed-tools".into(),
                "Read".into(),
                "--disallowedTools".into(),
                "Bash".into(),
            ],
        )
        .unwrap();

        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| matches!(arg.as_str(), "--allowedTools" | "--allowed-tools"))
                .count(),
            1
        );
        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| matches!(arg.as_str(), "--disallowedTools" | "--disallowed-tools"))
                .count(),
            1
        );
    }

    #[test]
    fn codex_projects_mcp_into_config_overrides() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
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
    fn codex_projects_remote_mcp_headers_into_supported_config() {
        let entries = codex_mcp_config_entries(&[remote_mcp_def_with_headers()]);
        let rendered = entries
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("mcp_servers.github.url=\"https://example.test/mcp\""));
        assert!(rendered.contains("mcp_servers.github.bearer_token_env_var=\"NVIM_GITHUB\""));
        assert!(rendered.contains("mcp_servers.github.http_headers.X-MCP-Insiders=\"true\""));
        assert!(rendered.contains("mcp_servers.github.env_http_headers.x-api-key=\"EXA_API_KEY\""));
        assert!(!rendered.contains("mcp_servers.github.headers."));
        assert!(!rendered.contains("Bearer "));
    }

    #[test]
    fn codex_projects_exact_mcp_tool_policy_into_supported_config() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
        )
        .unwrap();
        let joined = command.args.join("\n");

        assert!(joined.contains("mcp_servers.filesystem.enabled_tools=[\"read_file\"]"));
        assert!(joined.contains("mcp_servers.filesystem.disabled_tools=[\"delete_file\"]"));
        assert!(joined.contains("mcp_servers.filesystem.tools.read_file.approval_mode=\"approve\""));
        assert!(!joined.contains("list_*"));
        assert!(!joined.contains("write_*"));
    }

    #[test]
    fn codex_approval_mode_maps_to_ask_for_approval() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            Vec::new(),
        )
        .unwrap();

        assert!(command
            .args
            .windows(2)
            .any(|w| w == ["--ask-for-approval", "on-request"]));
    }

    #[test]
    fn codex_passes_resolved_cwd_to_native_cd_flag() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::Codex, "/tmp/hyprpilot-work"),
            None,
            &[],
            Vec::new(),
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--cd", "/tmp/hyprpilot-work"]));
        assert_eq!(command.cwd.as_deref(), Some(Path::new("/tmp/hyprpilot-work")));
    }

    #[test]
    fn codex_provider_args_suppress_generated_cd_flag() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::Codex, "/tmp/hyprpilot-work"),
            None,
            &[],
            vec!["-C".into(), "/tmp/other".into()],
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--cd").count(), 0);
        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "-C").count(), 1);
    }

    #[test]
    fn codex_sandbox_mode_maps_to_sandbox() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("workspace-write")),
            None,
            &[],
            Vec::new(),
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--sandbox", "workspace-write"]));
    }

    #[test]
    fn codex_rejects_unknown_mode_without_provider_override() {
        let err = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("plan")),
            None,
            &[],
            Vec::new(),
        )
        .expect_err("unknown mode should fail before spawning codex");

        assert!(err.to_string().contains("codex direct spawn mode 'plan'"), "{err}");
    }

    #[test]
    fn codex_provider_args_override_unknown_mode() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("plan")),
            None,
            &[],
            vec!["--ask-for-approval".into(), "never".into()],
        )
        .unwrap();

        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| arg.as_str() == "--ask-for-approval")
                .count(),
            1
        );
    }

    #[test]
    fn opencode_puts_prompt_mcp_and_variant_in_inline_config() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
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
        assert!(!command.args.iter().any(|arg| arg == "--session"));
    }

    #[test]
    fn opencode_projects_mcp_tool_policy_into_permission_env() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
        )
        .unwrap();
        let permissions: serde_json::Value =
            serde_json::from_str(command.env.get("OPENCODE_PERMISSION").unwrap()).unwrap();

        assert_eq!(permissions["filesystem_*"], "deny");
        assert_eq!(permissions["filesystem_read_file"], "allow");
        assert_eq!(permissions["filesystem_list_*"], "allow");
        assert_eq!(permissions["filesystem_delete_file"], "deny");
        assert_eq!(permissions["filesystem_write_*"], "deny");
    }

    #[test]
    fn opencode_permission_order_preserves_hyprpilot_policy_precedence() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[mcp_def_with_visibility_conflicts()],
            Vec::new(),
        )
        .unwrap();
        let permissions = command.env.get("OPENCODE_PERMISSION").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(permissions).unwrap();

        assert_eq!(parsed["filesystem_read_file"], "deny");
        assert_eq!(parsed["filesystem_delete_file"], "allow");
        assert_eq!(parsed["filesystem_delete_*"], "deny");
        assert_eq!(parsed["filesystem_write_file"], "allow");

        assert_json_key_before(permissions, "filesystem_delete_file", "filesystem_delete_*");
        assert_json_key_before(permissions, "filesystem_write_file", "filesystem_*");
        assert_json_key_before(permissions, "filesystem_*", "filesystem_read_file");
        assert_json_key_before(permissions, "filesystem_read_file", "filesystem_delete_*");
    }
}
