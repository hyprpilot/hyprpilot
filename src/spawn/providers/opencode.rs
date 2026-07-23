//! opencode (`opencode`) env-projected config + permission shape.

use anyhow::{Context, Result};

use crate::mcp::{project_transport, MCPDefinition, McpTransport};
use crate::profile::ResolvedProfile;

use super::argv::{combined_args, flag_value, has_flag};
use super::{base_command, ensure_inline_size, mcp_leaf_pattern, SpawnCommand};

pub(super) fn build_opencode(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
) -> Result<SpawnCommand> {
    let mut command = base_command(resolved);
    // Headless: `opencode run [OPTIONS] <prompt>` — the one-shot
    // subcommand. opencode has no stdin prompt support, so the buffered
    // prompt is the positional `message`. `run` is prepended before the
    // generated `--model` / `--agent` flags (both valid `run` options)
    // and the env-projected config still applies unchanged.
    if prompt.is_some() {
        command.args.insert(0, "run".into());
    }
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
    if let Some(prompt) = prompt {
        command.args.push(prompt.into());
    }

    Ok(command)
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
                // opencode's `type: "remote"` MCP entry carries no SSE
                // discriminator — the SSE-ness is dropped. Warn so the
                // downgrade to HTTP-style remote is not silent.
                tracing::warn!(
                    server = %name,
                    "cli spawn: opencode has no distinct SSE MCP transport; projecting `type=sse` server as a `remote` (HTTP) server"
                );
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
    let sanitized_leaf = opencode_sanitize_tool_pattern(leaf);
    if sanitized_leaf != leaf {
        tracing::warn!(
            server = %server,
            pattern = %leaf,
            sanitized = %sanitized_leaf,
            "cli spawn: opencode permission pattern altered by sanitization; character-class globs (e.g. `[abc]`) collapse to `_` and no longer match the intended tools"
        );
    }
    Some(format!("{}_{}", opencode_sanitize_tool_name(server), sanitized_leaf))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;
    use crate::spawn::providers::build_command;
    use crate::spawn::providers::fixtures::*;

    #[test]
    fn opencode_puts_prompt_mcp_and_variant_in_inline_config() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            Some("be terse"),
            &[mcp_def()],
            Vec::new(),
            None,
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
            None,
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
            None,
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

    #[test]
    fn opencode_agent_name_falls_back_to_hyprpilot() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            None,
            &[],
            vec![],
            None,
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--agent", "hyprpilot"]));
    }

    #[test]
    fn opencode_model_deduped_when_provider_arg_sets_it() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[],
            vec!["--model".into(), "user-model".into()],
            None,
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--model").count(), 1);
    }

    #[test]
    fn opencode_preset_config_content_is_not_overwritten() {
        let mut resolved = resolved(AgentProvider::OpenCode);
        resolved
            .agent
            .env
            .insert("OPENCODE_CONFIG_CONTENT".into(), "{\"preset\":true}".into());
        let command = build_command(&resolved, Some("be terse"), &[mcp_def()], vec![], None).unwrap();

        assert_eq!(
            command.env.get("OPENCODE_CONFIG_CONTENT").map(String::as_str),
            Some("{\"preset\":true}"),
            "an agent-provided OPENCODE_CONFIG_CONTENT must not be clobbered by generated config"
        );
    }

    #[test]
    fn opencode_preset_permission_is_not_overwritten() {
        let mut resolved = resolved(AgentProvider::OpenCode);
        resolved
            .agent
            .env
            .insert("OPENCODE_PERMISSION".into(), "{\"preset\":\"allow\"}".into());
        let command = build_command(&resolved, None, &[mcp_def_with_visibility()], vec![], None).unwrap();

        assert_eq!(
            command.env.get("OPENCODE_PERMISSION").map(String::as_str),
            Some("{\"preset\":\"allow\"}"),
            "an agent-provided OPENCODE_PERMISSION must not be clobbered by generated policy"
        );
    }

    #[test]
    fn opencode_projects_sse_as_remote() {
        let map = serde_json::Value::Object(opencode_mcp_config(&[sse_mcp_def()]));

        assert_eq!(map["events"]["type"], "remote");
        assert_eq!(map["events"]["url"], "https://example.test/sse");
    }

    #[test]
    fn opencode_sanitizes_charclass_glob_in_permission_key() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            None,
            &[mcp_def_with_charclass_glob()],
            vec![],
            None,
        )
        .unwrap();
        let permissions: serde_json::Value =
            serde_json::from_str(command.env.get("OPENCODE_PERMISSION").unwrap()).unwrap();

        // `[` and `]` collapse to `_`: read_[abc] → read__abc_ (the
        // pattern no longer matches the intended tools — hence the warn).
        assert_eq!(permissions["filesystem_read__abc_"], "allow");
    }

    #[test]
    fn opencode_headless_projects_run_subcommand_and_prompt_positional() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            None,
            &[],
            vec![],
            Some("write the readme"),
        )
        .unwrap();

        assert_eq!(
            command.args.first().map(String::as_str),
            Some("run"),
            "run is the subcommand"
        );
        assert!(command.args.iter().any(|a| a == "--model"), "model still projected");
        assert_eq!(command.args.last().map(String::as_str), Some("write the readme"));
    }
}
